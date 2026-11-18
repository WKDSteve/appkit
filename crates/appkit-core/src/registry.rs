//! The two small files a running server leaves in its config folder.
//!
//! * `hub.json`: who runs where (port, process, version, instance id). It
//!   holds **no secret**: any program may read it to find the server.
//! * `control.secret`: the credential that lets a local program ask the
//!   running server for a pairing code. It is a file with the rights of the
//!   user (mode 0600 where the system has modes; on Windows the folder's
//!   inherited ACL is what protects it, see `docs/SECURITY.md`).

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What `hub.json` says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubRecord {
    /// Always the app name; a different value is somebody else's file.
    pub app: String,
    pub port: u16,
    pub pid: u32,
    pub version: String,
    /// Changes with every start; the cache key of a client.
    pub instance: String,
}

/// The longest file of either kind that is read.
const MAX_FILE_BYTES: u64 = 4 * 1024;

#[must_use]
pub fn record_path(dir: &Path) -> PathBuf {
    dir.join("hub.json")
}

#[must_use]
pub fn secret_path(dir: &Path) -> PathBuf {
    dir.join("control.secret")
}

fn read_limited(path: &Path) -> io::Result<Option<String>> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the file is larger than expected",
        ));
    }
    Ok(Some(text))
}

/// Write `hub.json` (through a temporary file, so a reader never sees half).
///
/// # Errors
///
/// Any I/O error of creating the folder, writing or renaming.
pub fn write_record(dir: &Path, record: &HubRecord) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
    let path = record_path(dir);
    let tmp = dir.join("hub.json.tmp");
    fs::write(&tmp, format!("{text}\n"))?;
    fs::rename(&tmp, &path)
}

/// Read `hub.json`. A missing file is `None`; a file that is not a record
/// of this app is an error.
///
/// # Errors
///
/// I/O errors, a file over the size limit, or content that is not a record.
pub fn read_record(dir: &Path) -> io::Result<Option<HubRecord>> {
    let Some(text) = read_limited(&record_path(dir))? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Remove `hub.json` (and ignore that it is already gone).
pub fn remove_record(dir: &Path) {
    let _ = fs::remove_file(record_path(dir));
}

/// Write the control secret, readable by the user only where the system
/// has modes.
///
/// # Errors
///
/// Any I/O error of creating the folder or writing the file.
pub fn write_secret(dir: &Path, secret_hex: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = secret_path(dir);
    // The secret only ever goes into a file made new for it, with its rights from the first moment. An old file goes
    // first (one a crash left, or one somebody planted or holds open): a program that opened it before keeps its
    // handle when the rights change, and would read the secret through it.
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(io::Error::new(
                e.kind(),
                format!("cannot replace the old control secret: {e}"),
            ));
        }
    }
    let content = format!("{secret_hex}\n");
    create_for_user(&path, content.as_bytes())
}

/// Windows: a new file whose only right is the current user's, from its first moment (the folder's inherited list is
/// not a safe default: it can name other groups, measured on a development machine: a sandbox group with the right
/// to change the file). Without the right to do this the hub does not start, because a secret that others can read
/// is no secret. Through the security API of the system (`crate::acl`), never `icacls.exe`.
#[cfg(windows)]
fn create_for_user(path: &Path, content: &[u8]) -> io::Result<()> {
    crate::acl::create_user_only(path, content).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "the control secret could not be written for you alone ({e}); \
                 set the config-folder variable of this app (<PREFIX>_CONFIG_DIR) to a folder that only you own"
            ),
        )
    })
}

/// Elsewhere: a new file with mode 0600. `create_new` is `O_CREAT | O_EXCL`, which never follows a link that was put
/// in the file's place meanwhile.
#[cfg(not(windows))]
fn create_for_user(path: &Path, content: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let written = file.write_all(content);
    drop(file);
    if written.is_err() {
        let _ = fs::remove_file(path);
    }
    written
}

/// Read the control secret. A missing file is `None`.
///
/// # Errors
///
/// I/O errors, or a file over the size limit.
pub fn read_secret(dir: &Path) -> io::Result<Option<String>> {
    Ok(read_limited(&secret_path(dir))?.map(|t| t.trim().to_owned()))
}

/// Remove the control secret (and ignore that it is already gone).
pub fn remove_secret(dir: &Path) {
    let _ = fs::remove_file(secret_path(dir));
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    fn dir(name: &str) -> Scratch {
        Scratch::new(&format!("reg-{name}"))
    }

    fn record() -> HubRecord {
        HubRecord {
            app: "tasks-desktop".to_owned(),
            port: 17615,
            pid: 4242,
            version: "0.0.0".to_owned(),
            instance: "ab".repeat(16),
        }
    }

    #[test]
    fn a_record_round_trips_and_holds_no_secret() {
        let d = dir("record");
        write_record(&d, &record()).unwrap();
        assert_eq!(read_record(&d).unwrap(), Some(record()));
        let text = fs::read_to_string(record_path(&d)).unwrap();
        assert!(!text.to_lowercase().contains("token") && !text.to_lowercase().contains("secret"));
        remove_record(&d);
        assert_eq!(read_record(&d).unwrap(), None);
    }

    #[test]
    fn a_foreign_or_oversized_file_is_an_error() {
        let d = dir("foreign");
        fs::create_dir_all(&d).unwrap();
        fs::write(record_path(&d), r#"{"app":"x","port":1,"unknown":true}"#).unwrap();
        assert!(read_record(&d).is_err());
        fs::write(record_path(&d), vec![b' '; 8 * 1024]).unwrap();
        assert!(read_record(&d).is_err());
    }

    #[test]
    fn the_secret_file_round_trips() {
        let d = dir("secret");
        assert_eq!(read_secret(&d).unwrap(), None);
        write_secret(&d, "abc123").unwrap();
        assert_eq!(read_secret(&d).unwrap().as_deref(), Some("abc123"));
        write_secret(&d, "x").unwrap();
        assert_eq!(read_secret(&d).unwrap().as_deref(), Some("x"));
        remove_secret(&d);
        assert_eq!(read_secret(&d).unwrap(), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_secret_file_has_no_inherited_rights() {
        let d = dir("acl");
        write_secret(&d, "abc").unwrap();
        // Protected (nothing inherited), one entry, and that one gives the current user full control.
        assert_eq!(
            crate::acl::tests::rights_of(&secret_path(&d)),
            (true, 1, true)
        );
    }

    /// A program that opened the old file before (the folder's inherited rights may let it) keeps its handle when the
    /// rights change: the secret must go into a file made new for it, never into that old one.
    #[test]
    fn a_handle_on_the_old_file_does_not_see_the_secret() {
        use std::io::Read as _;
        let d = dir("old-handle");
        fs::create_dir_all(&d).unwrap();
        fs::write(secret_path(&d), "old\n").unwrap();
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE: as open as another program could hold it.
            options.share_mode(7);
        }
        let mut held = options.open(secret_path(&d)).unwrap();
        write_secret(&d, "fresh-secret").unwrap();
        let mut seen = String::new();
        held.read_to_string(&mut seen).unwrap();
        assert!(
            !seen.contains("fresh-secret"),
            "the old handle reads {seen:?}"
        );
        drop(held);
        assert_eq!(read_secret(&d).unwrap().as_deref(), Some("fresh-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_the_secret_is_not_followed() {
        let d = dir("link");
        fs::create_dir_all(&d).unwrap();
        let elsewhere = d.join("elsewhere.txt");
        fs::write(&elsewhere, "theirs\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, secret_path(&d)).unwrap();
        write_secret(&d, "fresh-secret").unwrap();
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "theirs\n");
        assert_eq!(read_secret(&d).unwrap().as_deref(), Some("fresh-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn the_secret_file_is_for_the_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("mode");
        write_secret(&d, "abc").unwrap();
        let mode = fs::metadata(secret_path(&d)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
