//! The faults, done to a server this tool started itself.
//!
//! **The cable.** Every scout reaches the server through [`Cable`], a TCP
//! relay. Pulling it does what a pulled Ethernet cable does: nothing arrives
//! and nothing is refused. Bytes in flight wait, a connection opened now
//! hangs, and when the cable goes back in whatever waited is delivered late
//! -- including a save its scout already gave up on, which is the case the
//! record id exists for.
//!
//! **The power.** [`Server`] is the tt-web process. Killing it is SIGKILL, no
//! shutdown, while the cable is also pulled: a Pi with no power answers
//! nothing, rather than refusing. It comes back on the same database, and
//! the cable goes back in once it answers `/health`, as a Pi that has booted.
//! SIGKILL loses what the process held, not what the kernel had yet to write
//! to disk; a real power cut can also lose that. See the Q3 notes.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::watch;

/// A relay to `upstream` that can be unplugged.
pub struct Cable {
    plugged: watch::Sender<bool>,
    pub addr: std::net::SocketAddr,
}

impl Cable {
    pub async fn start(upstream: std::net::SocketAddr) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (plugged, _) = watch::channel(true);
        let watch = plugged.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let plugged = watch.subscribe();
                tokio::spawn(relay(client, upstream, plugged));
            }
        });
        Ok(Self { plugged, addr })
    }

    pub fn set_plugged(&self, plugged: bool) {
        self.plugged.send_replace(plugged);
    }
}

async fn wait_plugged(plugged: &mut watch::Receiver<bool>) {
    // An error is the cable dropped, at the end of the run: nothing to wait for.
    let _ = plugged.wait_for(|in_| *in_).await;
}

async fn relay(
    client: TcpStream,
    upstream: std::net::SocketAddr,
    mut plugged: watch::Receiver<bool>,
) {
    // A SYN sent down a pulled cable is not answered.
    wait_plugged(&mut plugged).await;
    // A server that is off: the client finds out the way it would from a
    // rebooted Pi, by the connection closing.
    let Ok(server) = TcpStream::connect(upstream).await else {
        return;
    };
    let _ = client.set_nodelay(true);
    let _ = server.set_nodelay(true);
    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = server.into_split();
    let up = pump(client_read, server_write, plugged.clone());
    let down = pump(server_read, client_write, plugged);
    // Either side closing ends both, as a reset would.
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
}

/// Copy `from` to `to`, holding each chunk while the cable is out.
async fn pump(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    mut plugged: watch::Receiver<bool>,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        wait_plugged(&mut plugged).await;
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

/// A tt-web process on its own database, in `dir`.
pub struct Server {
    binary: PathBuf,
    dir: PathBuf,
    pub port: u16,
    child: Option<Child>,
}

impl Server {
    pub fn new(binary: &Path, dir: &Path) -> anyhow::Result<Self> {
        // A free port, found by asking for one and letting it go.
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        Ok(Self {
            binary: binary.to_path_buf(),
            dir: dir.to_path_buf(),
            port,
            child: None,
        })
    }

    pub fn database_url(dir: &Path) -> String {
        format!("sqlite://{}/load.db?mode=rwc", dir.display())
    }

    pub fn addr(&self) -> std::net::SocketAddr {
        ([127, 0, 0, 1], self.port).into()
    }

    /// Start it and wait until it answers.
    pub async fn start(&mut self) -> anyhow::Result<()> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("server.log"))?;
        // Backups on, as on event day: their copy competes for the database.
        // The server never creates its backup folder (Q4), so it is made here.
        std::fs::create_dir_all(self.dir.join("backups"))?;
        // A clean environment: no FIRST or TBA keys, no .env from elsewhere.
        let child = Command::new(&self.binary)
            .current_dir(&self.dir)
            .env_clear()
            .env("PORT", self.port.to_string())
            .env("DATABASE_URL", Self::database_url(&self.dir))
            .env("BACKUP_DIR", self.dir.join("backups"))
            .env("RUST_LOG", "warn")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", self.binary.display()))?;
        self.child = Some(child);

        let health = format!("http://{}/health", self.addr());
        let client = reqwest::Client::new();
        for _ in 0..300 {
            if let Ok(r) = client.get(&health).send().await
                && r.status().is_success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!("tt-web did not answer /health within 30s; see server.log")
    }

    /// SIGKILL: no shutdown, nothing flushed by the process.
    pub async fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn echo_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 64];
                    while let Ok(n) = s.read(&mut buf).await {
                        if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn a_pulled_cable_holds_bytes_and_delivers_them_when_plugged_back() {
        let cable = Cable::start(echo_server().await).await.unwrap();
        let mut s = TcpStream::connect(cable.addr).await.unwrap();
        let mut buf = [0u8; 5];

        s.write_all(b"hello").await.unwrap();
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");

        cable.set_plugged(false);
        s.write_all(b"again").await.unwrap();
        let waited = tokio::time::timeout(Duration::from_millis(300), s.read_exact(&mut buf)).await;
        assert!(waited.is_err(), "nothing comes back down a pulled cable");

        cable.set_plugged(true);
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"again", "and it arrives once plugged back in");
    }

    #[tokio::test]
    async fn a_connection_opened_while_unplugged_hangs_rather_than_failing() {
        let cable = Cable::start(echo_server().await).await.unwrap();
        cable.set_plugged(false);
        let mut s = TcpStream::connect(cable.addr).await.unwrap();
        s.write_all(b"x").await.unwrap();
        let mut buf = [0u8; 1];
        let waited = tokio::time::timeout(Duration::from_millis(300), s.read_exact(&mut buf)).await;
        assert!(waited.is_err());
        cable.set_plugged(true);
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"x");
    }
}
