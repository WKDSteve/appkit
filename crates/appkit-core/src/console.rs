//! The console of a program that is a window program on Windows (`windows_subsystem = "windows"`).
//!
//! A hub started by a link, by the login entry or by a double click should open no console window. So
//! its release build is a window program: Windows gives it no console. Started from a terminal it would
//! then write `status`, `--help` and its errors nowhere, so the first thing `main` does is
//! [`attach_parent`]: the program writes to the console of the terminal it was started from. Three
//! things follow, and the functions here are for them:
//!
//! - **The shell does not wait** for a window program. PowerShell and `cmd` show their prompt at once and
//!   read the same keys as the program. A question on stdin is therefore never asked in that console:
//!   [`open_own`] opens a console window of the program's own for it, [`close_own`] closes it again.
//! - **Without a terminal nobody reads stderr.** [`report`] writes an error to stderr and, when stderr
//!   goes nowhere, shows it in a message box; [`report_panics`] does the same for a panic.
//! - **Streams that go somewhere stay there.** A stream redirected to a file, a pipe or the null device is
//!   not replaced by the console. A program whose three streams all go somewhere (a hub put in the
//!   background, a test, `hub status > out.txt` from a script) does not attach at all: it gains nothing
//!   from the console and would end when the terminal is closed.
//!
//! On macOS and Linux a program has the streams its parent gave it, and the shell waits for it:
//! [`attach_parent`] says [`Console::Own`] and [`report`] writes to stderr only.
#![allow(unsafe_code)]

use std::io::Write as _;
use std::sync::Mutex;

/// Which console the process writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Console {
    /// A console of its own: a console program (a debug build; every program on macOS and Linux), or a
    /// window program that opened one with [`open_own`]. The shell that started it waits for it.
    Own,
    /// The console of the terminal the program was started from. The shell does not wait for a window
    /// program: the prompt is back and the shell reads the same keys.
    Parent,
    /// No console: a double click, a link, the login entry, or all three streams already go somewhere.
    None,
}

/// What this process knows about its console.
#[derive(Debug)]
struct State {
    /// `None` until [`attach_parent`] ran.
    console: Option<Console>,
    /// The three standard handles the parent gave (stdin, stdout, stderr; 0 for none), as they were
    /// before anything here changed them.
    given: [usize; 3],
    /// Which console the process had before [`open_own`] opened one; `None` when it did not.
    before_own: Option<Console>,
}

static STATE: Mutex<State> = Mutex::new(State {
    console: None,
    given: [0; 3],
    before_own: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Attach the process to the console of the terminal it was started from, once; afterwards it only says
/// which console the process writes to. Call it first in `main`, before anything is written.
///
/// Nothing happens when the process has a console already (a console program: [`Console::Own`]), when
/// the parent has none ([`Console::None`]: a double click, a link, the login entry), and when stdin,
/// stdout and stderr all go somewhere already ([`Console::None`]). A stream that goes somewhere (a file,
/// a pipe, the null device) keeps going there; only a stream that went nowhere is bound to the console.
pub fn attach_parent() -> Console {
    let mut state = state();
    if let Some(console) = state.console {
        return console;
    }
    let console = sys::attach(&mut state.given);
    state.console = Some(console);
    console
}

/// Open a console window of the program's own, for questions on stdin: in the console of the terminal
/// the shell reads the same keys, and without a console nobody would see them. Its standard streams are
/// that window until [`close_own`]. A program that has a console of its own already uses it (`true`,
/// nothing opened); `false` when the system refused one.
pub fn open_own(title: &str) -> bool {
    let current = attach_parent();
    if current == Console::Own {
        return true;
    }
    let mut state = state();
    let given = state.given;
    if !sys::open_own(current, title, &given) {
        return false;
    }
    state.before_own = Some(current);
    state.console = Some(Console::Own);
    true
}

/// Whether [`open_own`] opened a window that [`close_own`] closes.
#[must_use]
pub fn opened_own() -> bool {
    state().before_own.is_some()
}

/// Close the window [`open_own`] opened, and write where the program wrote before: to the console of the
/// terminal again (when it is still there) or nowhere. Nothing happens when no window was opened.
pub fn close_own() {
    let mut state = state();
    let Some(before) = state.before_own.take() else {
        return;
    };
    let given = state.given;
    state.console = Some(sys::close_own(before, &given));
}

/// Whether the console of the process is the one of the terminal it was started from (whose shell reads
/// the same keys). Questions go to a window of their own then ([`open_own`]).
#[must_use]
pub fn shares_terminal() -> bool {
    attach_parent() == Console::Parent
}

/// Whether the parent gave the process neither stdin nor stdout: a double click, a link or the login
/// entry on Windows. Never on other systems.
#[must_use]
pub fn started_without_streams() -> bool {
    let _ = attach_parent();
    let given = state().given;
    sys::without_streams(&given)
}

/// Whether what is written to stderr reaches anybody: a console, a file or a pipe.
#[must_use]
pub fn stderr_is_read() -> bool {
    sys::stderr_is_read()
}

/// Tell the person an error: `<title>: <message>` on stderr, and in a message box when stderr goes
/// nowhere (a window program without a console). The box waits for the person, so this is for the end
/// of a command, not for a hub that keeps running.
pub fn report(title: &str, message: &str) {
    let _ = writeln!(std::io::stderr(), "{title}: {message}");
    if !stderr_is_read() {
        sys::message_box(title, message);
    }
}

/// Show a panic in a message box when stderr goes nowhere, after the usual text of the panic (which goes
/// to stderr: the terminal, or `hub.log` for a hub in the background). Call it in `main` after
/// [`attach_parent`].
pub fn report_panics(title: &str) {
    let title = title.to_owned();
    let usual = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        usual(info);
        if !stderr_is_read() {
            sys::message_box(&title, &format!("The program stopped: {info}"));
        }
    }));
}

#[cfg(windows)]
mod sys {
    use super::Console;
    use std::os::windows::io::IntoRawHandle as _;
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_ACCESS_DENIED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{
        AllocConsole, AttachConsole, FreeConsole, GetStdHandle, SetConsoleTitleW, SetStdHandle,
        ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };

    const STREAMS: [STD_HANDLE; 3] = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];

    fn std_handle(which: STD_HANDLE) -> HANDLE {
        // SAFETY: GetStdHandle takes no pointer and only reads the handle table of the process.
        unsafe { GetStdHandle(which) }
    }

    fn set_std_handle(which: STD_HANDLE, handle: HANDLE) {
        // SAFETY: SetStdHandle takes no pointer; the handle is one of this process (or null), and the
        // standard library reads the standard handles anew on every use, so nothing holds the old one.
        let _ = unsafe { SetStdHandle(which, handle) };
    }

    /// A handle that leads somewhere: not null and not `INVALID_HANDLE_VALUE`.
    pub(super) fn leads_somewhere(handle: usize) -> bool {
        handle != 0 && handle != INVALID_HANDLE_VALUE as usize
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `CONIN$` or `CONOUT$` of the console the process is attached to, opened anew. The handle is the
    /// process's standard handle from now on and is never closed.
    fn console_stream(input: bool) -> HANDLE {
        let name = if input { "CONIN$" } else { "CONOUT$" };
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
            .map_or(std::ptr::null_mut(), |file| file.into_raw_handle())
    }

    /// Bind the standard streams to the console the process is attached to now, except a stream the
    /// parent gave (a file, a pipe, the null device): that one keeps its handle. Opened explicitly,
    /// instead of trusting which handles the system fills in after `AttachConsole`.
    fn bind(given: &[usize; 3]) {
        // Opened only for a stream that needs it: a handle opened and not used would stay open.
        let mut output = None;
        for (i, which) in STREAMS.into_iter().enumerate() {
            let handle = if leads_somewhere(given[i]) {
                given[i] as HANDLE
            } else if i == 0 {
                console_stream(true)
            } else {
                *output.get_or_insert_with(|| console_stream(false))
            };
            set_std_handle(which, handle);
        }
    }

    /// Whether the parent gave neither stdin nor stdout.
    pub(super) fn without_streams(given: &[usize; 3]) -> bool {
        !leads_somewhere(given[0]) && !leads_somewhere(given[1])
    }

    pub(super) fn attach(given: &mut [usize; 3]) -> Console {
        for (i, which) in STREAMS.into_iter().enumerate() {
            given[i] = std_handle(which) as usize;
        }
        // All three streams go somewhere: the console would only add the end of the program when the
        // terminal is closed (the hub `start` puts in the background must outlive the terminal).
        if given.iter().all(|&h| leads_somewhere(h)) {
            // A console program has a console all the same.
            return if has_console() {
                Console::Own
            } else {
                Console::None
            };
        }
        // SAFETY: AttachConsole takes no pointer and only changes which console the process writes to.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0 {
            bind(given);
            return Console::Parent;
        }
        // SAFETY: GetLastError only reads the last error of this thread, set by AttachConsole just now.
        if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
            Console::Own // a console program: it has one already
        } else {
            Console::None // the parent has none
        }
    }

    /// Whether the process is attached to a console now (also one without a window).
    fn has_console() -> bool {
        std::fs::OpenOptions::new()
            .write(true)
            .open("CONOUT$")
            .is_ok()
    }

    pub(super) fn open_own(current: Console, title: &str, given: &[usize; 3]) -> bool {
        if current == Console::Parent {
            // SAFETY: FreeConsole takes no argument; it detaches the process from the terminal's console,
            // which the terminal keeps.
            let _ = unsafe { FreeConsole() };
        }
        // SAFETY: AllocConsole takes no argument; it fails, harmlessly, when the process has a console.
        if unsafe { AllocConsole() } == 0 {
            if current == Console::Parent && reattach() {
                bind(given);
            }
            return false;
        }
        let title = wide(title);
        // SAFETY: the title is a NUL-terminated UTF-16 string that lives across the call.
        let _ = unsafe { SetConsoleTitleW(title.as_ptr()) };
        // The questions are asked and answered in the window, whatever the parent gave.
        bind(&[0; 3]);
        true
    }

    /// Attach to the terminal's console again (after the own window, or when none could be opened).
    fn reattach() -> bool {
        // SAFETY: as in `attach`.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
    }

    pub(super) fn close_own(before: Console, given: &[usize; 3]) -> Console {
        // SAFETY: FreeConsole takes no argument; the window closes with the last process attached to it.
        let _ = unsafe { FreeConsole() };
        if before == Console::Parent && reattach() {
            bind(given);
            return Console::Parent;
        }
        // Nowhere again, or to what the parent gave: never to a handle of the closed window.
        for (i, which) in STREAMS.into_iter().enumerate() {
            set_std_handle(which, given[i] as HANDLE);
        }
        Console::None
    }

    pub(super) fn stderr_is_read() -> bool {
        leads_somewhere(std_handle(STD_ERROR_HANDLE) as usize)
    }

    pub(super) fn message_box(title: &str, message: &str) {
        let (title, message) = (wide(title), wide(message));
        // SAFETY: both strings are NUL-terminated UTF-16 and live across the call; no owner window.
        let _ = unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            )
        };
    }
}

#[cfg(not(windows))]
mod sys {
    use super::Console;

    pub(super) fn attach(_given: &mut [usize; 3]) -> Console {
        Console::Own
    }

    /// A program here always has the streams its parent gave it.
    pub(super) fn without_streams(_given: &[usize; 3]) -> bool {
        false
    }

    pub(super) fn open_own(_current: Console, _title: &str, _given: &[usize; 3]) -> bool {
        true
    }

    pub(super) fn close_own(before: Console, _given: &[usize; 3]) -> Console {
        before
    }

    pub(super) fn stderr_is_read() -> bool {
        true
    }

    pub(super) fn message_box(_title: &str, _message: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attaching_twice_gives_the_same_answer_and_closing_without_a_window_does_nothing() {
        let first = attach_parent();
        assert_eq!(attach_parent(), first);
        assert!(!opened_own());
        close_own();
        assert_eq!(attach_parent(), first);
    }

    /// A test run gives its tests streams that go somewhere (a pipe of cargo, or the console of the
    /// terminal), so stderr is read and no message box would block the run.
    #[test]
    fn the_streams_of_a_test_run_go_somewhere() {
        assert!(stderr_is_read());
        assert!(!started_without_streams());
    }

    #[cfg(windows)]
    #[test]
    fn a_handle_leads_somewhere_unless_it_is_null_or_invalid() {
        assert!(!sys::leads_somewhere(0));
        assert!(!sys::leads_somewhere(usize::MAX));
        assert!(sys::leads_somewhere(0x40));
    }
}
