//! Windows: the few things the kit asks of the registry of the current user and the machine (a value of a key, whether a
//! key is there; a string value written or removed), through the registry API of the system. No `reg.exe`: a program
//! that starts the tools of the system to change the login entries is a pattern an endpoint protection reads as an
//! attack (appkit/edr-ausloeser-aus-allen-nativen-apps-entfernen).
//!
//! Keys are named as `reg.exe` names them (`HKCU\Software\...`, `HKLM\Software\...`). Strings are read and written
//! as UTF-16, so a path with any character comes back as it is (`reg.exe` printed in the code page of the console).
#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ,
};

/// The largest value that is read: a path, or a command line.
const MAX_VALUE_UNITS: usize = 32 * 1024;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The root and the rest of a key named as `reg.exe` names it.
fn split(key: &str) -> Result<(HKEY, Vec<u16>), String> {
    let (root, rest) = key.split_once('\\').unwrap_or((key, ""));
    let root = match root.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        _ => return Err(format!("not a key of the user or the machine: {key}")),
    };
    Ok((root, wide(rest)))
}

fn os_error(code: u32) -> String {
    std::io::Error::from_raw_os_error(code as i32).to_string()
}

/// Whether `key` is there (and may be read).
///
/// # Errors
///
/// The key is not one of the user or the machine.
pub fn key_exists(key: &str) -> Result<bool, String> {
    let (root, sub) = split(key)?;
    let mut opened: HKEY = std::ptr::null_mut();
    // SAFETY: `sub` ends with a NUL; `opened` receives a key we close below.
    let code = unsafe { RegOpenKeyExW(root, sub.as_ptr(), 0, KEY_READ, &mut opened) };
    if code == ERROR_SUCCESS {
        // SAFETY: the key RegOpenKeyExW opened, closed once.
        unsafe { RegCloseKey(opened) };
        return Ok(true);
    }
    Ok(false)
}

/// The string `value` of `key` as it is stored (`%VAR%` not expanded); `None` when the key or the value is not there,
/// is not a string or may not be read.
///
/// # Errors
///
/// The key is not one of the user or the machine.
pub fn get(key: &str, value: &str) -> Result<Option<String>, String> {
    let (root, sub) = split(key)?;
    let name = wide(value);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut bytes = 0u32;
    // SAFETY: a call without a buffer only asks for the size.
    let code = unsafe {
        RegGetValueW(
            root,
            sub.as_ptr(),
            name.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if code != ERROR_SUCCESS || bytes as usize > MAX_VALUE_UNITS * 2 {
        return Ok(None);
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    let mut size =
        u32::try_from(buf.len() * 2).map_err(|_| "a value that is too large".to_owned())?;
    // SAFETY: `buf` holds `size` bytes.
    let code = unsafe {
        RegGetValueW(
            root,
            sub.as_ptr(),
            name.as_ptr(),
            flags,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if code != ERROR_SUCCESS {
        return Ok(None);
    }
    let units = (size as usize / 2).min(buf.len());
    let text = &buf[..units];
    let end = text.iter().position(|&u| u == 0).unwrap_or(text.len());
    Ok(Some(String::from_utf16_lossy(&text[..end])))
}

/// Write the string `value` of `key` (the key is made when it is not there).
///
/// # Errors
///
/// What the system said.
pub fn set(key: &str, value: &str, data: &str) -> Result<(), String> {
    let (root, sub) = split(key)?;
    let name = wide(value);
    let text = wide(data);
    let bytes =
        u32::try_from(text.len() * 2).map_err(|_| "a value that is too large".to_owned())?;
    // SAFETY: all strings end with a NUL; `text` holds `bytes` bytes.
    let code = unsafe {
        RegSetKeyValueW(
            root,
            sub.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            text.as_ptr().cast(),
            bytes,
        )
    };
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(os_error(code))
    }
}

/// Remove `value` of `key`. Whether it was there: a value or key that is not there is not an error.
///
/// # Errors
///
/// What the system said (a value that is there and may not be removed).
pub fn delete(key: &str, value: &str) -> Result<bool, String> {
    let (root, sub) = split(key)?;
    let name = wide(value);
    // SAFETY: both strings end with a NUL.
    let code = unsafe { RegDeleteKeyValueW(root, sub.as_ptr(), name.as_ptr()) };
    match code {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(false),
        other => Err(os_error(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reading only, and only what every Windows has: the tests never write the registry of the machine.
    #[test]
    fn a_key_and_a_value_of_every_windows_are_read_as_they_are() {
        assert!(key_exists(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion").unwrap());
        assert!(!key_exists(r"HKCU\Software\appkit-test-no-such-key-6f1c").unwrap());
        let root = get(
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "SystemRoot",
        )
        .unwrap();
        assert!(root.is_some_and(|r| r.len() > 3));
        assert_eq!(
            get(r"HKCU\Software\appkit-test-no-such-key-6f1c", "x").unwrap(),
            None
        );
        assert!(split(r"HKCR\x").is_err());
        assert_eq!(
            delete(r"HKCU\Software\appkit-test-no-such-key-6f1c", "x"),
            Ok(false)
        );
    }
}
