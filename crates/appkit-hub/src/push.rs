//! Web Push from the hub to paired devices (VAPID, RFC 8030, 8291, 8292), without an account at any push provider.
//!
//! A device subscribes for kinds of events (`ci_red`, `reminder`, ...) with the endpoint its push service gave it and
//! the keys to encrypt for it: a browser's `PushManager` gives them, and so does a UnifiedPush distributor on Android
//! (the same format). The hub keeps the subscriptions per device and sends to them when the app says that something
//! happened. A revoked device's subscriptions go with it.
//!
//! What a push carries is a hint, not content (`PWA-06`, `MOB-02`): the kind of event and at most the id of an entry
//! whose class is released for devices. The phone fetches the rest over its paired way. The push service is somebody
//! else's server, even with the payload encrypted.
//!
//! The VAPID key of the hub is made once and kept in the settings folder of the app (`push-key.json`), never in a
//! repository; its public half is what a device subscribes with. The subscriptions are in `push.json` beside it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use appkit_core::classes::DataClass;
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer as _;
use p256::elliptic_curve::sec1::ToSec1Point as _;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::BoxFuture;

/// The most subscriptions a hub keeps (all devices together).
pub const MAX_SUBSCRIPTIONS: usize = 32;
/// The kinds one subscription may name.
pub const MAX_KINDS: usize = 8;
/// The longest endpoint taken.
pub const MAX_ENDPOINT: usize = 1024;
/// How long a push service keeps a message for an offline device.
const TTL_SECS: u64 = 4 * 3600;
/// How long a VAPID token is good (RFC 8292 allows up to 24 h).
const VAPID_SECS: u64 = 12 * 3600;
/// The record size of the aes128gcm content coding.
const RECORD_SIZE: u32 = 4096;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ── base64url without padding (RFC 4648 §5), as Web Push writes keys ─────────

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Bytes as base64url without padding.
#[must_use]
pub fn b64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        let take = chunk.len() + 1;
        for i in 0..take {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// base64url without padding (padding is taken too) to bytes; `None` for anything else.
#[must_use]
pub fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = B64.iter().position(|&b| b == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // Leftover bits must be zero (a canonical encoding).
    (acc == 0).then_some(out)
}

// ── the subscription of a device ──────────────────────────────────────────────

/// One subscription: where to send, how to encrypt for it, and for which kinds of event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    /// The device's public key, SEC1 uncompressed P-256, base64url.
    pub p256dh: String,
    /// The device's authentication secret, 16 bytes, base64url.
    pub auth: String,
    pub kinds: Vec<String>,
}

/// Is `kind` a plain word of the app: lower-case letters and `_`, 1 to 32?
#[must_use]
pub fn plain_kind(kind: &str) -> bool {
    (1..=32).contains(&kind.len()) && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// Is `endpoint` a URL a hub may send to: `https`, a host name (no address, no `localhost`, no name of the local
/// network), no user, no port but 443, no fragment, at most [`MAX_ENDPOINT`] plain characters? The hub posts to it, so
/// a device must not be able to point it at the PC or the local network.
#[must_use]
pub fn allowed_endpoint(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return false;
    };
    if endpoint.len() > MAX_ENDPOINT
        || !endpoint.bytes().all(|b| b.is_ascii_graphic())
        || endpoint.contains('#')
    {
        return false;
    }
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let host = match authority.split_once(':') {
        Some((host, "443")) => host,
        Some(_) => return false,
        None => authority,
    };
    let labels: Vec<&str> = host.split('.').collect();
    let lower = host.to_ascii_lowercase();
    !host.contains('@')
        && labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        && labels
            .last()
            .is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_alphabetic()))
        && ![
            "localhost",
            "local",
            "lan",
            "home",
            "internal",
            "arpa",
            "ts.net",
        ]
        .iter()
        .any(|bad| lower == *bad || lower.ends_with(&format!(".{bad}")))
}

impl Subscription {
    /// Checked as the hub takes it: an allowed endpoint, a P-256 point, 16 bytes of secret, plain kinds.
    #[must_use]
    pub fn valid(&self) -> bool {
        allowed_endpoint(&self.endpoint)
            && b64url_decode(&self.p256dh)
                .is_some_and(|k| k.len() == 65 && p256::PublicKey::from_sec1_bytes(&k).is_ok())
            && b64url_decode(&self.auth).is_some_and(|a| a.len() == 16)
            && !self.kinds.is_empty()
            && self.kinds.len() <= MAX_KINDS
            && self.kinds.iter().all(|k| plain_kind(k))
    }
}

// ── encryption (RFC 8291) and the VAPID token (RFC 8292) ────────────────────

fn random(len: usize) -> Result<Vec<u8>, String> {
    let hex = appkit_core::authn::Secret::generate(len)
        .map_err(|_| "the system has no random source".to_owned())?
        .to_hex();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn new_secret_key() -> Result<p256::SecretKey, String> {
    loop {
        if let Ok(key) = p256::SecretKey::from_slice(&random(32)?) {
            return Ok(key);
        }
    }
}

/// Encrypt `plaintext` for a subscriber (`ua_public`: SEC1 uncompressed, `auth`: 16 bytes) with the ephemeral key
/// `as_secret` and `salt`: the body of the push request in the `aes128gcm` content coding (RFC 8188, 8291).
///
/// # Errors
///
/// The subscriber's key is not a P-256 point, or the message is too large for one record.
pub fn encrypt(
    plaintext: &[u8],
    ua_public: &[u8],
    auth: &[u8],
    as_secret: &p256::SecretKey,
    salt: &[u8; 16],
) -> Result<Vec<u8>, String> {
    if plaintext.len() + 1 + 16 > RECORD_SIZE as usize {
        return Err("the message is too large".to_owned());
    }
    let ua =
        p256::PublicKey::from_sec1_bytes(ua_public).map_err(|_| "not a P-256 key".to_owned())?;
    let as_public = as_secret.public_key().to_sec1_point(false);
    let as_public = as_public.as_bytes();
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua.as_affine());
    // PRK_key = HKDF(auth, ecdh); IKM = Expand(PRK_key, "WebPush: info" 0 ua as, 32)
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|_| "hkdf".to_owned())?;
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|_| "hkdf".to_owned())?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| "hkdf".to_owned())?;
    // One record, the last: the delimiter 2 after the text.
    let mut record = plaintext.to_vec();
    record.push(2);
    let cipher = aes_gcm::Aes128Gcm::new_from_slice(&cek).map_err(|_| "aes".to_owned())?;
    let sealed = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &record,
                aad: b"",
            },
        )
        .map_err(|_| "aes".to_owned())?;
    let mut body = Vec::with_capacity(16 + 4 + 1 + 65 + sealed.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(65);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&sealed);
    Ok(body)
}

/// The origin of an endpoint (`https://host[:port]`), the audience of its VAPID token.
fn origin_of(endpoint: &str) -> String {
    let rest = endpoint.strip_prefix("https://").unwrap_or(endpoint);
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    format!("https://{authority}")
}

/// The `Authorization` header of a push request (RFC 8292): `vapid t=<ES256 JWT for the endpoint's origin>,
/// k=<public key>`. The token names no person: `sub` is left out.
#[must_use]
pub fn vapid_header(key: &p256::ecdsa::SigningKey, endpoint: &str, now: u64) -> String {
    let header = b64url(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = serde_json::json!({ "aud": origin_of(endpoint), "exp": now + VAPID_SECS });
    let claims = b64url(claims.to_string().as_bytes());
    let input = format!("{header}.{claims}");
    let signature: p256::ecdsa::Signature = key.sign(input.as_bytes());
    let public = key.verifying_key().to_sec1_point(false);
    format!(
        "vapid t={input}.{}, k={}",
        b64url(&signature.to_bytes()),
        b64url(public.as_bytes())
    )
}

// ── sending ─────────────────────────────────────────────────────────────────

/// One push request as it goes out.
#[derive(Debug, Clone)]
pub struct PushRequest {
    pub endpoint: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

/// The network under the push: reqwest in an app (feature `push-net`), a stand-in in a test. Gives the status of the
/// push service's answer, or `None` when there was none.
pub trait PushTransport: Send + Sync {
    fn send(&self, request: PushRequest) -> BoxFuture<'_, Option<u16>>;
}

/// What one notice did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sent {
    /// Taken by the push service.
    pub delivered: usize,
    /// Refused or not answered (kept: maybe later).
    pub failed: usize,
    /// Gone at the push service (404, 410): forgotten.
    pub dropped: usize,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct KeyFile {
    schema: u32,
    /// The VAPID signing key, 32 bytes, hex. Secret: the file is the person's only.
    key: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct SubsFile {
    schema: u32,
    /// Per device id.
    devices: BTreeMap<String, Subscription>,
}

/// The routes of push (`GuardRules::with_push`): the public key, subscribing and unsubscribing, for devices.
pub const PUSH_KEY_PATH: &str = "/api/push/key";
pub const PUSH_SUBSCRIBE_PATH: &str = "/api/push/subscribe";
pub const PUSH_UNSUBSCRIBE_PATH: &str = "/api/push/unsubscribe";

/// The push of a hub: its VAPID key, the subscriptions, and the way out.
pub struct Push {
    key: p256::ecdsa::SigningKey,
    subs_path: PathBuf,
    subs: Mutex<BTreeMap<String, Subscription>>,
    transport: Arc<dyn PushTransport>,
}

fn read_small(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
        Ok(m) if m.len() > 64 * 1024 => {
            Err(std::io::Error::other("the file is larger than allowed"))
        }
        Ok(_) => std::fs::read_to_string(path).map(Some),
    }
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

impl std::fmt::Debug for Push {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Neither the key nor an endpoint (a capability of the device) is printed.
        f.write_str("Push { .. }")
    }
}

impl Push {
    /// The push of a hub whose settings are in `dir`: the key in `push-key.json` (made when it is not there), the
    /// subscriptions in `push.json`.
    ///
    /// # Errors
    ///
    /// A file is there but cannot be read or is not one of these (the hub does not start rather than make a new key
    /// that no device knows), or the folder cannot be written.
    pub fn open(dir: &Path, transport: Arc<dyn PushTransport>) -> std::io::Result<Self> {
        let key_path = dir.join("push-key.json");
        let key = match read_small(&key_path)? {
            Some(text) => {
                let file: KeyFile = serde_json::from_str(&text)
                    .map_err(|_| std::io::Error::other("push-key.json cannot be read"))?;
                let bytes = (file.schema == 1 && file.key.len() == 64)
                    .then(|| {
                        (0..64)
                            .step_by(2)
                            .map(|i| u8::from_str_radix(&file.key[i..i + 2], 16).ok())
                            .collect::<Option<Vec<u8>>>()
                    })
                    .flatten()
                    .ok_or_else(|| std::io::Error::other("push-key.json is not a key"))?;
                p256::ecdsa::SigningKey::from_slice(&bytes)
                    .map_err(|_| std::io::Error::other("push-key.json is not a key"))?
            }
            None => {
                let secret = new_secret_key().map_err(std::io::Error::other)?;
                let key = p256::ecdsa::SigningKey::from(&secret);
                let hex: String = key.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
                let text = serde_json::to_string(&KeyFile {
                    schema: 1,
                    key: hex,
                })
                .map_err(std::io::Error::other)?;
                write_atomic(&key_path, &text)?;
                key
            }
        };
        let subs_path = dir.join("push.json");
        let subs = match read_small(&subs_path)? {
            None => BTreeMap::new(),
            Some(text) => {
                let file: SubsFile = serde_json::from_str(&text)
                    .map_err(|_| std::io::Error::other("push.json cannot be read"))?;
                if file.schema != 1
                    || file.devices.len() > MAX_SUBSCRIPTIONS
                    || !file.devices.values().all(Subscription::valid)
                {
                    return Err(std::io::Error::other(
                        "push.json is not a list of subscriptions",
                    ));
                }
                file.devices
            }
        };
        Ok(Self {
            key,
            subs_path,
            subs: Mutex::new(subs),
            transport,
        })
    }

    /// The public key a device subscribes with (`applicationServerKey`), base64url.
    #[must_use]
    pub fn public_key(&self) -> String {
        b64url(self.key.verifying_key().to_sec1_point(false).as_bytes())
    }

    fn persist(&self, subs: &BTreeMap<String, Subscription>) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&SubsFile {
            schema: 1,
            devices: subs.clone(),
        })
        .map_err(std::io::Error::other)?;
        write_atomic(&self.subs_path, &text)
    }

    /// Keep the subscription of a device, in place of the one before.
    ///
    /// # Errors
    ///
    /// `InvalidInput` for a subscription that is not valid or a full list; else the file could not be written.
    pub fn subscribe(&self, device: &str, sub: Subscription) -> std::io::Result<()> {
        if !sub.valid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a subscription",
            ));
        }
        let mut subs = lock(&self.subs);
        if !subs.contains_key(device) && subs.len() >= MAX_SUBSCRIPTIONS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the list is full",
            ));
        }
        let mut next = subs.clone();
        next.insert(device.to_owned(), sub);
        self.persist(&next)?;
        *subs = next;
        Ok(())
    }

    /// Forget the subscription of a device (it unsubscribed, or was revoked). `Ok(false)`: it had none.
    ///
    /// # Errors
    ///
    /// The file could not be written.
    pub fn forget(&self, device: &str) -> std::io::Result<bool> {
        let mut subs = lock(&self.subs);
        if !subs.contains_key(device) {
            return Ok(false);
        }
        let mut next = subs.clone();
        next.remove(device);
        self.persist(&next)?;
        *subs = next;
        Ok(true)
    }

    /// The kinds a device subscribed to.
    #[must_use]
    pub fn kinds_of(&self, device: &str) -> Vec<String> {
        lock(&self.subs)
            .get(device)
            .map(|s| s.kinds.clone())
            .unwrap_or_default()
    }

    /// The text of a notice: the kind, and the id of the entry only when its class is released for devices.
    #[must_use]
    pub fn payload(kind: &str, item: Option<(&str, DataClass)>) -> String {
        let id = item
            .filter(|(id, class)| {
                class.released_for_device()
                    && (1..=140).contains(&id.len())
                    && id.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/')
                    })
            })
            .map(|(id, _)| id);
        serde_json::json!({ "kind": kind, "id": id }).to_string()
    }

    /// Tell every device that subscribed to `kind` that it happened, with the id of the entry when its class allows
    /// it. A subscription the push service says is gone (404, 410) is forgotten.
    pub async fn notify(&self, kind: &str, item: Option<(&str, DataClass)>, urgent: bool) -> Sent {
        let mut sent = Sent::default();
        if !plain_kind(kind) {
            return sent;
        }
        let payload = Self::payload(kind, item);
        let targets: Vec<(String, Subscription)> = lock(&self.subs)
            .iter()
            .filter(|(_, s)| s.kinds.iter().any(|k| k == kind))
            .map(|(d, s)| (d.clone(), s.clone()))
            .collect();
        let now = unix_now();
        for (device, sub) in targets {
            let (Some(ua), Some(auth)) = (b64url_decode(&sub.p256dh), b64url_decode(&sub.auth))
            else {
                sent.failed += 1;
                continue;
            };
            let body = new_secret_key().and_then(|eph| {
                let salt: [u8; 16] = random(16)?.try_into().map_err(|_| "salt".to_owned())?;
                encrypt(payload.as_bytes(), &ua, &auth, &eph, &salt)
            });
            let Ok(body) = body else {
                sent.failed += 1;
                continue;
            };
            let request = PushRequest {
                endpoint: sub.endpoint.clone(),
                headers: vec![
                    ("TTL", TTL_SECS.to_string()),
                    ("Urgency", if urgent { "high" } else { "normal" }.to_owned()),
                    ("Content-Encoding", "aes128gcm".to_owned()),
                    ("Content-Type", "application/octet-stream".to_owned()),
                    ("Authorization", vapid_header(&self.key, &sub.endpoint, now)),
                ],
                body,
            };
            match self.transport.send(request).await {
                Some(200..=202) => sent.delivered += 1,
                Some(404 | 410) => {
                    let _ = self.forget(&device);
                    sent.dropped += 1;
                }
                _ => sent.failed += 1,
            }
        }
        sent
    }
}

/// The time a push service gets to answer.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    /// The receiving side of RFC 8291, as a phone does it: the test reads what the hub sent.
    fn decrypt(body: &[u8], ua_secret: &p256::SecretKey, auth: &[u8]) -> Vec<u8> {
        let salt = &body[..16];
        let rs = u32::from_be_bytes(body[16..20].try_into().unwrap());
        assert_eq!(rs, RECORD_SIZE);
        assert_eq!(body[20], 65);
        let as_public = &body[21..86];
        let ua_public = ua_secret.public_key().to_sec1_point(false);
        let shared = p256::ecdh::diffie_hellman(
            ua_secret.to_nonzero_scalar(),
            p256::PublicKey::from_sec1_bytes(as_public)
                .unwrap()
                .as_affine(),
        );
        let mut key_info = b"WebPush: info\0".to_vec();
        key_info.extend_from_slice(ua_public.as_bytes());
        key_info.extend_from_slice(as_public);
        let mut ikm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
            .expand(&key_info, &mut ikm)
            .unwrap();
        let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
        prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
            .unwrap();
        prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
            .unwrap();
        let mut plain = aes_gcm::Aes128Gcm::new_from_slice(&cek)
            .unwrap()
            .decrypt((&nonce).into(), &body[86..])
            .unwrap();
        assert_eq!(plain.pop(), Some(2), "the last record");
        plain
    }

    struct Capture {
        seen: Mutex<Vec<PushRequest>>,
        answer: u16,
    }

    impl PushTransport for Capture {
        fn send(&self, request: PushRequest) -> BoxFuture<'_, Option<u16>> {
            lock(&self.seen).push(request);
            let answer = self.answer;
            Box::pin(async move { Some(answer) })
        }
    }

    fn device_keys() -> (p256::SecretKey, String, Vec<u8>, String) {
        let secret = new_secret_key().unwrap();
        let public = b64url(secret.public_key().to_sec1_point(false).as_bytes());
        let auth = random(16).unwrap();
        let auth_text = b64url(&auth);
        (secret, public, auth, auth_text)
    }

    /// The example of RFC 8291, Appendix A: the same keys, secret and salt give the same message, byte for byte.
    #[test]
    fn the_message_of_rfc_8291_appendix_a() {
        let as_secret = p256::SecretKey::from_slice(
            &b64url_decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw").unwrap(),
        )
        .unwrap();
        let ua_public = b64url_decode(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        )
        .unwrap();
        let auth = b64url_decode("BTBZMqHH6r4Tts7J_aSIgg").unwrap();
        let salt: [u8; 16] = b64url_decode("DGv6ra1nlYgDCS1FRnbzlw")
            .unwrap()
            .try_into()
            .unwrap();
        let body = encrypt(
            b"When I grow up, I want to be a watermelon",
            &ua_public,
            &auth,
            &as_secret,
            &salt,
        )
        .unwrap();
        assert_eq!(
            b64url(&body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
        let ua_secret = p256::SecretKey::from_slice(
            &b64url_decode("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94").unwrap(),
        )
        .unwrap();
        assert_eq!(
            decrypt(&body, &ua_secret, &auth),
            b"When I grow up, I want to be a watermelon"
        );
    }

    #[test]
    fn base64url_reads_back_and_refuses_what_is_not() {
        for n in 0..70u8 {
            let bytes: Vec<u8> = (0..n).collect();
            assert_eq!(b64url_decode(&b64url(&bytes)), Some(bytes));
        }
        assert_eq!(b64url(b"\xfb\xff"), "-_8");
        for bad in ["a", "ab+c", "ab/c", "a b", "ab$"] {
            assert_eq!(b64url_decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_device_reads_what_the_hub_encrypted() {
        let (secret, public, auth, _) = device_keys();
        let eph = new_secret_key().unwrap();
        let body = encrypt(
            br#"{"kind":"ci_red","id":null}"#,
            &b64url_decode(&public).unwrap(),
            &auth,
            &eph,
            &[7; 16],
        )
        .unwrap();
        assert_eq!(
            decrypt(&body, &secret, &auth),
            br#"{"kind":"ci_red","id":null}"#
        );
        assert!(encrypt(
            &[0; 5000],
            &b64url_decode(&public).unwrap(),
            &auth,
            &eph,
            &[7; 16]
        )
        .is_err());
    }

    #[test]
    fn the_vapid_token_names_the_push_service_and_is_signed_by_the_hub() {
        use p256::ecdsa::signature::Verifier;
        let key = p256::ecdsa::SigningKey::from(&new_secret_key().unwrap());
        let header = vapid_header(&key, "https://push.example.org/wpush/v2/abc?x=1", 1000);
        let (t, k) = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        let parts: Vec<&str> = t.split('.').collect();
        let claims: serde_json::Value =
            serde_json::from_slice(&b64url_decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.org");
        assert_eq!(claims["exp"], 1000 + VAPID_SECS);
        assert!(claims.get("sub").is_none(), "no person is named");
        let verifying =
            p256::ecdsa::VerifyingKey::from_sec1_bytes(&b64url_decode(k).unwrap()).unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&b64url_decode(parts[2]).unwrap()).unwrap();
        assert!(verifying
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .is_ok());
    }

    #[test]
    fn the_hub_posts_only_to_push_services_out_there() {
        for ok in [
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://updates.push.services.mozilla.com/wpush/v2/gAAA",
            "https://ntfy.sh/upAbc?up=1",
            "https://push.example.org:443/x",
        ] {
            assert!(allowed_endpoint(ok), "{ok}");
        }
        for bad in [
            "http://push.example.org/x",
            "https://127.0.0.1/x",
            "https://[::1]/x",
            "https://localhost/x",
            "https://pc.local/x",
            "https://pc.example.ts.net/x",
            "https://push.example.org:8443/x",
            "https://user@push.example.org/x",
            "https://10.0.0.1/x",
            "https://push.example.org/x#y",
            "https://push.example.org/a b",
            "https://nas/x",
            &format!("https://push.example.org/{}", "a".repeat(MAX_ENDPOINT)),
        ] {
            assert!(!allowed_endpoint(bad), "{bad}");
        }
    }

    #[test]
    fn a_notice_carries_the_kind_and_only_a_released_id() {
        assert_eq!(
            Push::payload("ci_red", Some(("lib.nvim", DataClass::Internal))),
            r#"{"id":"lib.nvim","kind":"ci_red"}"#
        );
        for class in [DataClass::Private, DataClass::Unclassified] {
            assert_eq!(
                Push::payload("ci_red", Some(("secret-repo", class))),
                r#"{"id":null,"kind":"ci_red"}"#
            );
        }
        assert_eq!(
            Push::payload("ci_red", Some(("a b<c>", DataClass::Public))),
            r#"{"id":null,"kind":"ci_red"}"#
        );
    }

    #[tokio::test]
    async fn subscribers_of_a_kind_get_it_encrypted_and_a_gone_one_is_forgotten() {
        let dir = Scratch::new("push");
        let capture = Arc::new(Capture {
            seen: Mutex::new(Vec::new()),
            answer: 201,
        });
        let push = Push::open(&dir, capture.clone()).unwrap();
        let (secret, public, auth, auth_text) = device_keys();
        let sub = Subscription {
            endpoint: "https://push.example.org/wpush/abc".to_owned(),
            p256dh: public.clone(),
            auth: auth_text.clone(),
            kinds: vec!["ci_red".to_owned()],
        };
        push.subscribe("d1", sub.clone()).unwrap();
        // Not valid: refused, nothing kept.
        let mut bad = sub.clone();
        bad.endpoint = "https://192.168.1.10/x".to_owned();
        assert!(push.subscribe("d2", bad).is_err());
        let sent = push
            .notify("ci_red", Some(("secret-repo", DataClass::Private)), true)
            .await;
        assert_eq!(sent.delivered, 1);
        let req = lock(&capture.seen).pop().unwrap();
        assert_eq!(req.endpoint, sub.endpoint);
        let header = |n: &str| req.headers.iter().find(|(k, _)| *k == n).unwrap().1.clone();
        assert_eq!(header("Content-Encoding"), "aes128gcm");
        assert_eq!(header("Urgency"), "high");
        assert!(header("Authorization").ends_with(&format!("k={}", push.public_key())));
        let plain = decrypt(&req.body, &secret, &auth);
        assert_eq!(plain, br#"{"id":null,"kind":"ci_red"}"#);
        assert!(!String::from_utf8_lossy(&req.body).contains("secret-repo"));
        // Another kind reaches nobody.
        assert_eq!(push.notify("reminder", None, false).await, Sent::default());
        // The key and the subscriptions outlive a restart; a gone subscription is dropped.
        let gone = Arc::new(Capture {
            seen: Mutex::new(Vec::new()),
            answer: 410,
        });
        let again = Push::open(&dir, gone).unwrap();
        assert_eq!(again.public_key(), push.public_key());
        assert_eq!(again.notify("ci_red", None, false).await.dropped, 1);
        assert!(again.kinds_of("d1").is_empty());
        assert!(Push::open(&dir, capture).unwrap().kinds_of("d1").is_empty());
    }

    #[test]
    fn files_that_are_not_ours_stop_the_start() {
        let dir = Scratch::new("push-bad");
        let capture: Arc<dyn PushTransport> = Arc::new(Capture {
            seen: Mutex::new(Vec::new()),
            answer: 201,
        });
        std::fs::write(dir.join("push-key.json"), r#"{"schema":1,"key":"zz"}"#).unwrap();
        assert!(Push::open(&dir, capture.clone()).is_err());
        std::fs::remove_file(dir.join("push-key.json")).unwrap();
        std::fs::write(
            dir.join("push.json"),
            r#"{"schema":1,"devices":{"d":{"endpoint":"http://x","p256dh":"","auth":"","kinds":[]}}}"#,
        )
        .unwrap();
        assert!(Push::open(&dir, capture).is_err());
    }
}
