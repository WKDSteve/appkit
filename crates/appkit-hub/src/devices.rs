//! Devices: a phone (or any other browser that installs an app's page) is paired once and from then on proves itself
//! with a key of its own, so that it never needs a pairing code again.
//!
//! At pairing the page makes an ECDSA P-256 key pair that WebCrypto keeps non-extractable (a script of the page can
//! sign with it, nobody can read it out) and sends the public half with the one-time pairing code. The hub keeps the
//! public key in a list of devices, in a file of the settings (`devices.json`), so the pairing outlives a restart.
//! To open a session the page asks for a challenge (random, one use, short-lived, bound to the device) and signs it
//! together with the origin it speaks to; the hub checks the signature against the key it keeps. A revoked device is
//! gone from the list, and its open sessions are closed.
//!
//! Nothing secret is in the file: public keys, names the person gave and times. A key never leaves the device.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use appkit_core::authn::{Clock, Secret};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

/// The most devices one hub keeps.
pub const MAX_DEVICES: usize = 16;
/// How long a challenge may be answered.
pub const CHALLENGE_TTL: Duration = Duration::from_secs(60);
/// The most challenges open at once (the oldest goes first).
const MAX_CHALLENGES: usize = 64;
/// The longest name a device may have, in characters.
pub const MAX_NAME: usize = 40;
/// The most the file may be when it is read.
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Bytes of a device id and of a challenge.
const ID_BYTES: usize = 16;
const CHALLENGE_BYTES: usize = 32;

/// What a device signs to open a session: a fixed word, the origin it speaks to, its id and the challenge.
#[must_use]
pub fn login_message(origin: &str, device: &str, challenge: &str) -> String {
    format!("appkit-device-login/1\n{origin}\n{device}\n{challenge}")
}

/// One device as the file keeps it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Stored {
    id: String,
    name: String,
    /// The public key, SEC1 uncompressed (65 bytes), as lower-case hex.
    key: String,
    /// Seconds since 1970.
    paired: u64,
    last_used: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    schema: u32,
    devices: Vec<Stored>,
}

/// A device as the person sees it on the PC: no key.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub paired: u64,
    pub last_used: Option<u64>,
}

/// Why a pairing was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    /// The name is empty, too long or holds a control or hidden character.
    BadName,
    /// The key is not a P-256 public key.
    BadKey,
    /// The list is full: revoke a device first.
    Full,
    /// The same key is paired already.
    Known,
    /// The list could not be written (nothing was paired).
    Io,
    /// The system has no random source.
    Random,
}

struct Challenge {
    device: String,
    value: String,
    until: Duration,
}

/// The devices of a hub and the challenges open for them.
pub struct DeviceStore {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    devices: Mutex<Vec<Stored>>,
    challenges: Mutex<Vec<Challenge>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn is_hex(text: &str, bytes: usize) -> bool {
    text.len() == bytes * 2 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// A name the person gave the device: trimmed, one line, printable, no hidden or reordering characters.
fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    let hidden = |c: char| {
        c.is_control()
            || matches!(c as u32, 0xAD | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x2069 | 0xFEFF)
    };
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(hidden) {
        return None;
    }
    Some(name.to_owned())
}

/// A public key as the page sends it: SEC1 uncompressed P-256 (65 bytes) in lower-case hex.
fn parse_key(hex: &str) -> Option<VerifyingKey> {
    if !is_hex(hex, 65) {
        return None;
    }
    VerifyingKey::from_sec1_bytes(&decode_hex(hex)?).ok()
}

fn valid(stored: &Stored) -> bool {
    is_hex(&stored.id, ID_BYTES)
        && clean_name(&stored.name).as_deref() == Some(stored.name.as_str())
        && parse_key(&stored.key).is_some()
}

/// Would this name and key be taken for a pairing? Checked before the one-time code is spent.
///
/// # Errors
///
/// [`PairError::BadName`] or [`PairError::BadKey`].
pub fn check_pairing(name: &str, key: &str) -> Result<(), PairError> {
    clean_name(name).ok_or(PairError::BadName)?;
    parse_key(key).map(|_| ()).ok_or(PairError::BadKey)
}

impl DeviceStore {
    /// The devices kept in `path`; none when the file is not there yet.
    ///
    /// # Errors
    ///
    /// The file is there but cannot be read, is larger than allowed, or is not a list of devices: the hub does not
    /// start rather than forget (or half read) the devices it trusts.
    pub fn open(path: &Path, clock: Arc<dyn Clock>) -> std::io::Result<Self> {
        let devices = match std::fs::metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
            Ok(meta) => {
                if meta.len() > MAX_FILE_BYTES {
                    return Err(std::io::Error::other(
                        "the list of devices is larger than allowed",
                    ));
                }
                let text = std::fs::read_to_string(path)?;
                let file: File = serde_json::from_str(&text).map_err(|e| {
                    std::io::Error::other(format!("the list of devices cannot be read: {e}"))
                })?;
                if file.schema != 1
                    || file.devices.len() > MAX_DEVICES
                    || !file.devices.iter().all(valid)
                {
                    return Err(std::io::Error::other(
                        "the list of devices is not one this hub knows",
                    ));
                }
                file.devices
            }
        };
        Ok(Self {
            path: path.to_owned(),
            clock,
            devices: Mutex::new(devices),
            challenges: Mutex::new(Vec::new()),
        })
    }

    /// Write the list: to a file beside it first, then in place, so a crash leaves the old list or the new one.
    fn persist(&self, devices: &[Stored]) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&File {
            schema: 1,
            devices: devices.to_vec(),
        })
        .map_err(std::io::Error::other)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Pair a device: its name and public key (SEC1 uncompressed, hex). Returns the new device's id.
    ///
    /// # Errors
    ///
    /// [`PairError`]: what was wrong; nothing is kept then.
    pub fn pair(&self, name: &str, key: &str) -> Result<String, PairError> {
        let name = clean_name(name).ok_or(PairError::BadName)?;
        if parse_key(key).is_none() {
            return Err(PairError::BadKey);
        }
        let id = Secret::generate(ID_BYTES)
            .map_err(|_| PairError::Random)?
            .to_hex();
        let mut devices = lock(&self.devices);
        if devices.iter().any(|d| d.key == key) {
            return Err(PairError::Known);
        }
        if devices.len() >= MAX_DEVICES {
            return Err(PairError::Full);
        }
        let mut next = devices.clone();
        next.push(Stored {
            id: id.clone(),
            name,
            key: key.to_owned(),
            paired: unix_now(),
            last_used: None,
        });
        self.persist(&next).map_err(|_| PairError::Io)?;
        *devices = next;
        Ok(id)
    }

    /// A challenge for a device to sign. An id that is not paired gets one too (it is never accepted): the answer
    /// does not tell which ids exist.
    ///
    /// # Errors
    ///
    /// The system has no random source.
    pub fn challenge(&self, device: &str) -> Result<String, appkit_core::authn::RandomError> {
        let value = Secret::generate(CHALLENGE_BYTES)?.to_hex();
        let now = self.clock.now();
        let mut open = lock(&self.challenges);
        open.retain(|c| c.until > now);
        while open.len() >= MAX_CHALLENGES {
            open.remove(0);
        }
        open.push(Challenge {
            device: device.to_owned(),
            value: value.clone(),
            until: now + CHALLENGE_TTL,
        });
        Ok(value)
    }

    /// Does `signature` (P1363, r and s, 64 bytes as hex, as WebCrypto makes it) sign the login message of this
    /// origin, device and challenge with the device's key? The challenge is spent either way.
    pub fn login(&self, device: &str, challenge: &str, signature: &str, origin: &str) -> bool {
        if !is_hex(challenge, CHALLENGE_BYTES) || !is_hex(signature, 64) {
            return false;
        }
        let now = self.clock.now();
        let fresh = {
            let mut open = lock(&self.challenges);
            let at = open.iter().position(|c| c.value == challenge);
            match at {
                Some(i) => {
                    let c = open.remove(i);
                    c.device == device && c.until > now
                }
                None => false,
            }
        };
        if !fresh {
            return false;
        }
        let mut devices = lock(&self.devices);
        let Some(stored) = devices.iter_mut().find(|d| d.id == device) else {
            return false;
        };
        let (Some(key), Some(bytes)) = (parse_key(&stored.key), decode_hex(signature)) else {
            return false;
        };
        let Ok(sig) = Signature::from_slice(&bytes) else {
            return false;
        };
        // WebCrypto does not promise a low S; the check takes either form of the same signature.
        let sig = sig.normalize_s();
        if key
            .verify(login_message(origin, device, challenge).as_bytes(), &sig)
            .is_err()
        {
            return false;
        }
        stored.last_used = Some(unix_now());
        let snapshot = devices.clone();
        drop(devices);
        // When it was last used is a help for the person, not a reason to refuse the session.
        let _ = self.persist(&snapshot);
        true
    }

    /// The paired devices, without their keys.
    #[must_use]
    pub fn list(&self) -> Vec<DeviceInfo> {
        lock(&self.devices)
            .iter()
            .map(|d| DeviceInfo {
                id: d.id.clone(),
                name: d.name.clone(),
                paired: d.paired,
                last_used: d.last_used,
            })
            .collect()
    }

    /// Forget a device. `Ok(false)`: no such device.
    ///
    /// # Errors
    ///
    /// The list could not be written; the device is still paired then.
    pub fn revoke(&self, device: &str) -> std::io::Result<bool> {
        let mut devices = lock(&self.devices);
        if !devices.iter().any(|d| d.id == device) {
            return Ok(false);
        }
        let next: Vec<Stored> = devices.iter().filter(|d| d.id != device).cloned().collect();
        self.persist(&next)?;
        *devices = next;
        lock(&self.challenges).retain(|c| c.device != device);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::authn::ManualClock;
    use appkit_scratch::Scratch;
    use p256::ecdsa::signature::Signer;
    use p256::ecdsa::SigningKey;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn key(seed: u8) -> (SigningKey, String) {
        let signing = SigningKey::from_slice(&[seed; 32]).unwrap();
        let public = hex(signing.verifying_key().to_sec1_point(false).as_bytes());
        (signing, public)
    }

    fn sign(signing: &SigningKey, origin: &str, device: &str, challenge: &str) -> String {
        let sig: Signature = signing.sign(login_message(origin, device, challenge).as_bytes());
        hex(&sig.to_bytes())
    }

    fn store(name: &str) -> (DeviceStore, Arc<ManualClock>, Scratch) {
        let dir = Scratch::new(&format!("devices-{name}"));
        let clock = Arc::new(ManualClock::new());
        let s = DeviceStore::open(&dir.join("devices.json"), clock.clone()).unwrap();
        (s, clock, dir)
    }

    const ORIGIN: &str = "http://127.0.0.1:17615";

    #[test]
    fn a_paired_device_opens_a_session_with_a_signed_challenge_once() {
        let (s, _, _d) = store("login");
        let (signing, public) = key(7);
        let id = s.pair("Fairphone", &public).unwrap();
        let c = s.challenge(&id).unwrap();
        assert!(s.login(&id, &c, &sign(&signing, ORIGIN, &id, &c), ORIGIN));
        // The challenge is spent.
        assert!(!s.login(&id, &c, &sign(&signing, ORIGIN, &id, &c), ORIGIN));
        assert!(s.list()[0].last_used.is_some());
    }

    #[test]
    fn a_login_is_refused_for_another_key_origin_device_or_an_old_challenge() {
        let (s, clock, _d) = store("refuse");
        let (signing, public) = key(7);
        let (other, other_public) = key(9);
        let id = s.pair("A", &public).unwrap();
        let id2 = s.pair("B", &other_public).unwrap();
        // Another key.
        let c = s.challenge(&id).unwrap();
        assert!(!s.login(&id, &c, &sign(&other, ORIGIN, &id, &c), ORIGIN));
        // Signed for another origin (a page elsewhere that relays the challenge).
        let c = s.challenge(&id).unwrap();
        assert!(!s.login(
            &id,
            &c,
            &sign(&signing, "http://127.0.0.1:1", &id, &c),
            ORIGIN
        ));
        // A challenge of another device.
        let c = s.challenge(&id2).unwrap();
        assert!(!s.login(&id, &c, &sign(&signing, ORIGIN, &id, &c), ORIGIN));
        // Expired.
        let c = s.challenge(&id).unwrap();
        clock.advance(CHALLENGE_TTL + Duration::from_secs(1));
        assert!(!s.login(&id, &c, &sign(&signing, ORIGIN, &id, &c), ORIGIN));
        // An unknown device gets a challenge but no session.
        let c = s.challenge("00".repeat(16).as_str()).unwrap();
        assert!(!s.login(
            &"00".repeat(16),
            &c,
            &sign(&signing, ORIGIN, &"00".repeat(16), &c),
            ORIGIN
        ));
        // Garbage.
        let c = s.challenge(&id).unwrap();
        assert!(!s.login(&id, &c, "zz", ORIGIN));
    }

    #[test]
    fn a_high_s_signature_counts_as_the_same_signature() {
        let (s, _, _d) = store("highs");
        let (signing, public) = key(7);
        let id = s.pair("A", &public).unwrap();
        let c = s.challenge(&id).unwrap();
        let sig: Signature = signing.sign(login_message(ORIGIN, &id, &c).as_bytes());
        // The other S of the same signature: n - s.
        let (r, low) = sig.split_scalars();
        let high = Signature::from_scalars(r, -*low).unwrap();
        assert!(s.login(&id, &c, &hex(&high.to_bytes()), ORIGIN));
    }

    #[test]
    fn the_list_outlives_a_restart_and_a_revoked_device_is_gone() {
        let (s, clock, d) = store("restart");
        let (signing, public) = key(7);
        let id = s.pair("Fairphone", &public).unwrap();
        let again = DeviceStore::open(&d.join("devices.json"), clock.clone()).unwrap();
        assert_eq!(again.list().len(), 1);
        let c = again.challenge(&id).unwrap();
        let open = again.challenge(&id).unwrap();
        assert!(again.login(&id, &c, &sign(&signing, ORIGIN, &id, &c), ORIGIN));
        assert!(again.revoke(&id).unwrap());
        assert!(!again.revoke(&id).unwrap());
        // A challenge asked before the revoke does not open a session after it.
        assert!(!again.login(&id, &open, &sign(&signing, ORIGIN, &id, &open), ORIGIN));
        assert!(DeviceStore::open(&d.join("devices.json"), clock)
            .unwrap()
            .list()
            .is_empty());
        // The file holds no private key, only public ones.
        let text = std::fs::read_to_string(d.join("devices.json")).unwrap();
        assert!(!text.contains(&hex(&signing.to_bytes())));
    }

    #[test]
    fn pairing_refuses_bad_names_bad_keys_a_known_key_and_a_full_list() {
        let (s, _, _d) = store("pair");
        let (_, public) = key(7);
        for bad in ["", "   ", "a\nb", "x\u{202E}y", &"n".repeat(MAX_NAME + 1)] {
            assert_eq!(s.pair(bad, &public), Err(PairError::BadName), "{bad:?}");
        }
        for bad in [
            "",
            "04",
            &"ab".repeat(65),
            &public.to_uppercase(),
            &public[..128],
        ] {
            assert_eq!(s.pair("A", bad), Err(PairError::BadKey), "{bad:?}");
        }
        s.pair("A", &public).unwrap();
        assert_eq!(s.pair("B", &public), Err(PairError::Known));
        for seed in 10..(10 + MAX_DEVICES as u8 - 1) {
            s.pair("x", &key(seed).1).unwrap();
        }
        assert_eq!(s.pair("y", &key(99).1), Err(PairError::Full));
    }

    #[test]
    fn a_file_that_is_not_a_list_of_devices_stops_the_start() {
        let (_, clock, d) = store("broken");
        let path = d.join("devices.json");
        for bad in [
            "not json",
            r#"{"schema":2,"devices":[]}"#,
            r#"{"schema":1,"devices":[{"id":"x","name":"a","key":"04","paired":0,"last_used":null}]}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            assert!(DeviceStore::open(&path, clock.clone()).is_err(), "{bad}");
        }
    }
}
