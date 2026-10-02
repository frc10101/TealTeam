//! The tethered phone as the Pi's automatic uplink (S6).
//!
//! NetworkManager does the plumbing (P6): a phone plugged in and set to USB
//! tethering appears as `usb0` (Android) or `eth1` (iPhone), takes a lease from
//! the phone, and gets the default route at metric 50. What it cannot do is tell
//! the server. Left alone, the sync loop finds out at its next pass, which is up
//! to two minutes away during an event and three hours away when it booted with
//! an empty calendar (I13).
//!
//! So this watches the kernel's IPv4 routing table, `/proc/net/route`, every few
//! seconds. **The moment a default route goes through a tether interface, the
//! server syncs**: the event list if no event sync has landed since it started,
//! then the TBA pass, by the same path as the lead scout's "Sync now"
//! ([`upstream::tether_up`]). The phone's owner plugs it in and does nothing
//! else.
//!
//! The route, not the link: `usb0` is up a few seconds before the phone's DHCP
//! answers, and a sync then would only find no internet. The route appears
//! once there is somewhere to send packets.
//!
//! Unplugging needs nothing from here. The next request fails, the uplink says
//! "No internet" (I10), and the loop keeps its cadence until the route comes
//! back. Off Linux, where there is no `/proc/net/route`, the watch never starts
//! and the sync works as it did.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::info;
use tt_repo_sqlite::SqliteRepo;

use crate::upstream::{self, Upstream};

/// The kernel's IPv4 routes. Android and iPhone tethering both hand out IPv4.
pub const ROUTES: &str = "/proc/net/route";

/// How often to look. Reading the file costs nothing; a phone plugged in
/// should be syncing before its owner has put it down.
pub const EVERY: Duration = Duration::from_secs(3);

/// The interfaces `tealteam-uplink.nmconnection` matches. Keep the two in step.
pub const DEFAULT_INTERFACES: [&str; 2] = ["usb0", "eth1"];

/// `RTF_UP`: the route is usable.
const ROUTE_UP: u32 = 0x1;

/// What the server knows about the phone, for the lead-scout page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Tether {
    /// Not watching: not Linux, or switched off with `TETHER_INTERFACES=off`.
    #[default]
    Unwatched,
    /// No default route through any tether interface.
    Unplugged,
    /// The default route goes through this interface.
    Up(String),
}

impl Tether {
    /// One phrase for a person. Empty when nobody is watching.
    pub fn describe(&self) -> String {
        match self {
            Self::Unwatched => String::new(),
            Self::Unplugged => "not plugged in".into(),
            Self::Up(iface) => format!("plugged in ({iface})"),
        }
    }
}

/// Where to look and for what.
#[derive(Debug, Clone)]
pub struct Watch {
    pub routes: PathBuf,
    pub interfaces: Vec<String>,
    pub every: Duration,
}

impl Watch {
    /// The real routing table, for `interfaces`.
    pub fn new(interfaces: Vec<String>) -> Self {
        Self {
            routes: PathBuf::from(ROUTES),
            interfaces,
            every: EVERY,
        }
    }

    /// The tether interface the default route goes through now, or `None`.
    /// An unreadable table is an error, which only matters at the start.
    fn read(&self) -> std::io::Result<Option<String>> {
        let routes = std::fs::read_to_string(&self.routes)?;
        Ok(uplink_via(&routes, &self.interfaces))
    }
}

/// Which of `interfaces` holds a usable default route in `routes`, the text of
/// `/proc/net/route`. With several, the lowest metric: the one the kernel uses.
///
/// Each line after the header is `Iface Destination Gateway Flags RefCnt Use
/// Metric Mask ...`, with addresses and flags in hex. A default route has
/// destination and mask both zero.
pub fn uplink_via(routes: &str, interfaces: &[String]) -> Option<String> {
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [
                iface,
                destination,
                _gateway,
                flags,
                _refcnt,
                _use,
                metric,
                mask,
                ..,
            ] = fields[..]
            else {
                return None;
            };
            let flags = u32::from_str_radix(flags, 16).ok()?;
            let default = destination == "00000000" && mask == "00000000";
            let ours = interfaces.iter().any(|i| i == iface);
            (default && ours && flags & ROUTE_UP != 0)
                .then(|| Some((metric.parse::<u32>().ok()?, iface)))?
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, iface)| iface.to_string())
}

/// Watch for the tether and sync whenever it comes up. `None` when there is
/// nothing to watch: no interfaces, nothing upstream to sync, or no routing
/// table to read.
///
/// A tether already up at start is the boot sync's, not this; only a route
/// that appears afterwards starts a sync.
pub fn spawn(
    repo: Arc<SqliteRepo>,
    upstream: Arc<Upstream>,
    watch: Watch,
) -> Option<JoinHandle<()>> {
    if watch.interfaces.is_empty() {
        info!("TETHER_INTERFACES is off; not watching for a tethered phone");
        return None;
    }
    if upstream.first.is_none() && upstream.tba.is_none() {
        return None;
    }
    let mut was = match watch.read() {
        Ok(now) => now,
        Err(e) => {
            info!(
                "not watching for a tethered phone: cannot read {}: {e}",
                watch.routes.display()
            );
            return None;
        }
    };
    info!(
        "watching for a tethered phone on {}",
        watch.interfaces.join(", ")
    );
    upstream.set_tether(was.clone().map_or(Tether::Unplugged, Tether::Up));

    Some(tokio::spawn(async move {
        loop {
            tokio::time::sleep(watch.every).await;
            // A read that fails once (it should not) changes nothing.
            let Ok(now) = watch.read() else { continue };
            match (&was, &now) {
                (None, Some(iface)) => {
                    info!("phone tether up on {iface}; syncing now");
                    // Not awaited: a sync can take a minute, and the page
                    // should say the phone is in meanwhile.
                    let (repo, upstream) = (repo.clone(), upstream.clone());
                    tokio::spawn(async move { upstream::tether_up(&repo, &upstream).await });
                }
                (Some(iface), None) => info!("phone tether on {iface} is gone"),
                _ => {}
            }
            upstream.set_tether(now.clone().map_or(Tether::Unplugged, Tether::Up));
            was = now;
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upstream::test_support::{EVENTS, first_stub, upstream_at};
    use tt_repo::Repo;

    const HEADER: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT";

    /// A routing table with `rows` under the header, as the kernel prints it.
    fn table(rows: &[&str]) -> String {
        std::iter::once(HEADER)
            .chain(rows.iter().copied())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn ours() -> Vec<String> {
        DEFAULT_INTERFACES.map(String::from).to_vec()
    }

    const ETH0_LAN: &str = "eth0\t0065650A\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0";
    const USB0_DEFAULT: &str = "usb0\t00000000\t012BA8C0\t0003\t0\t0\t50\t00000000\t0\t0\t0";
    const USB0_SUBNET: &str = "usb0\t002BA8C0\t00000000\t0001\t0\t0\t50\t00FFFFFF\t0\t0\t0";
    const WLAN0_DEFAULT: &str = "wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0";

    #[test]
    fn the_tether_counts_once_it_holds_the_default_route() {
        // At the venue with no phone: only the scouts' LAN.
        assert_eq!(uplink_via(&table(&[ETH0_LAN]), &ours()), None);
        // usb0 has a lease but no default route yet: not yet.
        assert_eq!(uplink_via(&table(&[ETH0_LAN, USB0_SUBNET]), &ours()), None);
        // Now it does.
        let up = table(&[ETH0_LAN, USB0_SUBNET, USB0_DEFAULT]);
        assert_eq!(uplink_via(&up, &ours()).as_deref(), Some("usb0"));
    }

    #[test]
    fn other_ways_out_are_not_the_tether() {
        // The shop's Wi-Fi, at metric 600, is internet but not the phone.
        assert_eq!(uplink_via(&table(&[WLAN0_DEFAULT]), &ours()), None);
        // With both, the tether is still found.
        let both = table(&[WLAN0_DEFAULT, USB0_DEFAULT]);
        assert_eq!(uplink_via(&both, &ours()).as_deref(), Some("usb0"));
    }

    #[test]
    fn a_route_that_is_down_or_garbled_is_no_tether() {
        let down = USB0_DEFAULT.replace("\t0003\t", "\t0002\t");
        assert_eq!(uplink_via(&table(&[&down]), &ours()), None);
        assert_eq!(uplink_via(&table(&["usb0\t00000000"]), &ours()), None);
        assert_eq!(uplink_via("", &ours()), None);
    }

    #[test]
    fn with_two_phones_the_lower_metric_wins() {
        let iphone = "eth1\t00000000\t01AAA8C0\t0003\t0\t0\t20\t00000000\t0\t0\t0";
        let both = table(&[USB0_DEFAULT, iphone]);
        assert_eq!(uplink_via(&both, &ours()).as_deref(), Some("eth1"));
    }

    #[test]
    fn this_machines_routing_table_parses() {
        // Whatever it says, reading it must not fail where the file exists.
        if let Ok(routes) = std::fs::read_to_string(ROUTES) {
            let _ = uplink_via(&routes, &ours());
        }
    }

    // ── The watch ───────────────────────────────────────────────────────────

    async fn repo() -> Arc<SqliteRepo> {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        tt_repo_sqlite::migrate::apply(repo.pool())
            .await
            .expect("migrate");
        Arc::new(repo)
    }

    /// A routing table in a scratch directory, removed on drop.
    struct Routes {
        dir: PathBuf,
        watch: Watch,
    }

    impl Routes {
        fn new(rows: &[&str]) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "tt-web-tether-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&dir).expect("mkdir");
            let watch = Watch {
                routes: dir.join("route"),
                interfaces: ours(),
                every: Duration::from_millis(20),
            };
            let routes = Self { dir, watch };
            routes.set(rows);
            routes
        }

        fn set(&self, rows: &[&str]) {
            std::fs::write(&self.watch.routes, table(rows)).expect("write routes");
        }
    }

    impl Drop for Routes {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Poll `check` until it holds, for at most a second.
    async fn eventually(mut check: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if check() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn plugging_the_phone_in_syncs_the_events_the_boot_missed() {
        let base = first_stub(EVENTS).await;
        let repo = repo().await;
        let upstream = Arc::new(upstream_at(Some(&base), false));
        let routes = Routes::new(&[ETH0_LAN]);

        let task = spawn(repo.clone(), upstream.clone(), routes.watch.clone()).expect("watching");
        assert_eq!(upstream.tether(), Tether::Unplugged);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(repo.list_events().await.unwrap().is_empty(), "no phone yet");

        routes.set(&[ETH0_LAN, USB0_SUBNET, USB0_DEFAULT]);
        assert!(eventually(|| upstream.tether() == Tether::Up("usb0".into())).await);
        let mut landed = false;
        for _ in 0..100 {
            if !repo.list_events().await.unwrap().is_empty() {
                landed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(landed, "the event list came in over the tether");

        routes.set(&[ETH0_LAN]);
        assert!(eventually(|| upstream.tether() == Tether::Unplugged).await);
        task.abort();
    }

    #[tokio::test]
    async fn plugging_the_phone_in_wakes_the_background_loop() {
        let base = first_stub(EVENTS).await;
        let upstream = Arc::new(upstream_at(Some(&base), true));
        // The events are in already, so only the loop has anything to do.
        upstream.mark_events_synced();
        let routes = Routes::new(&[]);

        let task = spawn(repo().await, upstream.clone(), routes.watch.clone()).expect("watching");
        routes.set(&[USB0_DEFAULT]);

        tokio::time::timeout(Duration::from_secs(1), upstream.woken())
            .await
            .expect("the loop was woken");
        task.abort();
    }

    #[tokio::test]
    async fn a_phone_already_in_at_start_is_left_to_the_boot_sync() {
        let base = first_stub(EVENTS).await;
        let repo = repo().await;
        let upstream = Arc::new(upstream_at(Some(&base), false));
        let routes = Routes::new(&[USB0_DEFAULT]);

        let task = spawn(repo.clone(), upstream.clone(), routes.watch.clone()).expect("watching");
        assert_eq!(upstream.tether(), Tether::Up("usb0".into()));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(repo.list_events().await.unwrap().is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn nothing_is_watched_without_a_table_interfaces_or_upstream() {
        let base = first_stub(EVENTS).await;
        let configured = || Arc::new(upstream_at(Some(&base), false));
        let routes = Routes::new(&[]);

        let missing = Watch {
            routes: routes.watch.routes.with_file_name("absent"),
            ..routes.watch.clone()
        };
        let upstream = configured();
        assert!(spawn(repo().await, upstream.clone(), missing).is_none());
        assert_eq!(upstream.tether(), Tether::Unwatched);

        let off = Watch {
            interfaces: vec![],
            ..routes.watch.clone()
        };
        assert!(spawn(repo().await, configured(), off).is_none());

        let nothing = Arc::new(upstream_at(None, false));
        assert!(spawn(repo().await, nothing, routes.watch.clone()).is_none());
    }

    #[test]
    fn the_page_says_where_the_phone_is() {
        assert_eq!(Tether::Unwatched.describe(), "");
        assert_eq!(Tether::Unplugged.describe(), "not plugged in");
        assert_eq!(Tether::Up("usb0".into()).describe(), "plugged in (usb0)");
    }
}
