//! The real way out of the hub's Web Push (feature `push-net`): reqwest with rustls and the Mozilla roots, no redirect,
//! no proxy of the system, a time limit, and the answer read no further than its status. Only to endpoints
//! `push::allowed_endpoint` took when the device subscribed, and only to a public address: the name of an endpoint is
//! resolved here, and an address of this machine, the local network, the tailnet or any other range that is not
//! public is never connected to (a name like `127.0.0.1.nip.io` passes the check of the name, not this one).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use crate::push::{PushRequest, PushTransport, SEND_TIMEOUT};
use crate::BoxFuture;

/// Is `ip` an address of the public internet? Not loopback, private, link-local, shared (CGNAT, the tailnet's
/// `100.64.0.0/10`), unspecified, broadcast, documentation, benchmarking, reserved or multicast; IPv6 not loopback,
/// unique local, link-local, documentation, NAT64 or multicast, and a mapped IPv4 address as that address.
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                // NAT64 and the IPv4-compatible range carry an IPv4 address that is not checked here.
                || (s[0] == 0x0064 && s[1] == 0xff9b)
                || s[..6] == [0; 6])
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, c, _] = v4.octets();
    !(v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || v4.is_multicast()
        || v4.is_documentation()
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

/// Resolves a name to its public addresses only; a name with none is an error, and nothing is connected to.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 443))
                .await?
                .filter(|a| is_public(a.ip()))
                .collect();
            if found.is_empty() {
                return Err("the push service has no public address".into());
            }
            let addrs: reqwest::dns::Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// The transport of a hub that sends Web Push.
pub struct NetPush {
    client: reqwest::Client,
}

impl NetPush {
    /// # Errors
    ///
    /// The TLS backend could not be set up (a fault of the build).
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(PublicOnly))
            .timeout(SEND_TIMEOUT)
            .build()
            .map_err(|e| format!("no HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

impl PushTransport for NetPush {
    fn send(&self, request: PushRequest) -> BoxFuture<'_, Option<u16>> {
        Box::pin(async move {
            if !crate::push::allowed_endpoint(&request.endpoint) {
                return None;
            }
            let mut builder = self.client.post(&request.endpoint).body(request.body);
            for (name, value) in &request.headers {
                builder = builder.header(*name, value);
            }
            builder.send().await.ok().map(|r| r.status().as_u16())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_connected_to() {
        for public in [
            "8.8.8.8",
            "151.101.1.69",
            "2a00:1450:4001:80b::200e",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
        for not in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.10",
            "169.254.169.254",
            "100.114.201.106",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "192.0.0.8",
            "198.18.0.1",
            "203.0.113.5",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "2001:db8::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
            "64:ff9b::7f00:1",
            "::127.0.0.1",
        ] {
            assert!(!is_public(not.parse().unwrap()), "{not}");
        }
    }

    #[tokio::test]
    async fn a_name_of_this_machine_resolves_to_nothing() {
        use reqwest::dns::Resolve;
        assert!(PublicOnly
            .resolve("localhost".parse().unwrap())
            .await
            .is_err());
    }
}
