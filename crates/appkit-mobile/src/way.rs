//! The three ways from the phone to the data of an app (`MOB-06`), and the link that pairs a phone.
//!
//! | Way | What it is | Read | Create, change | Trigger | Push | Offline |
//! |---|---|---|---|---|---|---|
//! | [`Way::Tailnet`] | `https://<pc>.<tailnet>.ts.net`, Tailscale Serve forwards to the loopback hub | yes | yes | with a confirmation | yes | the kept copy |
//! | [`Way::Tunnel`] | `http://127.0.0.1:<port>` on the phone, an SSH client forwards it to the same port on the PC | yes | yes | with a confirmation | no (the tunnel is down when the app is) | the kept copy |
//! | [`Way::Vault`] | a folder cloned with Git on the phone, read by the app | yes | no | no | no | always |
//!
//! The hub is the same in the first two: the guard tells them apart by the `Host` (a tailnet name, or a loopback name
//! that only a tunnel or the PC itself can use). A phone never reaches the hub any other way: no LAN address, no
//! port forward, no Funnel (`MOB-01`).

use serde::{Deserialize, Serialize};

/// How the phone reaches the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Way {
    /// Tailscale Serve: HTTPS with the certificate of the tailnet name.
    Tailnet,
    /// An SSH tunnel to the loopback port of the hub, opened by an SSH client on the phone.
    Tunnel,
    /// A vault (or an export folder) cloned with Git on the phone: read only, no hub.
    Vault,
}

/// What a way can do. The page leaves out what its way cannot (`MOB-09`: says so once, does not fail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Abilities {
    pub read: bool,
    /// Create and change (through the hub, with `if_match`).
    pub write: bool,
    /// Start something on the PC, with a confirmation on the phone.
    pub trigger: bool,
    /// Web Push from the hub can reach the phone through this way's registration.
    pub push: bool,
}

impl Way {
    /// What the way can do; the hub still decides per device (`MOB-03`).
    #[must_use]
    pub fn abilities(self) -> Abilities {
        match self {
            Self::Tailnet => Abilities {
                read: true,
                write: true,
                trigger: true,
                push: true,
            },
            Self::Tunnel => Abilities {
                read: true,
                write: true,
                trigger: true,
                push: false,
            },
            Self::Vault => Abilities {
                read: true,
                write: false,
                trigger: false,
                push: false,
            },
        }
    }
}

/// Where a hub is: the base the requests go to and the origin they carry. Made only by the constructors, so the base
/// is always a tailnet name over HTTPS or the phone's own loopback over HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubAddress {
    way: Way,
    /// `https://pc.example.ts.net` or `http://127.0.0.1:17618`, no slash at the end.
    base: String,
}

/// Is `name` a plain host name: lower-case labels of letters, digits and `-`, at least two, none empty or with a `-`
/// at an end, the last one not all digits (that would be an address)? At most 253 characters.
fn plain_host(name: &str) -> bool {
    let labels: Vec<&str> = name.split('.').collect();
    name.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| {
            (1..=63).contains(&l.len())
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_lowercase()))
}

/// Is `name` a tailnet name as the hub's guard takes it: a plain host name, with an optional `:<port>` (plain digits,
/// no leading zero, 1 to 65535, not 443) for Serve on another HTTPS port. The same rule as
/// `appkit_core::authn::tailnet_name_ok` (this crate stays without the core for the phone; a test holds both equal).
fn tailnet_name(name: &str) -> bool {
    match name.split_once(':') {
        None => plain_host(name),
        Some((host, port)) => {
            plain_host(host)
                && (1..=5).contains(&port.len())
                && port.bytes().all(|b| b.is_ascii_digit())
                && !port.starts_with('0')
                && port.parse::<u16>().is_ok_and(|p| p != 443)
        }
    }
}

impl HubAddress {
    /// The hub behind Tailscale Serve on `name` (`pc.example.ts.net`, or `pc.example.ts.net:8443` when Serve forwards
    /// another HTTPS port than 443 to it). `None` when it is not a plain host name with an optional port other than
    /// 443 (the rule of the hub's guard).
    #[must_use]
    pub fn tailnet(name: &str) -> Option<Self> {
        tailnet_name(name).then(|| Self {
            way: Way::Tailnet,
            base: format!("https://{name}"),
        })
    }

    /// The hub at the phone's loopback `port`, where an SSH tunnel ends. The tunnel must use the hub's own port on
    /// both ends: the guard takes `127.0.0.1:<its port>` as the host. `None` for port 0.
    #[must_use]
    pub fn tunnel(port: u16) -> Option<Self> {
        (port != 0).then(|| Self {
            way: Way::Tunnel,
            base: format!("http://127.0.0.1:{port}"),
        })
    }

    #[must_use]
    pub fn way(&self) -> Way {
        self.way
    }

    /// The base of every request, without a slash at the end.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The origin the requests carry: the hub's own (what a page served by the hub would send), so the guard takes
    /// it and a device signs its login for exactly this hub.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.base
    }

    /// Read back from the kept state: only what the constructors would make.
    #[must_use]
    pub fn checked(self) -> Option<Self> {
        let again = match self.way {
            Way::Tailnet => self.base.strip_prefix("https://").and_then(Self::tailnet),
            Way::Tunnel => self
                .base
                .strip_prefix("http://127.0.0.1:")
                .filter(|p| !p.starts_with('0') && p.len() <= 5)
                .and_then(|p| p.parse::<u16>().ok())
                .and_then(Self::tunnel),
            Way::Vault => None,
        };
        again.filter(|a| *a == self)
    }
}

/// What a pairing link holds: where the hub is, which app it is, and the one-time code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairLink {
    pub address: HubAddress,
    pub app: String,
    pub code: String,
}

/// The longest pairing link that is read.
pub const MAX_PAIR_LINK: usize = 512;
/// The marker of a pairing link in its fragment, with its version.
pub const PAIR_MARK: &str = "appkit-pair=1";

/// The pairing link the PC shows (as a QR code or as text): `<base>/#appkit-pair=1&app=<app>&code=<hex>`. The code
/// is in the fragment, which no browser sends and no server logs (`MSEC-02`).
#[must_use]
pub fn pair_link(address: &HubAddress, app: &str, code: &str) -> String {
    format!("{}/#{PAIR_MARK}&app={app}&code={code}", address.base())
}

/// Read a pairing link (scanned or pasted). Strict: exactly the form [`pair_link`] makes, the code 32 hex digits
/// (128 bits), the app a plain name; anything else is `None`, and nothing of it is used.
#[must_use]
pub fn parse_pair_link(text: &str) -> Option<PairLink> {
    let text = text.trim();
    if text.len() > MAX_PAIR_LINK {
        return None;
    }
    let (base, fragment) = text.split_once("/#")?;
    let address = if let Some(host) = base.strip_prefix("https://") {
        HubAddress::tailnet(host)?
    } else {
        let port = base.strip_prefix("http://127.0.0.1:").filter(|p| {
            !p.starts_with('0') && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit())
        })?;
        HubAddress::tunnel(port.parse().ok()?)?
    };
    let mut parts = fragment.split('&');
    if parts.next()? != PAIR_MARK {
        return None;
    }
    let app = parts.next()?.strip_prefix("app=")?;
    let code = parts.next()?.strip_prefix("code=")?;
    if parts.next().is_some()
        || app.is_empty()
        || app.len() > 32
        || !app
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || !crate::is_hex(code, 16)
    {
        return None;
    }
    Some(PairLink {
        address,
        app: app.to_owned(),
        code: code.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_tailnet_name_is_a_plain_host_and_a_tunnel_a_loopback_port() {
        let a = HubAddress::tailnet("pc.example.ts.net").unwrap();
        assert_eq!(a.base(), "https://pc.example.ts.net");
        assert_eq!(a.origin(), a.base());
        assert_eq!(a.way(), Way::Tailnet);
        for bad in [
            "",
            "pc",
            "PC.example.ts.net",
            "pc.example.ts.net:443",
            "pc..ts.net",
            "-pc.ts.net",
            "10.0.0.1",
            "pc.example.ts.net/",
            "user@pc.ts.net",
        ] {
            assert!(HubAddress::tailnet(bad).is_none(), "{bad:?}");
        }
        // A second hub of the same PC on another port of Serve.
        let p = HubAddress::tailnet("pc.example.ts.net:8443").unwrap();
        assert_eq!(p.base(), "https://pc.example.ts.net:8443");
        assert_eq!(p.clone().checked(), Some(p.clone()));
        let link = pair_link(&p, "testing", CODE);
        assert_eq!(parse_pair_link(&link).unwrap().address, p);
        for bad in [
            "pc.example.ts.net:0",
            "pc.example.ts.net:08443",
            "pc.example.ts.net:65536",
        ] {
            assert!(HubAddress::tailnet(bad).is_none(), "{bad:?}");
            assert!(
                parse_pair_link(&format!("https://{bad}/#{PAIR_MARK}&app=x&code={CODE}")).is_none()
            );
        }
        // The phone takes exactly the names the guard of the hub takes.
        for name in [
            "pc.example.ts.net",
            "pc.example.ts.net:8443",
            "pc.ts.net:65535",
            "pc.example.ts.net:443",
            "pc.example.ts.net:0",
            "pc.example.ts.net:",
            "pc.example.ts.net:+1",
            "pc.example.ts.net:1:2",
            ":8443",
            "10.0.0.1:8443",
            "PC.example.ts.net:8443",
        ] {
            assert_eq!(
                tailnet_name(name),
                appkit_core::authn::tailnet_name_ok(name),
                "{name:?}"
            );
        }
        let t = HubAddress::tunnel(17618).unwrap();
        assert_eq!(t.base(), "http://127.0.0.1:17618");
        assert!(HubAddress::tunnel(0).is_none());
    }

    #[test]
    fn a_kept_address_is_taken_only_in_the_form_the_constructors_make() {
        let ok = HubAddress::tailnet("pc.example.ts.net").unwrap();
        assert_eq!(ok.clone().checked(), Some(ok));
        let tampered = [
            HubAddress {
                way: Way::Tailnet,
                base: "http://pc.example.ts.net".to_owned(),
            },
            HubAddress {
                way: Way::Tunnel,
                base: "http://192.168.1.2:17618".to_owned(),
            },
            HubAddress {
                way: Way::Tunnel,
                base: "http://127.0.0.1:017618".to_owned(),
            },
            HubAddress {
                way: Way::Vault,
                base: "https://pc.example.ts.net".to_owned(),
            },
        ];
        for t in tampered {
            assert!(t.clone().checked().is_none(), "{t:?}");
        }
    }

    #[test]
    fn a_pairing_link_reads_back_exactly_and_nothing_else_does() {
        let a = HubAddress::tailnet("pc.example.ts.net").unwrap();
        let link = pair_link(&a, "testing", CODE);
        assert_eq!(
            link,
            format!("https://pc.example.ts.net/#appkit-pair=1&app=testing&code={CODE}")
        );
        let read = parse_pair_link(&format!("  {link}\n")).unwrap();
        assert_eq!(read.address, a);
        assert_eq!(read.app, "testing");
        assert_eq!(read.code, CODE);
        let t = HubAddress::tunnel(17618).unwrap();
        assert_eq!(
            parse_pair_link(&pair_link(&t, "testing", CODE))
                .unwrap()
                .address,
            t
        );
        for bad in [
            format!("http://pc.example.ts.net/#appkit-pair=1&app=testing&code={CODE}"),
            format!("https://pc.example.ts.net/?code={CODE}"),
            format!("https://pc.example.ts.net/#appkit-pair=2&app=testing&code={CODE}"),
            format!("https://pc.example.ts.net/#appkit-pair=1&code={CODE}&app=testing"),
            format!("https://pc.example.ts.net/#appkit-pair=1&app=Testing&code={CODE}"),
            format!("https://pc.example.ts.net/#appkit-pair=1&app=testing&code={CODE}&x=1"),
            "https://pc.example.ts.net/#appkit-pair=1&app=testing&code=00".to_owned(),
            format!("http://192.168.1.5:17618/#appkit-pair=1&app=testing&code={CODE}"),
            format!("http://127.0.0.1:0/#appkit-pair=1&app=testing&code={CODE}"),
            format!("javascript:alert(1)/#appkit-pair=1&app=testing&code={CODE}"),
            format!(
                "https://{}.ts.net/#appkit-pair=1&app=testing&code={CODE}",
                "a".repeat(600)
            ),
        ] {
            assert!(parse_pair_link(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_vault_way_only_reads() {
        let v = Way::Vault.abilities();
        assert!(v.read && !v.write && !v.trigger && !v.push);
        assert!(Way::Tailnet.abilities().push);
        assert!(!Way::Tunnel.abilities().push);
    }
}
