//! Credentials and the rules for who may talk to the local server.
//!
//! This module knows nothing of HTTP types: it works on strings and bytes,
//! so the rules can be tested without a socket and moved with the crate.
//!
//! * a [`Secret`] is random, from the operating system, and its text form is
//!   lowercase hex; it never prints itself;
//! * every comparison of a credential goes through [`ct_eq`], and the
//!   stores look at *all* their entries, so the time taken does not say
//!   which one was close;
//! * time comes from a [`Clock`], so an expiry is tested without waiting.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The monotonic time the stores use. Only differences matter.
pub trait Clock: Send + Sync {
    /// Time since an arbitrary, fixed origin.
    fn now(&self) -> Duration;
}

/// The real clock.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A clock a test moves by hand.
#[derive(Debug, Default)]
pub struct ManualClock {
    millis: AtomicU64,
}

impl ManualClock {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Move time forward.
    pub fn advance(&self, by: Duration) {
        let ms = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        self.millis.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
}

/// Why a secret could not be made.
#[derive(Debug)]
pub struct RandomError(getrandom::Error);

impl fmt::Display for RandomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the operating system gave no random bytes: {}", self.0)
    }
}

impl std::error::Error for RandomError {}

/// Random bytes with a text form. It never appears in `Debug` output.
///
/// There is no `PartialEq`: `==` would stop at the first differing byte.
/// Use [`Secret::matches`].
#[derive(Clone)]
pub struct Secret(Vec<u8>);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// A session token: 256 bits.
pub const TOKEN_BYTES: usize = 32;
/// A pairing code or an instance id: 128 bits.
pub const CODE_BYTES: usize = 16;

impl Secret {
    /// `len` random bytes from the operating system.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the system has no random source.
    pub fn generate(len: usize) -> Result<Self, RandomError> {
        let mut bytes = vec![0u8; len];
        getrandom::fill(&mut bytes).map_err(RandomError)?;
        Ok(Self(bytes))
    }

    /// Lowercase hex, `2 * len` characters.
    #[must_use]
    pub fn to_hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(self.0.len() * 2);
        for b in &self.0 {
            out.push(char::from(DIGITS[usize::from(b >> 4)]));
            out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
        }
        out
    }

    /// Read back what [`Secret::to_hex`] wrote: exactly `len` bytes of
    /// lowercase hex, nothing else.
    #[must_use]
    pub fn from_hex(text: &str, len: usize) -> Option<Self> {
        if text.len() != len * 2 {
            return None;
        }
        let mut bytes = Vec::with_capacity(len);
        for pair in text.as_bytes().chunks(2) {
            let hi = hex_digit(pair[0])?;
            let lo = hex_digit(pair[1])?;
            bytes.push((hi << 4) | lo);
        }
        Some(Self(bytes))
    }

    /// Constant-time equality.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        ct_eq(&self.0, &other.0)
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

/// Equality that takes the same time wherever two inputs differ. The
/// lengths are not secret and are compared first.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    // `black_box` keeps the optimizer from turning the fold into an early exit.
    std::hint::black_box(diff) == 0
}

// ── Host, Origin, Fetch Metadata ────────────────────────────────────────────

/// The names the server answers to. Exact string comparison: no regex, no
/// case folding, no "starts with" (the CVE class of 5.4, T1).
#[derive(Debug, Clone)]
pub struct HostPolicy {
    hosts: Vec<String>,
    origins: Vec<String>,
    /// The name of this machine in the tailnet (`pc.example.ts.net`), when Tailscale Serve forwards to the hub.
    remote: Option<String>,
}

impl HostPolicy {
    /// The three loopback names with the port the server really bound.
    #[must_use]
    pub fn loopback(port: u16) -> Self {
        let names = [
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ];
        let origins = names.iter().map(|n| format!("http://{n}")).collect();
        Self {
            hosts: names.to_vec(),
            origins,
            remote: None,
        }
    }

    /// Also answer to `name`, the name of this machine in the tailnet, as Tailscale Serve forwards it: the `Host`
    /// `name` exactly and the `Origin` `https://name`. `name` is a plain host name for Serve on 443, or
    /// `host:port` for Serve on another HTTPS port ([`tailnet_name_ok`]): browsers send that port in both headers.
    /// The server still binds loopback only; a request with this host came through the tunnel, and the guard holds
    /// it to what a device may.
    #[must_use]
    pub fn with_remote(mut self, name: &str) -> Self {
        self.hosts.push(name.to_owned());
        self.origins.push(format!("https://{name}"));
        self.remote = Some(name.to_owned());
        self
    }

    /// Did this `Host` header come through the tailnet (the name of [`HostPolicy::with_remote`])?
    #[must_use]
    pub fn is_remote(&self, host: &str) -> bool {
        self.remote.as_deref() == Some(host)
    }

    /// Is this `Host` header one of ours?
    #[must_use]
    pub fn host_ok(&self, host: &str) -> bool {
        self.hosts.iter().any(|h| h == host)
    }

    /// Is this `Origin` header one of ours? `null` is not.
    #[must_use]
    pub fn origin_ok(&self, origin: &str) -> bool {
        self.origins.iter().any(|o| o == origin)
    }
}

/// Is `name` a plain host name: lower-case labels of letters, digits and `-`, at least two, each 1 to 63
/// characters, none with a `-` at an end, the last one not all digits (that would be an address)? At most 253
/// characters.
#[must_use]
pub fn plain_host_name(name: &str) -> bool {
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

/// Is `name` the name of this machine in the tailnet as Tailscale Serve answers it: a plain host name
/// ([`plain_host_name`], Serve on 443), or one with `:<port>` for Serve on another HTTPS port (so that two hubs of
/// one machine can each have a port, `tailscale serve --https=<port>`)? The port is 1 to 65535 in plain digits, no
/// leading zero, and never 443: a browser leaves the default port out of `Host` and `Origin`, so `name:443` would
/// never match.
#[must_use]
pub fn tailnet_name_ok(name: &str) -> bool {
    match name.split_once(':') {
        None => plain_host_name(name),
        Some((host, port)) => {
            plain_host_name(host)
                && (1..=5).contains(&port.len())
                && port.bytes().all(|b| b.is_ascii_digit())
                && !port.starts_with('0')
                && port.parse::<u16>().is_ok_and(|p| p != 443)
        }
    }
}

/// `Sec-Fetch-Site`: absent is allowed (an older browser or a program
/// client; the token and the Origin rule still apply), present must say
/// that the request came from this origin or from the user typing it.
#[must_use]
pub fn fetch_site_ok(value: Option<&str>) -> bool {
    matches!(value, None | Some("same-origin" | "none"))
}

// ── Pairing codes, sessions, rate limit ─────────────────────────────────────

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while the lock was held leaves the data as it was; the
    // stores only push and remove, so it is still usable.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One-time pairing codes with a lifetime.
pub struct PairingStore {
    ttl: Duration,
    max: usize,
    clock: Arc<dyn Clock>,
    codes: Mutex<Vec<(Secret, Duration)>>,
}

impl PairingStore {
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, ttl: Duration) -> Self {
        Self {
            ttl,
            max: 8,
            clock,
            codes: Mutex::new(Vec::new()),
        }
    }

    /// Make a code. At most a few are open at once; the oldest goes first.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the system has no random source.
    pub fn mint(&self) -> Result<Secret, RandomError> {
        let code = Secret::generate(CODE_BYTES)?;
        let now = self.clock.now();
        let mut codes = lock(&self.codes);
        codes.retain(|(_, born)| now.saturating_sub(*born) < self.ttl);
        if codes.len() >= self.max {
            codes.remove(0);
        }
        codes.push((code.clone(), now));
        Ok(code)
    }

    /// Spend a code. True once, then never again; false after the lifetime.
    #[must_use]
    pub fn redeem(&self, text: &str) -> bool {
        let Some(given) = Secret::from_hex(text, CODE_BYTES) else {
            return false;
        };
        let now = self.clock.now();
        let mut codes = lock(&self.codes);
        codes.retain(|(_, born)| now.saturating_sub(*born) < self.ttl);
        // Look at every entry, whichever one matches.
        let mut found = None;
        for (i, (code, _)) in codes.iter().enumerate() {
            if code.matches(&given) {
                found = Some(i);
            }
        }
        match found {
            Some(i) => {
                codes.remove(i);
                true
            }
            None => false,
        }
    }
}

/// The sessions of this run of the server. Nothing is kept on disk: a
/// restart ends every session.
pub struct SessionStore {
    max: usize,
    /// Each session with the device it was opened for (`None`: a browser on this machine, paired by code).
    sessions: Mutex<VecDeque<(Secret, Option<String>)>>,
}

impl SessionStore {
    #[must_use]
    pub fn new(max: usize) -> Self {
        Self {
            max: max.max(1),
            sessions: Mutex::new(VecDeque::new()),
        }
    }

    /// Open a session. Over the limit the oldest one is closed.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the system has no random source.
    pub fn create(&self) -> Result<Secret, RandomError> {
        self.open(None)
    }

    /// Open a session for a paired device (its id), so that revoking the device closes it.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the system has no random source.
    pub fn create_for_device(&self, device: &str) -> Result<Secret, RandomError> {
        self.open(Some(device.to_owned()))
    }

    fn open(&self, device: Option<String>) -> Result<Secret, RandomError> {
        let token = Secret::generate(TOKEN_BYTES)?;
        let mut sessions = lock(&self.sessions);
        while sessions.len() >= self.max {
            sessions.pop_front();
        }
        sessions.push_back((token.clone(), device));
        Ok(token)
    }

    /// Close every session of a device (it was revoked).
    pub fn close_device(&self, device: &str) {
        lock(&self.sessions).retain(|(_, d)| d.as_deref() != Some(device));
    }

    /// The device a session was opened for; `None` for a session of a code, or for no session.
    #[must_use]
    pub fn device_of(&self, text: &str) -> Option<String> {
        let given = Secret::from_hex(text, TOKEN_BYTES)?;
        let sessions = lock(&self.sessions);
        let mut found = None;
        for (token, device) in sessions.iter() {
            if token.matches(&given) {
                found.clone_from(device);
            }
        }
        found
    }

    /// Is this the token of an open session?
    #[must_use]
    pub fn check(&self, text: &str) -> bool {
        let Some(given) = Secret::from_hex(text, TOKEN_BYTES) else {
            return false;
        };
        let sessions = lock(&self.sessions);
        let mut ok = false;
        for (token, _) in sessions.iter() {
            ok |= token.matches(&given);
        }
        ok
    }

    /// Close every session.
    pub fn clear(&self) {
        lock(&self.sessions).clear();
    }
}

/// At most `max` events in any window of `window`.
pub struct RateLimiter {
    max: usize,
    window: Duration,
    clock: Arc<dyn Clock>,
    hits: Mutex<VecDeque<Duration>>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            clock,
            hits: Mutex::new(VecDeque::new()),
        }
    }

    /// Count one event; false when it is over the limit (and not counted).
    #[must_use]
    pub fn allow(&self) -> bool {
        let now = self.clock.now();
        let mut hits = lock(&self.hits);
        while hits
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= self.window)
        {
            hits.pop_front();
        }
        if hits.len() >= self.max {
            return false;
        }
        hits.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> (Arc<ManualClock>, Arc<dyn Clock>) {
        let manual = Arc::new(ManualClock::new());
        (Arc::clone(&manual), manual)
    }

    #[test]
    fn hex_round_trips_and_refuses_anything_else() {
        let s = Secret::generate(TOKEN_BYTES).unwrap();
        let text = s.to_hex();
        assert_eq!(text.len(), 64);
        assert!(Secret::from_hex(&text, TOKEN_BYTES).unwrap().matches(&s));
        assert!(Secret::from_hex(&text.to_uppercase(), TOKEN_BYTES).is_none());
        assert!(Secret::from_hex(&text[..62], TOKEN_BYTES).is_none());
        assert!(Secret::from_hex(&format!("{text}00"), TOKEN_BYTES).is_none());
        assert!(Secret::from_hex(&"g".repeat(64), TOKEN_BYTES).is_none());
        assert!(Secret::from_hex("", TOKEN_BYTES).is_none());
    }

    #[test]
    fn a_secret_never_prints_itself() {
        let s = Secret::generate(CODE_BYTES).unwrap();
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn ct_eq_is_plain_equality() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn two_secrets_differ() {
        let a = Secret::generate(TOKEN_BYTES).unwrap();
        let b = Secret::generate(TOKEN_BYTES).unwrap();
        assert!(!a.matches(&b));
    }

    #[test]
    fn the_tailnet_name_is_one_more_exact_host_and_its_https_origin() {
        let p = HostPolicy::loopback(17615).with_remote("pc.example.ts.net");
        assert!(p.host_ok("pc.example.ts.net"));
        assert!(p.origin_ok("https://pc.example.ts.net"));
        assert!(p.is_remote("pc.example.ts.net"));
        assert!(p.host_ok("127.0.0.1:17615") && !p.is_remote("127.0.0.1:17615"));
        for bad in [
            "pc.example.ts.net:443",
            "PC.example.ts.net",
            "pc.example.ts.net.",
            "evil.pc.example.ts.net",
        ] {
            assert!(!p.host_ok(bad) && !p.is_remote(bad), "{bad:?}");
        }
        for bad in ["http://pc.example.ts.net", "https://pc.example.ts.net:443"] {
            assert!(!p.origin_ok(bad), "{bad:?}");
        }
        assert!(!HostPolicy::loopback(17615).is_remote("pc.example.ts.net"));
    }

    #[test]
    fn a_tailnet_name_with_a_port_is_its_own_exact_host_and_origin() {
        // Two hubs of one machine: one on Serve's 443, the other on another HTTPS port.
        let p = HostPolicy::loopback(17618).with_remote("pc.example.ts.net:8443");
        assert!(p.host_ok("pc.example.ts.net:8443") && p.is_remote("pc.example.ts.net:8443"));
        assert!(p.origin_ok("https://pc.example.ts.net:8443"));
        for bad in [
            "pc.example.ts.net",
            "pc.example.ts.net:443",
            "pc.example.ts.net:84430",
        ] {
            assert!(!p.host_ok(bad) && !p.is_remote(bad), "{bad:?}");
        }
        assert!(!p.origin_ok("https://pc.example.ts.net"));
    }

    #[test]
    fn a_tailnet_name_is_a_plain_host_with_an_optional_port_other_than_443() {
        for good in [
            "pc.example.ts.net",
            "pc.example.ts.net:8443",
            "a-b.c1.ts.net:1",
            "pc.ts.net:65535",
        ] {
            assert!(tailnet_name_ok(good), "{good:?}");
        }
        for bad in [
            "",
            "pc",
            "PC.example.ts.net",
            "pc.example.ts.net:443",
            "pc.example.ts.net:",
            "pc.example.ts.net:0",
            "pc.example.ts.net:08443",
            "pc.example.ts.net:65536",
            "pc.example.ts.net:+8443",
            "pc.example.ts.net:84 3",
            "pc.example.ts.net:8443:1",
            ":8443",
            "10.0.0.1:8443",
            "https://pc.example.ts.net",
            "pc.example.ts.net/",
            "user@pc.ts.net",
            "pc..ts.net",
            "-pc.ts.net",
        ] {
            assert!(!tailnet_name_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn host_and_origin_are_exact() {
        let p = HostPolicy::loopback(17615);
        for ok in ["127.0.0.1:17615", "localhost:17615", "[::1]:17615"] {
            assert!(p.host_ok(ok), "{ok}");
            assert!(p.origin_ok(&format!("http://{ok}")), "{ok}");
        }
        for bad in [
            "",
            "127.0.0.1",
            "127.0.0.1:1",
            "127.0.0.1:17615.evil.example",
            "evil.example",
            "LOCALHOST:17615",
            "0x7f.1:17615",
            "127.0.0.1:17615 ",
            " 127.0.0.1:17615",
            "localhost:17615/",
        ] {
            assert!(!p.host_ok(bad), "{bad:?}");
        }
        for bad in [
            "null",
            "",
            "https://127.0.0.1:17615",
            "http://127.0.0.1:17615/",
            "http://evil.example",
            "http://127.0.0.1:17615.evil.example",
        ] {
            assert!(!p.origin_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn fetch_site_rules() {
        assert!(fetch_site_ok(None));
        assert!(fetch_site_ok(Some("same-origin")));
        assert!(fetch_site_ok(Some("none")));
        assert!(!fetch_site_ok(Some("cross-site")));
        assert!(!fetch_site_ok(Some("same-site")));
        assert!(!fetch_site_ok(Some("")));
    }

    #[test]
    fn a_pairing_code_is_good_once() {
        let (_, c) = clock();
        let store = PairingStore::new(c, Duration::from_secs(60));
        let code = store.mint().unwrap().to_hex();
        assert!(store.redeem(&code));
        assert!(!store.redeem(&code));
    }

    #[test]
    fn a_pairing_code_expires() {
        let (manual, c) = clock();
        let store = PairingStore::new(c, Duration::from_secs(60));
        let code = store.mint().unwrap().to_hex();
        manual.advance(Duration::from_secs(61));
        assert!(!store.redeem(&code));
        // The lifetime is a limit, not a cliff for the next one.
        let fresh = store.mint().unwrap().to_hex();
        manual.advance(Duration::from_secs(59));
        assert!(store.redeem(&fresh));
    }

    #[test]
    fn an_unknown_or_malformed_code_is_refused() {
        let (_, c) = clock();
        let store = PairingStore::new(c, Duration::from_secs(60));
        let _ = store.mint().unwrap();
        assert!(!store.redeem(&"0".repeat(32)));
        assert!(!store.redeem("short"));
        assert!(!store.redeem(""));
    }

    #[test]
    fn a_session_is_valid_until_it_is_evicted() {
        let store = SessionStore::new(2);
        let a = store.create().unwrap().to_hex();
        let b = store.create().unwrap().to_hex();
        assert!(store.check(&a) && store.check(&b));
        let c = store.create().unwrap().to_hex();
        assert!(!store.check(&a), "the oldest session should be gone");
        assert!(store.check(&b) && store.check(&c));
        store.clear();
        assert!(!store.check(&b));
        assert!(!store.check("nonsense"));
    }

    #[test]
    fn closing_a_device_closes_its_sessions_and_no_other() {
        let store = SessionStore::new(8);
        let code = store.create().unwrap().to_hex();
        let phone = store.create_for_device("p1").unwrap().to_hex();
        let phone_again = store.create_for_device("p1").unwrap().to_hex();
        let tablet = store.create_for_device("t1").unwrap().to_hex();
        assert_eq!(store.device_of(&phone).as_deref(), Some("p1"));
        assert_eq!(store.device_of(&code), None);
        assert_eq!(store.device_of("nonsense"), None);
        store.close_device("p1");
        assert!(!store.check(&phone) && !store.check(&phone_again));
        assert!(store.check(&code) && store.check(&tablet));
    }

    #[test]
    fn the_rate_limit_is_a_sliding_window() {
        let (manual, c) = clock();
        let limit = RateLimiter::new(c, 3, Duration::from_secs(10));
        assert!(limit.allow() && limit.allow() && limit.allow());
        assert!(!limit.allow());
        manual.advance(Duration::from_secs(10));
        assert!(limit.allow());
    }
}
