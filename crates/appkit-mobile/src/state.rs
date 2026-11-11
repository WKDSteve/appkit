//! What the phone keeps about its pairing: the way and address of the hub, the app, the device id and the hub's id.
//! Nothing secret (the key is the platform's, the session lives in memory), but read back as untrusted (`MSEC-13`):
//! a file of another version, a broken one or one larger than allowed counts as no pairing, and is removed. Written
//! atomically (a file beside it, then renamed). Forgetting removes it and checks that it is gone (`MSEC-12`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::way::HubAddress;

/// The version of the file.
pub const SCHEMA: u32 = 1;
/// The most the file may be when it is read.
const MAX_BYTES: u64 = 16 * 1024;

/// A pairing as the phone keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paired {
    pub address: HubAddress,
    /// The namespace of the app (`testing`).
    pub app: String,
    /// The device id the hub gave (16 bytes of hex).
    pub device: String,
    /// The id of the hub (16 bytes of hex), checked at every login.
    pub hub: String,
    /// The name the person gave this phone.
    pub name: String,
}

impl Paired {
    fn valid(self) -> Option<Self> {
        let address = self.address.clone().checked()?;
        (crate::is_hex(&self.device, 16)
            && crate::is_hex(&self.hub, 16)
            && !self.app.is_empty()
            && self.app.len() <= 32
            && self.name.chars().count() <= 40
            && !self.name.chars().any(char::is_control))
        .then_some(Self { address, ..self })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    paired: Paired,
}

/// The file of the pairing, in the app's private data folder (excluded from backups by the app's manifest, `AND-04`).
#[derive(Debug, Clone)]
pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The kept pairing; `None` when there is none or it cannot be taken (then the file is removed, and the app pairs
    /// again).
    ///
    /// # Errors
    ///
    /// The file is there but cannot be read or removed (the folder is not the app's): the app says so rather than
    /// pairing again on top of it.
    pub fn load(&self) -> std::io::Result<Option<Paired>> {
        let meta = match std::fs::metadata(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(m) => m,
        };
        let taken = (meta.is_file() && meta.len() <= MAX_BYTES)
            .then(|| std::fs::read(&self.path).ok())
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
            .filter(|f| f.schema == SCHEMA)
            .and_then(|f| f.paired.valid());
        if taken.is_none() {
            self.forget()?;
        }
        Ok(taken)
    }

    /// Keep a pairing, in place of the one before.
    ///
    /// # Errors
    ///
    /// The folder cannot be written.
    pub fn save(&self, paired: &Paired) -> std::io::Result<()> {
        let text = serde_json::to_vec_pretty(&File {
            schema: SCHEMA,
            paired: paired.clone(),
        })
        .map_err(std::io::Error::other)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Forget the pairing: the file (and a half written one beside it) is removed, and that it is gone is checked.
    ///
    /// # Errors
    ///
    /// It could not be removed, or is still there.
    pub fn forget(&self) -> std::io::Result<()> {
        for path in [self.path.clone(), self.path.with_extension("tmp")] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            if path.exists() {
                return Err(std::io::Error::other("the pairing is still there"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    fn paired() -> Paired {
        Paired {
            address: HubAddress::tailnet("pc.example.ts.net").unwrap(),
            app: "testing".to_owned(),
            device: "ab".repeat(16),
            hub: "cd".repeat(16),
            name: "Fairphone".to_owned(),
        }
    }

    #[test]
    fn a_pairing_is_kept_read_back_and_forgotten_wholly() {
        let dir = Scratch::new("mobile-state");
        let file = StateFile::new(dir.join("pairing.json"));
        assert_eq!(file.load().unwrap(), None);
        file.save(&paired()).unwrap();
        assert_eq!(file.load().unwrap(), Some(paired()));
        file.forget().unwrap();
        assert!(!file.path().exists());
        assert_eq!(file.load().unwrap(), None);
    }

    #[test]
    fn a_file_that_is_not_a_pairing_counts_as_none_and_is_removed() {
        let dir = Scratch::new("mobile-state-bad");
        let file = StateFile::new(dir.join("pairing.json"));
        let good = serde_json::to_string(&File {
            schema: SCHEMA,
            paired: paired(),
        })
        .unwrap();
        for bad in [
            "not json".to_owned(),
            good.replace("\"schema\":1", "\"schema\":2"),
            good.replace("https://pc.example.ts.net", "http://192.168.1.2:80"),
            good.replace(&"ab".repeat(16), "zz"),
            good.replace("\"name\":\"Fairphone\"", "\"name\":\"a\\nb\""),
            good.replace('}', ",\"extra\":1}"),
            format!("{good}{}", " ".repeat(20_000)),
            // Half written.
            good[..good.len() / 2].to_owned(),
        ] {
            std::fs::write(file.path(), &bad).unwrap();
            assert_eq!(file.load().unwrap(), None, "{bad}");
            assert!(!file.path().exists(), "{bad}");
        }
    }
}
