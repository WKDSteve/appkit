//! The two small files a running server leaves in its config folder.
//!
//! * `hub.json`: who runs where (port, process, version, instance id). It
//!   holds **no secret**: any program may read it to find the server.
//! * `control.secret`: the credential that lets a local program ask the
//!   running server for a pairing code. It is a file with the rights of the
//!   user (mode 0600 where the system has modes; on Windows the folder's
//!   inherited ACL is what protects it, see `docs/SECURITY.md`).

use std::fs;
use std::io::{self, Read, Write};
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
    // Windows: the file is made (empty) and given to the user before the
    // secret is written into it, so no moment exists in which the inherited
    // rights guard the secret.
    #[cfg(windows)]
    {
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        restrict_to_user(&path)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `mode` only applies to a file that is created; fix an old one.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(secret_hex.as_bytes())?;
    file.write_all(b"\n")
}

/// Windows: take every inherited right off the file and give it to the
/// current user alone. The folder's inherited list is not a safe default:
/// it can name other groups (measured on a development machine: a sandbox
/// group with the right to change the file). Without the right to do this
/// the hub does not start, because a secret that others can read is no secret.
#[cfg(windows)]
fn restrict_to_user(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};

    /// `CREATE_NO_WINDOW`: a hub that is a window program has no console, and a console program it
    /// starts would get one of its own, with a window.
    const NO_WINDOW: u32 = 0x0800_0000;
    let root =
        std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("SystemRoot is not set"))?;
    let icacls = PathBuf::from(root).join("System32").join("icacls.exe");
    let user = std::env::var("USERNAME").map_err(|_| io::Error::other("USERNAME is not set"))?;
    let principal = match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    };
    let status = Command::new(icacls)
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{principal}:F"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(NO_WINDOW)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "icacls could not restrict the control secret (exit {:?}); \
             set the config-folder variable of this app (<PREFIX>_CONFIG_DIR) to a folder that only you own",
            status.code()
        )))
    }
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
        let out = std::process::Command::new("icacls")
            .arg(secret_path(&d))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // `(I)` marks an inherited entry; there must be none, and the one
        // entry left is the current user's.
        assert!(!text.contains("(I)"), "{text}");
        let user = std::env::var("USERNAME").unwrap().to_lowercase();
        assert!(text.to_lowercase().contains(&user), "{text}");
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
