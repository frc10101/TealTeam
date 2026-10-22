//! HTTPS on the event LAN, with our own certificate authority (H1).
//!
//! Chrome turns on service workers and OPFS only in a secure context, and
//! over plain `http://tealteam.local` that is never true, so the offline
//! client stays off (LAN rehearsal 2026-10-22). There is no internet at the
//! venue and no public name, so no public CA can issue a certificate. The
//! server is its own: on first start it makes a CA (ten years) and a server
//! certificate signed by it for every name a device might type, and keeps
//! both in `TLS_DIR`. Each device trusts `ca.crt` once, downloaded from
//! `/ca.crt` (docs/event/HTTPS_SETUP.md). Plain HTTP keeps serving beside it,
//! so a device without the CA still works.
//!
//! `TLS_CERT` and `TLS_KEY` replace all of that with a certificate from
//! elsewhere; `TLS_PORT=off` turns HTTPS off.
//!
//! The crypto is `ring`, which the build already carries for `reqwest`:
//! `aws-lc-rs` would need cmake and a C toolchain on the Pi.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use time::OffsetDateTime;
use tracing::{info, warn};

/// The names a device might type, beside `TLS_NAMES`: the Pi's DNS and mDNS
/// names, its fixed address (on every printed card), and this machine.
pub const DEFAULT_NAMES: [&str; 5] = [
    "tealteam.local",
    "tealteam",
    "10.101.0.1",
    "localhost",
    "127.0.0.1",
];

/// A CA lasts until long after this season; replacing it means every device
/// trusts the new one.
const CA_DAYS: i64 = 3650;
/// Under 398 days, the most any browser accepts for a server certificate.
const SERVER_DAYS: i64 = 397;
/// Made again at start-up when it has less than this left.
const RENEW_WITHIN_DAYS: i64 = 30;
/// A connection that has not finished its handshake by then is dropped, so
/// a stalled client cannot hold a task open.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the server's certificate comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Our own CA in this folder, made on first start.
    OwnCa { dir: PathBuf, names: Vec<String> },
    /// A certificate chain and key made elsewhere.
    Files { cert: PathBuf, key: PathBuf },
}

/// What [`load`] hands the listener: the chain and key in PEM, and our CA's
/// certificate when there is one, for `/ca.crt`.
pub struct Material {
    pub chain_pem: String,
    pub key_pem: String,
    pub ca_pem: Option<String>,
}

/// Our CA's certificate, for `GET /ca.crt`. Set once at start-up; unset when
/// HTTPS is off or the certificate came from elsewhere.
static CA_PEM: OnceLock<String> = OnceLock::new();

/// `GET /ca.crt`: the certificate a device installs to trust this server.
/// Public, like `/help`: a device needs it before anyone can sign in on it.
pub async fn ca_certificate() -> Response {
    match CA_PEM.get() {
        Some(pem) => (
            [
                (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"tealteam-ca.crt\"",
                ),
            ],
            pem.clone(),
        )
            .into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            "This server has no certificate authority of its own: HTTPS is off, \
             or its certificate came from elsewhere (TLS_CERT).",
        )
            .into_response(),
    }
}

/// Read or make the certificate. Files are written with mode 600 where the
/// platform has modes; the CA's key is what every trusting device relies on.
pub fn load(source: &Source) -> Result<Material> {
    match source {
        Source::Files { cert, key } => Ok(Material {
            chain_pem: read(cert)?,
            key_pem: read(key)?,
            ca_pem: None,
        }),
        Source::OwnCa { dir, names } => own_ca(dir, names, OffsetDateTime::now_utc()),
    }
}

fn own_ca(dir: &Path, names: &[String], now: OffsetDateTime) -> Result<Material> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let (ca_crt, ca_key) = (dir.join("ca.crt"), dir.join("ca.key"));
    let (srv_crt, srv_key, srv_meta) = (
        dir.join("server.crt"),
        dir.join("server.key"),
        dir.join("server.names"),
    );

    let (ca_pem, ca_key_pem) = if ca_crt.is_file() && ca_key.is_file() {
        (read(&ca_crt)?, read(&ca_key)?)
    } else {
        let (pem, key) = make_ca(now)?;
        write_private(&ca_key, &key)?;
        write(&ca_crt, &pem)?;
        info!(
            "made a certificate authority in {}: every device must trust ca.crt (GET /ca.crt)",
            dir.display()
        );
        (pem, key)
    };

    let wanted = meta(names);
    let fresh = srv_crt.is_file()
        && srv_key.is_file()
        && std::fs::read_to_string(&srv_meta)
            .ok()
            .and_then(|m| still_good(&m, &wanted, now))
            .unwrap_or(false);
    let (srv_pem, srv_key_pem) = if fresh {
        (read(&srv_crt)?, read(&srv_key)?)
    } else {
        let (pem, key, expires) = make_server(&ca_pem, &ca_key_pem, names, now)?;
        write_private(&srv_key, &key)?;
        write(&srv_crt, &pem)?;
        write(
            &srv_meta,
            &format!("{wanted}expires={}\n", expires.unix_timestamp()),
        )?;
        info!(names = %names.join(", "), "made the server certificate");
        (pem, key)
    };

    Ok(Material {
        chain_pem: format!("{srv_pem}{ca_pem}"),
        key_pem: srv_key_pem,
        ca_pem: Some(ca_pem),
    })
}

fn make_ca(now: OffsetDateTime) -> Result<(String, String)> {
    let key = KeyPair::generate().context("making the CA key")?;
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "TealTeam event CA");
    dn.push(DnType::OrganizationName, "TealTeam");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(CA_DAYS);
    let cert = params.self_signed(&key).context("signing the CA")?;
    Ok((cert.pem(), key.serialize_pem()))
}

fn make_server(
    ca_pem: &str,
    ca_key_pem: &str,
    names: &[String],
    now: OffsetDateTime,
) -> Result<(String, String, OffsetDateTime)> {
    let ca_key = KeyPair::from_pem(ca_key_pem).context("reading ca.key")?;
    let ca = CertificateParams::from_ca_cert_pem(ca_pem)
        .context("reading ca.crt")?
        .self_signed(&ca_key)
        .context("ca.key does not match ca.crt")?;

    if let Some(bad) = names.iter().find(|n| !valid_name(n)) {
        anyhow::bail!("{bad:?} in TLS_NAMES is neither a host name nor an IP address");
    }
    let key = KeyPair::generate().context("making the server key")?;
    let mut params = CertificateParams::new(names.to_vec()).context("a name in TLS_NAMES")?;
    let mut dn = DistinguishedName::new();
    dn.push(
        DnType::CommonName,
        names.first().map_or("tealteam", String::as_str),
    );
    params.distinguished_name = dn;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    params.not_before = now - time::Duration::days(1);
    let expires = now + time::Duration::days(SERVER_DAYS);
    params.not_after = expires;
    let cert = params
        .signed_by(&key, &ca, &ca_key)
        .context("signing the server certificate")?;
    Ok((cert.pem(), key.serialize_pem(), expires))
}

/// An IP address, or a host name of letters, digits, hyphens and dots.
fn valid_name(name: &str) -> bool {
    name.parse::<std::net::IpAddr>().is_ok()
        || (!name.is_empty()
            && name.len() <= 253
            && name.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            }))
}

/// `server.names`: one name a line, then `expires=<unix seconds>`.
fn meta(names: &[String]) -> String {
    names.iter().map(|n| format!("{n}\n")).collect()
}

/// The server certificate is kept when it was made for exactly these names
/// and has more than [`RENEW_WITHIN_DAYS`] left.
fn still_good(stored: &str, wanted: &str, now: OffsetDateTime) -> Option<bool> {
    let (names, expires) = stored.rsplit_once("expires=")?;
    let expires = expires.trim().parse::<i64>().ok()?;
    let left = expires - now.unix_timestamp();
    Some(names == wanted && left > RENEW_WITHIN_DAYS * 86_400)
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("making {} private", path.display()))?;
    }
    Ok(())
}

fn server_config(material: &Material) -> Result<rustls::ServerConfig> {
    let chain = CertificateDer::pem_slice_iter(material.chain_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .context("reading the certificate chain")?;
    anyhow::ensure!(
        !chain.is_empty(),
        "the certificate file holds no certificate"
    );
    let key = PrivateKeyDer::from_pem_slice(material.key_pem.as_bytes())
        .context("reading the private key")?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .context("TLS versions")?
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .context("the key does not match the certificate")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// Serve `app` over HTTPS on `0.0.0.0:port` until the process ends. An error
/// here is returned before anything is bound, so the caller can log it and
/// keep serving plain HTTP.
pub async fn serve(port: u16, source: &Source, app: Router) -> Result<()> {
    let material = load(source)?;
    let config = server_config(&material)?;
    if let Some(ca) = material.ca_pem {
        let _ = CA_PEM.set(ca);
    }
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!("listening on https://{addr}");

    tokio::spawn(async move {
        loop {
            let (tcp, _) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => {
                    // Out of file descriptors and the like: wait, do not spin.
                    warn!("https accept: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let (acceptor, app) = (acceptor.clone(), app.clone());
            tokio::spawn(async move {
                let tls = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await
                {
                    Ok(Ok(tls)) => tls,
                    // A device without our CA hangs up here: expected, and noisy.
                    Ok(Err(e)) => return tracing::debug!("https handshake: {e}"),
                    Err(_) => return tracing::debug!("https handshake timed out"),
                };
                let service = hyper_util::service::TowerToHyperService::new(app);
                let builder = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                );
                if let Err(e) = builder
                    .serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(tls), service)
                    .await
                {
                    tracing::debug!("https connection: {e}");
                }
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        DEFAULT_NAMES.map(String::from).to_vec()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-tls-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_first_start_makes_a_ca_and_a_server_certificate_rustls_accepts() {
        let dir = scratch("first");
        let m = own_ca(&dir, &names(), OffsetDateTime::now_utc()).unwrap();
        assert!(m.ca_pem.as_deref().unwrap().contains("BEGIN CERTIFICATE"));
        assert_eq!(m.chain_pem.matches("BEGIN CERTIFICATE").count(), 2);
        server_config(&m).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("ca.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restart_keeps_the_ca_and_the_server_certificate() {
        let dir = scratch("restart");
        let now = OffsetDateTime::now_utc();
        let a = own_ca(&dir, &names(), now).unwrap();
        let b = own_ca(&dir, &names(), now).unwrap();
        assert_eq!(a.ca_pem, b.ca_pem);
        assert_eq!(a.chain_pem, b.chain_pem);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_names_or_a_near_expiry_remake_the_server_certificate_under_the_same_ca() {
        let dir = scratch("renew");
        let now = OffsetDateTime::now_utc();
        let a = own_ca(&dir, &names(), now).unwrap();

        let mut more = names();
        more.push("scouting.lan".into());
        let b = own_ca(&dir, &more, now).unwrap();
        assert_eq!(a.ca_pem, b.ca_pem);
        assert_ne!(a.chain_pem, b.chain_pem);

        let later = now + time::Duration::days(SERVER_DAYS - RENEW_WITHIN_DAYS + 1);
        let c = own_ca(&dir, &more, later).unwrap();
        assert_eq!(a.ca_pem, c.ca_pem);
        assert_ne!(b.chain_pem, c.chain_pem);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ca_crt_serves_our_ca_for_a_device_to_install() {
        let dir = scratch("route");
        let m = own_ca(&dir, &names(), OffsetDateTime::now_utc()).unwrap();
        let _ = CA_PEM.set(m.ca_pem.clone().unwrap());
        let response = ca_certificate().await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/x-x509-ca-cert"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_name_is_an_error_not_a_panic() {
        let dir = scratch("bad");
        assert!(own_ca(&dir, &["not a name!".into()], OffsetDateTime::now_utc()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
