//! Is there a route to the internet at all (I10)?
//!
//! A raw TCP connect, not an HTTP request: no DNS, no TLS handshake, no payload.
//! On a Pi sharing a phone's tethered connection that difference is the
//! difference between a 40ms answer and a multi-second one.

use std::time::Duration;

use crate::Uplink;

/// Cloudflare's resolver. Chosen because it answers on 443 from essentially
/// anywhere with a route, and because it is an IP -- so a broken DNS server does
/// not read as "no internet".
const PROBE_HOST: &str = "1.1.1.1";
const PROBE_PORT: u16 = 443;
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Test the uplink and record the result.
pub async fn probe(uplink: &Uplink) -> bool {
    let address = format!("{PROBE_HOST}:{PROBE_PORT}");
    let attempt = tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio::net::TcpStream::connect(address.clone()),
    )
    .await;

    match attempt {
        Ok(Ok(_)) => {
            uplink.record_probe(true, "");
            true
        }
        Ok(Err(e)) => {
            uplink.record_probe(false, &format!("cannot reach {address}: {e}"));
            false
        }
        Err(_) => {
            uplink.record_probe(false, &format!("cannot reach {address}: timed out"));
            false
        }
    }
}

/// Whether a base URL points somewhere on the local network.
///
/// Probing the internet before calling a LAN address is wrong: a test server on
/// `192.168.1.10` is reachable precisely when the internet is not. Loopback,
/// RFC1918, and link-local all skip the probe.
pub fn is_local(base_url: &str) -> bool {
    let Some(host) = host_of(base_url) else {
        return false;
    };

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    if ip.is_loopback() {
        return true;
    }

    match ip {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 10
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 169 && b == 254) // link-local
        }
        std::net::IpAddr::V6(v6) => v6.is_unicast_link_local() || v6.is_unique_local(),
    }
}

/// Extract the host from a URL.
///
/// Hand-rolled rather than pulling a URL parser: this only needs to answer
/// "is that address on my LAN", and the shapes that reach it are a handful of
/// configured base URLs.
///
/// Order matters -- scheme, then authority, then userinfo, then port -- and
/// IPv6 literals keep their brackets until last.
fn host_of(base_url: &str) -> Option<String> {
    let rest = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))?;

    // Authority is everything before the path.
    let authority = rest.split(['/', '?', '#']).next()?;

    // Drop any user:pass@ prefix.
    let authority = authority.rsplit('@').next()?;

    let host = if let Some(closing) = authority.find(']') {
        // IPv6 literal: [::1]:8080 -> ::1
        authority.get(1..closing)?
    } else {
        // Only strip a port if what follows the colon is numeric, so a stray
        // colon does not eat part of a hostname.
        match authority.rsplit_once(':') {
            Some((head, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
                head
            }
            _ => authority,
        }
    };

    (!host.is_empty()).then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_private_ranges_are_local() {
        for url in [
            "http://localhost:8080",
            "http://127.0.0.1:8080",
            "http://10.0.0.5/api",
            "http://172.16.4.4",
            "http://172.31.255.255",
            "http://192.168.1.10:3000",
            "http://169.254.1.1",
        ] {
            assert!(is_local(url), "{url} should be local");
        }
    }

    #[test]
    fn public_hosts_are_not_local() {
        for url in [
            "https://www.thebluealliance.com/api/v3",
            "https://frc-api.firstinspires.org/v3.0",
            "http://8.8.8.8",
            // 172.32 is outside the RFC1918 block, which is an easy off-by-one.
            "http://172.32.0.1",
            "http://172.15.0.1",
        ] {
            assert!(!is_local(url), "{url} should not be local");
        }
    }

    #[test]
    fn malformed_urls_are_treated_as_remote() {
        // Safer default: probe unnecessarily rather than skip a needed check.
        for url in ["", "not a url", "ftp://example.com"] {
            assert!(!is_local(url));
        }
    }

    #[test]
    fn hosts_parse_out_of_realistic_urls() {
        assert_eq!(
            host_of("https://a.example.com/x/y").as_deref(),
            Some("a.example.com")
        );
        assert_eq!(host_of("http://1.2.3.4:8080/x").as_deref(), Some("1.2.3.4"));
        assert_eq!(
            host_of("http://user:pass@1.2.3.4/x").as_deref(),
            Some("1.2.3.4")
        );
        assert_eq!(
            host_of("http://example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host_of("http://example.com?q=1").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn ipv6_literals_lose_their_brackets_and_are_recognised() {
        assert_eq!(host_of("http://[::1]:8080/").as_deref(), Some("::1"));
        assert!(is_local("http://[::1]:8080/"), "loopback");
        assert!(is_local("http://[fe80::1]/"), "link-local");
        assert!(is_local("http://[fd00::1]/"), "unique-local");
        assert!(!is_local("http://[2606:4700::1111]/"), "public");
    }

    #[test]
    fn a_non_numeric_colon_suffix_is_not_treated_as_a_port() {
        assert_eq!(
            host_of("http://example.com:notaport/").as_deref(),
            Some("example.com:notaport")
        );
    }
}
