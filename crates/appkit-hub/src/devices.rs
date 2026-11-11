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
//!
//! Each device has a level (`MOB-03`): read, create, change, trigger. A new device gets the app's default (`create`
//! unless the app says otherwise; never `trigger`); the PC raises or lowers it. A route the app released for devices
//! names the level it needs, and the guard checks it per request. `trigger` (something with an effect beyond the vault:
//! a run on the PC) also needs a confirmation on the device for each request: a second key, which the phone keeps bound
//! to its biometrics or its lock (`AND-05`), signs the request itself (its method, path and body) with a fresh
//! challenge. A device paired without that key cannot trigger, whatever its level.

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

/// What the confirmation key of a device signs to trigger one request: a fixed word, the origin, the device, a fresh
/// challenge, the method with the path and query, and the SHA-256 of the body (hex). A confirmation is good for exactly
/// this request, once.
#[must_use]
pub fn confirm_message(
    origin: &str,
    device: &str,
    challenge: &str,
    method: &str,
    path_and_query: &str,
    body_sha256: &str,
) -> String {
    format!(
        "appkit-device-confirm/1\n{origin}\n{device}\n{challenge}\n{method} {path_and_query}\n{body_sha256}"
    )
}

/// What a device may do, from the least to the most (`MOB-03`). The guard compares the level of the device with the
/// level a route needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Read what is released for devices.
    Read,
    /// Also create (a new entry, an answer to a question).
    Create,
    /// Also change (a status, a field).
    Change,
    /// Also start something on the PC, each time with a confirmation on the device.
    Trigger,
}

impl Level {
    /// The level a word names (`read`, `create`, `change`, `trigger`).
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "read" => Some(Self::Read),
            "create" => Some(Self::Create),
            "change" => Some(Self::Change),
            "trigger" => Some(Self::Trigger),
            _ => None,
        }
    }

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Create => "create",
            Self::Change => "change",
            Self::Trigger => "trigger",
        }
    }
}

/// The level of a device the file names none for (paired before there were levels): the kit's default.
fn default_level() -> Level {
    Level::Create
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
    #[serde(default = "default_level")]
    level: Level,
    /// The public key that confirms a trigger (bound to the device's biometrics or lock), as `key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirm_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    schema: u32,
    /// The id of this hub, made once with the list: a device sends nothing to a hub that names another (`MSEC-04`).
    /// A list written before it was there gets one when it is opened.
    #[serde(default)]
    hub: Option<String>,
    devices: Vec<Stored>,
}

/// A device as the person sees it on the PC: no key.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub paired: u64,
    pub last_used: Option<u64>,
    pub level: Level,
    /// Whether the device has a key to confirm a trigger.
    pub confirm: bool,
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
    /// The id of this hub (`hub_id`).
    hub: String,
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
        && stored
            .confirm_key
            .as_deref()
            .is_none_or(|k| parse_key(k).is_some() && k != stored.key)
}

/// Would this name and key be taken for a pairing? Checked before the one-time code is spent.
///
/// # Errors
///
/// [`PairError::BadName`] or [`PairError::BadKey`].
pub fn check_pairing(name: &str, key: &str, confirm_key: Option<&str>) -> Result<(), PairError> {
    clean_name(name).ok_or(PairError::BadName)?;
    parse_key(key).ok_or(PairError::BadKey)?;
    match confirm_key {
        Some(k) if parse_key(k).is_none() || k == key => Err(PairError::BadKey),
        _ => Ok(()),
    }
}

impl DeviceStore {
    /// The devices kept in `path`; none when the file is not there yet.
    ///
    /// # Errors
    ///
    /// The file is there but cannot be read, is larger than allowed, or is not a list of devices: the hub does not
    /// start rather than forget (or half read) the devices it trusts.
    pub fn open(path: &Path, clock: Arc<dyn Clock>) -> std::io::Result<Self> {
        let new_id = || {
            Secret::generate(ID_BYTES)
                .map(|s| s.to_hex())
                .map_err(|_| std::io::Error::other("the system has no random source"))
        };
        let (hub, devices, write) = match std::fs::metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (new_id()?, Vec::new(), false),
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
                    || file.hub.as_deref().is_some_and(|h| !is_hex(h, ID_BYTES))
                {
                    return Err(std::io::Error::other(
                        "the list of devices is not one this hub knows",
                    ));
                }
                match file.hub {
                    Some(hub) => (hub, file.devices, false),
                    None => (new_id()?, file.devices, true),
                }
            }
        };
        let store = Self {
            path: path.to_owned(),
            hub,
            clock,
            devices: Mutex::new(devices),
            challenges: Mutex::new(Vec::new()),
        };
        // A list of an older hub keeps its id from now on: written at once, or the hub does not start (a new id at
        // every start would make every device pair again).
        if write {
            store.persist(&lock(&store.devices))?;
        }
        Ok(store)
    }

    /// The id of this hub: random, made once, kept with the list. Pairing and every challenge name it, so a device
    /// can tell that it speaks to the hub it paired with before it signs anything.
    #[must_use]
    pub fn hub_id(&self) -> &str {
        &self.hub
    }

    /// Write the list: to a file beside it first, then in place, so a crash leaves the old list or the new one.
    fn persist(&self, devices: &[Stored]) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&File {
            schema: 1,
            hub: Some(self.hub.clone()),
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
    pub fn pair(
        &self,
        name: &str,
        key: &str,
        confirm_key: Option<&str>,
        level: Level,
    ) -> Result<String, PairError> {
        check_pairing(name, key, confirm_key)?;
        let name = clean_name(name).ok_or(PairError::BadName)?;
        let id = Secret::generate(ID_BYTES)
            .map_err(|_| PairError::Random)?
            .to_hex();
        let mut devices = lock(&self.devices);
        let known = |k: &str| {
            devices
                .iter()
                .any(|d| d.key == k || d.confirm_key.as_deref() == Some(k))
        };
        if known(key) || confirm_key.is_some_and(known) {
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
            // Trigger is never a default: the PC grants it to one device at a time.
            level: level.min(Level::Change),
            confirm_key: confirm_key.map(str::to_owned),
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
        if !is_hex(signature, 64) || !self.spend(device, challenge) {
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

    /// Spend a challenge: true when it was open, for this device, and is not too old. Gone either way.
    fn spend(&self, device: &str, challenge: &str) -> bool {
        if !is_hex(challenge, CHALLENGE_BYTES) {
            return false;
        }
        let now = self.clock.now();
        let mut open = lock(&self.challenges);
        match open.iter().position(|c| c.value == challenge) {
            Some(i) => {
                let c = open.remove(i);
                c.device == device && c.until > now
            }
            None => false,
        }
    }

    /// The level of a device; `None` when it is not paired (any more).
    #[must_use]
    pub fn level_of(&self, device: &str) -> Option<Level> {
        lock(&self.devices)
            .iter()
            .find(|d| d.id == device)
            .map(|d| d.level)
    }

    /// Set the level of a device (the PC does). `Ok(false)`: no such device.
    ///
    /// # Errors
    ///
    /// The list could not be written; the level is as it was then.
    pub fn set_level(&self, device: &str, level: Level) -> std::io::Result<bool> {
        let mut devices = lock(&self.devices);
        let Some(at) = devices.iter().position(|d| d.id == device) else {
            return Ok(false);
        };
        let mut next = devices.clone();
        next[at].level = level;
        self.persist(&next)?;
        *devices = next;
        Ok(true)
    }

    /// Does `signature` (P1363 hex) of the device's confirmation key sign [`confirm_message`] of this request with
    /// an open challenge of the device? The challenge is spent either way. A device without a confirmation key never
    /// confirms.
    #[must_use]
    pub fn confirm(
        &self,
        device: &str,
        challenge: &str,
        signature: &str,
        message_of: impl FnOnce(&str) -> String,
    ) -> bool {
        if !is_hex(signature, 64) || !self.spend(device, challenge) {
            return false;
        }
        let devices = lock(&self.devices);
        let Some(key) = devices
            .iter()
            .find(|d| d.id == device)
            .and_then(|d| d.confirm_key.as_deref())
            .and_then(parse_key)
        else {
            return false;
        };
        let Some(sig) = decode_hex(signature).and_then(|b| Signature::from_slice(&b).ok()) else {
            return false;
        };
        key.verify(message_of(challenge).as_bytes(), &sig.normalize_s())
            .is_ok()
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
                level: d.level,
                confirm: d.confirm_key.is_some(),
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
        let id = s.pair("Fairphone", &public, None, Level::Create).unwrap();
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
        let id = s.pair("A", &public, None, Level::Create).unwrap();
        let id2 = s.pair("B", &other_public, None, Level::Create).unwrap();
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
        let id = s.pair("A", &public, None, Level::Create).unwrap();
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
        let id = s.pair("Fairphone", &public, None, Level::Create).unwrap();
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
    fn the_hub_keeps_one_id_across_restarts_and_an_older_list_gets_one_at_once() {
        let (s, clock, d) = store("hubid");
        let path = d.join("devices.json");
        assert!(is_hex(s.hub_id(), ID_BYTES));
        s.pair("A", &key(7).1, None, Level::Create).unwrap();
        let again = DeviceStore::open(&path, clock.clone()).unwrap();
        assert_eq!(again.hub_id(), s.hub_id());
        // A list of a hub before the id: it gets one, written at once, the same at the next start.
        std::fs::write(&path, r#"{"schema":1,"devices":[]}"#).unwrap();
        let old = DeviceStore::open(&path, clock.clone()).unwrap();
        assert!(is_hex(old.hub_id(), ID_BYTES));
        assert_ne!(old.hub_id(), s.hub_id());
        assert_eq!(
            DeviceStore::open(&path, clock).unwrap().hub_id(),
            old.hub_id()
        );
    }

    #[test]
    fn pairing_refuses_bad_names_bad_keys_a_known_key_and_a_full_list() {
        let (s, _, _d) = store("pair");
        let (_, public) = key(7);
        for bad in ["", "   ", "a\nb", "x\u{202E}y", &"n".repeat(MAX_NAME + 1)] {
            assert_eq!(
                s.pair(bad, &public, None, Level::Create),
                Err(PairError::BadName),
                "{bad:?}"
            );
        }
        for bad in [
            "",
            "04",
            &"ab".repeat(65),
            &public.to_uppercase(),
            &public[..128],
        ] {
            assert_eq!(
                s.pair("A", bad, None, Level::Create),
                Err(PairError::BadKey),
                "{bad:?}"
            );
        }
        s.pair("A", &public, None, Level::Create).unwrap();
        assert_eq!(
            s.pair("B", &public, None, Level::Create),
            Err(PairError::Known)
        );
        for seed in 10..(10 + MAX_DEVICES as u8 - 1) {
            s.pair("x", &key(seed).1, None, Level::Create).unwrap();
        }
        assert_eq!(
            s.pair("y", &key(99).1, None, Level::Create),
            Err(PairError::Full)
        );
    }

    #[test]
    fn a_file_that_is_not_a_list_of_devices_stops_the_start() {
        let (_, clock, d) = store("broken");
        let path = d.join("devices.json");
        for bad in [
            "not json",
            r#"{"schema":2,"devices":[]}"#,
            r#"{"schema":1,"devices":[{"id":"x","name":"a","key":"04","paired":0,"last_used":null}]}"#,
            r#"{"schema":1,"hub":"not hex","devices":[]}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            assert!(DeviceStore::open(&path, clock.clone()).is_err(), "{bad}");
        }
    }
}
