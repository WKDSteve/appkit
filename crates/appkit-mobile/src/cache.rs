//! The kept copy of what the hub answered, for the start and for a phone without a way to the hub (`MOB-08`).
//!
//! It holds exactly the answers the hub gave this device for routes of the app, so only data the hub released for
//! devices (`MOB-02`: the hub cuts, the cache never has to). Each entry is the whole answer of one route and query, as
//! the hub sent it: a part is never made into a whole. Entries older than the limit are dropped when read; the file is
//! versioned, bounded and written atomically (`MSEC-13`); forgetting removes it and checks that it is gone
//! (`MSEC-12`). The app's data folder is excluded from backups (`AND-04`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const SCHEMA: u32 = 1;

/// How much the cache holds.
#[derive(Debug, Clone, Copy)]
pub struct CacheLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
    /// An entry older than this is not shown any more.
    pub max_age: Duration,
}

impl Default for CacheLimits {
    fn default() -> Self {
        Self {
            max_entries: 64,
            max_bytes: 8 * 1024 * 1024,
            max_age: Duration::from_secs(14 * 24 * 3600),
        }
    }
}

/// One kept answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub etag: Option<String>,
    /// When the hub sent it, seconds since 1970.
    pub fetched_at: u64,
    pub body: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    entries: BTreeMap<String, Entry>,
}

/// The cache of one pairing.
pub struct Cache {
    path: PathBuf,
    limits: CacheLimits,
    entries: Mutex<BTreeMap<String, Entry>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Cache {
    /// The cache kept in `path`; a file that is not one (another version, broken, too large) is an empty cache.
    #[must_use]
    pub fn open(path: impl Into<PathBuf>, limits: CacheLimits, now: u64) -> Self {
        let path = path.into();
        let entries = std::fs::metadata(&path)
            .ok()
            // The bodies as JSON strings, escaped at most twice over, and the frame around them.
            .filter(|m| m.is_file() && m.len() <= limits.max_bytes as u64 * 2 + 64 * 1024)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
            .filter(|f| f.schema == SCHEMA && f.entries.len() <= limits.max_entries)
            .map(|f| f.entries)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, e)| now.saturating_sub(e.fetched_at) <= limits.max_age.as_secs())
            .collect();
        Self {
            path,
            limits,
            entries: Mutex::new(entries),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The kept answer of a route (with its query).
    #[must_use]
    pub fn get(&self, route: &str) -> Option<Entry> {
        lock(&self.entries).get(route).cloned()
    }

    /// Keep an answer of the hub, in place of the one before. Over the limits the oldest entries go; an answer larger
    /// than the whole cache is not kept.
    ///
    /// # Errors
    ///
    /// The file cannot be written (the copy in memory is kept all the same).
    pub fn put(&self, route: &str, entry: Entry) -> std::io::Result<()> {
        if entry.body.len() > self.limits.max_bytes {
            return Ok(());
        }
        let snapshot = {
            let mut entries = lock(&self.entries);
            entries.insert(route.to_owned(), entry);
            while entries.len() > self.limits.max_entries
                || entries.values().map(|e| e.body.len()).sum::<usize>() > self.limits.max_bytes
            {
                let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, e)| e.fetched_at)
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                entries.remove(&oldest);
            }
            entries.clone()
        };
        self.write(snapshot)
    }

    fn write(&self, entries: BTreeMap<String, Entry>) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(&File {
            schema: SCHEMA,
            entries,
        })
        .map_err(std::io::Error::other)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Forget everything (the phone is unpaired, or the hub said the device is gone): the file is removed and that it
    /// is gone is checked.
    ///
    /// # Errors
    ///
    /// It could not be removed, or is still there.
    pub fn forget(&self) -> std::io::Result<()> {
        lock(&self.entries).clear();
        for path in [self.path.clone(), self.path.with_extension("tmp")] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            if path.exists() {
                return Err(std::io::Error::other("the cache is still there"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    fn entry(at: u64, body: &str) -> Entry {
        Entry {
            etag: Some("t".to_owned()),
            fetched_at: at,
            body: body.to_owned(),
        }
    }

    #[test]
    fn answers_are_kept_bounded_and_old_ones_are_not_shown() {
        let dir = Scratch::new("mobile-cache");
        let limits = CacheLimits {
            max_entries: 2,
            max_bytes: 10,
            max_age: Duration::from_secs(100),
        };
        let c = Cache::open(dir.join("cache.json"), limits, 1000);
        c.put("/a", entry(1000, "aaaa")).unwrap();
        c.put("/b", entry(1001, "bbbb")).unwrap();
        c.put("/c", entry(1002, "cccc")).unwrap();
        assert_eq!(c.get("/a"), None, "the oldest went");
        c.put("/huge", entry(1003, &"x".repeat(11))).unwrap();
        assert_eq!(c.get("/huge"), None);
        let again = Cache::open(dir.join("cache.json"), limits, 1050);
        assert_eq!(again.get("/c"), Some(entry(1002, "cccc")));
        let later = Cache::open(dir.join("cache.json"), limits, 1102);
        assert_eq!(later.get("/b"), None, "too old");
        assert_eq!(later.get("/c"), Some(entry(1002, "cccc")));
        later.forget().unwrap();
        assert!(!dir.join("cache.json").exists());
    }

    #[test]
    fn a_file_that_is_not_a_cache_is_an_empty_one() {
        let dir = Scratch::new("mobile-cache-bad");
        let path = dir.join("cache.json");
        for bad in [
            "x",
            r#"{"schema":2,"entries":{}}"#,
            r#"{"schema":1,"entries":{},"more":1}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            assert_eq!(
                Cache::open(&path, CacheLimits::default(), 0).get("/a"),
                None
            );
        }
    }
}
