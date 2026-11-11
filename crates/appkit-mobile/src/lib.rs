//! The phone side of an app's hub (the mobile basis of the kit, rules `MOB-*` and `MSEC-*` of the ruleset `mobile`).
//!
//! A phone app of a family (testing mobile, later the others) is a client of the hub on the PC. This crate is what
//! every such app needs between its page and that hub, written once:
//!
//! * [`way`]: the three ways to the data (Tailscale Serve, an SSH tunnel, a vault cloned on the phone) behind one
//!   description; the page knows which one is in use (`MUI-05`) but never builds an address itself;
//! * [`link`]: pairing with a one-time code, the device login with a key the phone keeps (a [`link::Signer`]: the
//!   Android Keystore in an app, a key in memory in tests), the check that the hub is the one paired with
//!   (`MSEC-04`), and every request with a time limit and a byte limit (`MSEC-08`); the session token stays in memory
//!   (`MSEC-01`);
//! * [`state`]: what the phone keeps about its pairing, versioned and written atomically (`MSEC-13`), and wholly
//!   removed on unpairing (`MSEC-12`).
//!
//! The hub decides what a phone may see and do (`MOB-02`, `MOB-03`); nothing here filters or grants. What the page
//! shows comes through [`link::Link::fetch`], which only reaches the routes of the app it was made for.

pub mod link;
pub mod state;
pub mod way;

#[cfg(feature = "net")]
pub mod net;

/// Lower-case hex of bytes.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Is `text` exactly `bytes` bytes of lower-case hex?
#[must_use]
pub fn is_hex(text: &str, bytes: usize) -> bool {
    text.len() == bytes * 2 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
