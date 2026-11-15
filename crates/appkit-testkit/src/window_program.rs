//! Whether a program of Windows is a window program or a console program, read from its header.
//!
//! A hub's release build is a window program (`windows_subsystem = "windows"`), so Windows opens no
//! console window when it is started by a link, by the login entry or by a double click (see
//! `appkit_core::console`). An app proves it with a test of its own that reads its program:
//!
//! ```no_run
//! use appkit_testkit::window_program::{subsystem, CONSOLE, WINDOW};
//!
//! // In the app's test: `env!("CARGO_BIN_EXE_tasks-hub")`.
//! let program = std::path::Path::new("target/release/tasks-hub.exe");
//! let exe = std::fs::read(program).unwrap();
//! let want = if cfg!(debug_assertions) { CONSOLE } else { WINDOW };
//! assert_eq!(subsystem(&exe), Some(want));
//! ```
//!
//! Such a test proves the attribute only in a release test run (`cargo test --release`); a debug build
//! keeps its console on purpose.

/// `IMAGE_SUBSYSTEM_WINDOWS_GUI`: a window program, started without a console.
pub const WINDOW: u16 = 2;
/// `IMAGE_SUBSYSTEM_WINDOWS_CUI`: a console program; Windows gives it a console, with a window.
pub const CONSOLE: u16 = 3;

/// The subsystem field of a PE file: the optional header starts 24 bytes after the signature `PE\0\0`,
/// and the field is at offset 68 of it (the same in PE32 and PE32+). `None` when the bytes are no
/// program of Windows.
#[must_use]
pub fn subsystem(exe: &[u8]) -> Option<u16> {
    if exe.get(..2)? != b"MZ" {
        return None;
    }
    let at: [u8; 4] = exe.get(0x3c..0x40)?.try_into().ok()?;
    let pe = usize::try_from(u32::from_le_bytes(at)).ok()?;
    if exe.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let field = pe.checked_add(24 + 68)?;
    let bytes: [u8; 2] = exe.get(field..field.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_that_are_no_program_have_no_subsystem() {
        assert_eq!(subsystem(b""), None);
        assert_eq!(subsystem(b"MZ"), None);
        let mut exe = vec![0u8; 0x100];
        exe[..2].copy_from_slice(b"MZ");
        exe[0x3c] = 0x80;
        assert_eq!(subsystem(&exe), None, "no PE signature");
        exe[0x80..0x84].copy_from_slice(b"PE\0\0");
        exe[0x80 + 92] = 2;
        assert_eq!(subsystem(&exe), Some(WINDOW));
        exe[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(subsystem(&exe), None, "an offset past the end");
    }
}
