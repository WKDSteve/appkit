//! The folder dialog of Windows, shown by this process (`IFileOpenDialog` with `FOS_PICKFOLDERS`).
//!
//! No program is started for it: the dialog is an object of the system that lives in this
//! process. It wants a thread with a single-threaded apartment and a message loop of its own,
//! which `Show` runs; so every call makes a thread, shows the dialog on it and ends the thread.
//!
//! `windows-sys` has the functions and the constants but not the interfaces, so the few methods
//! used here are called by their place in the table of the interface (`ShObjIdl.h`; the order of
//! a published interface never changes).
#![allow(unsafe_code)]

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{ERROR_CANCELLED, HWND, S_FALSE, S_OK};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::UI::Shell::{
    FileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

/// `IFileOpenDialog`.
const IID_FILE_OPEN_DIALOG: GUID = GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);

// The places of the methods in the tables. `IUnknown` has three, `IModalWindow` adds `Show`,
// `IFileDialog` the next twenty-three; `IShellItem` follows `IUnknown` directly.
const RELEASE: usize = 2;
const DIALOG_SHOW: usize = 3;
const DIALOG_SET_OPTIONS: usize = 9;
const DIALOG_GET_OPTIONS: usize = 10;
#[cfg(test)]
const DIALOG_SET_FOLDER: usize = 12;
const DIALOG_GET_RESULT: usize = 20;
const ITEM_GET_DISPLAY_NAME: usize = 5;

/// The longest path that is read back (in UTF-16 units): the limit of the system for a long path.
const MAX_PATH_UNITS: usize = 32_768;

/// The answer of a dialog the person closed without a choice: `HRESULT_FROM_WIN32(ERROR_CANCELLED)`.
const CANCELLED: HRESULT = (0x8007_0000_u32 | ERROR_CANCELLED) as HRESULT;

/// A reference to an object of the system, given back when dropped.
struct Object(*mut c_void);

impl Object {
    /// The method at `index` of the object's table, as a function of type `F`.
    ///
    /// # Safety
    ///
    /// The object has a method of exactly that type at that place.
    unsafe fn method<F: Copy>(&self, index: usize) -> F {
        debug_assert_eq!(size_of::<F>(), size_of::<*const c_void>());
        // SAFETY: an interface pointer points to a pointer to its table of function pointers;
        // the caller vouches for the index and the type.
        unsafe {
            let table = *self.0.cast::<*const *const c_void>();
            std::mem::transmute_copy(&*table.add(index))
        }
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        type Release = unsafe extern "system" fn(*mut c_void) -> u32;
        // SAFETY: `Release` is the third method of every interface; the reference is ours and
        // is not used again.
        unsafe { self.method::<Release>(RELEASE)(self.0) };
    }
}

/// The single-threaded apartment of the calling thread, left when dropped.
struct Apartment;

impl Apartment {
    fn enter() -> Result<Self, String> {
        // SAFETY: the reserved pointer is null as documented; the result is checked.
        let entered = unsafe {
            CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        // S_FALSE: the thread is in such an apartment already, and the call still counts.
        if entered == S_OK || entered == S_FALSE {
            Ok(Self)
        } else {
            Err(failed("the folder dialog could not be prepared", entered))
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: pairs the successful `CoInitializeEx` of `enter` on this thread.
        unsafe { CoUninitialize() };
    }
}

fn failed(what: &str, code: HRESULT) -> String {
    format!("{what} (the system answered {:#010x})", code as u32)
}

/// The dialog, made and set to ask for a folder of the file system. Lives on the thread that made
/// it, inside an [`Apartment`].
struct Dialog(Object);

impl Dialog {
    fn new() -> Result<Self, String> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: the ids are constants, no outer object, `raw` receives the reference.
        let made = unsafe {
            CoCreateInstance(
                &FileOpenDialog,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_FILE_OPEN_DIALOG,
                &mut raw,
            )
        };
        if made != S_OK || raw.is_null() {
            return Err(failed("the folder dialog could not be made", made));
        }
        let dialog = Self(Object(raw));
        let options = dialog.options()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM;
        type SetOptions = unsafe extern "system" fn(*mut c_void, u32) -> HRESULT;
        // SAFETY: `IFileDialog::SetOptions(FILEOPENDIALOGOPTIONS)` at its place in the table.
        let set = unsafe { dialog.0.method::<SetOptions>(DIALOG_SET_OPTIONS)(raw, options) };
        if set != S_OK {
            return Err(failed("the folder dialog could not be set up", set));
        }
        Ok(dialog)
    }

    fn options(&self) -> Result<u32, String> {
        type GetOptions = unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
        let mut options = 0u32;
        // SAFETY: `IFileDialog::GetOptions(FILEOPENDIALOGOPTIONS*)` at its place in the table.
        let got =
            unsafe { self.0.method::<GetOptions>(DIALOG_GET_OPTIONS)(self.0 .0, &mut options) };
        if got == S_OK {
            Ok(options)
        } else {
            Err(failed("the folder dialog could not be set up", got))
        }
    }

    /// The folder the dialog opens in (the tests: a folder they made).
    #[cfg(test)]
    fn open_in(&self, dir: &std::path::Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt as _;
        use windows_sys::Win32::UI::Shell::SHCreateItemFromParsingName;

        /// `IShellItem`.
        const IID_SHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and lives for the call; `raw` receives the reference.
        let made = unsafe {
            SHCreateItemFromParsingName(
                wide.as_ptr(),
                std::ptr::null_mut(),
                &IID_SHELL_ITEM,
                &mut raw,
            )
        };
        if made != S_OK || raw.is_null() {
            return Err(failed("no item for the folder", made));
        }
        let item = Object(raw);
        type SetFolder = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;
        // SAFETY: `IFileDialog::SetFolder(IShellItem*)` at its place in the table.
        let set = unsafe { self.0.method::<SetFolder>(DIALOG_SET_FOLDER)(self.0 .0, item.0) };
        if set == S_OK {
            Ok(())
        } else {
            Err(failed("the folder could not be set", set))
        }
    }

    /// Show the dialog and wait for the person. `None`: closed without a choice.
    fn ask(&self) -> Result<Option<PathBuf>, String> {
        type Show = unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT;
        // SAFETY: `IModalWindow::Show(HWND)` at its place; no owner window (a hub has none).
        let shown = unsafe { self.0.method::<Show>(DIALOG_SHOW)(self.0 .0, std::ptr::null_mut()) };
        if shown == CANCELLED {
            return Ok(None);
        }
        if shown != S_OK {
            return Err(failed("the folder dialog could not be shown", shown));
        }
        type GetResult = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: `IFileDialog::GetResult(IShellItem**)` at its place; `raw` receives the reference.
        let got = unsafe { self.0.method::<GetResult>(DIALOG_GET_RESULT)(self.0 .0, &mut raw) };
        if got != S_OK || raw.is_null() {
            return Err(failed("the folder dialog gave no folder", got));
        }
        let item = Object(raw);
        type GetDisplayName = unsafe extern "system" fn(*mut c_void, i32, *mut *mut u16) -> HRESULT;
        let mut name: *mut u16 = std::ptr::null_mut();
        // SAFETY: `IShellItem::GetDisplayName(SIGDN, PWSTR*)` at its place; `name` receives a
        // string of the system that is freed below.
        let named = unsafe {
            item.method::<GetDisplayName>(ITEM_GET_DISPLAY_NAME)(
                item.0,
                SIGDN_FILESYSPATH,
                &mut name,
            )
        };
        if named != S_OK || name.is_null() {
            return Err(failed("the chosen folder has no path", named));
        }
        // SAFETY: `name` is a NUL-terminated string of the system; it is read up to its end or
        // the longest path there is, copied, and freed once.
        let path = unsafe {
            let mut len = 0;
            while len < MAX_PATH_UNITS && *name.add(len) != 0 {
                len += 1;
            }
            let path = OsString::from_wide(std::slice::from_raw_parts(name, len));
            CoTaskMemFree(name.cast());
            path
        };
        Ok(Some(PathBuf::from(path)))
    }
}

/// Run `work` on a thread of its own, inside a single-threaded apartment, and wait for it.
fn on_own_thread<T: Send>(work: impl FnOnce() -> Result<T, String> + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("folder-dialog".to_owned())
            .spawn_scoped(scope, || {
                let _apartment = Apartment::enter()?;
                work()
            })
            .map_err(|_| "no thread for the folder dialog".to_owned())?
            .join()
            .map_err(|_| "the folder dialog broke off".to_owned())?
    })
}

/// Ask the person for a folder and wait for the answer. `Ok(None)`: the dialog was closed without
/// a choice. Blocks the calling thread; the dialog runs on a thread of its own.
///
/// # Errors
///
/// The system could not make or show the dialog.
pub fn pick_folder() -> Result<Option<PathBuf>, String> {
    on_own_thread(|| Dialog::new()?.ask())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::time::{Duration, Instant};

    use appkit_scratch::Scratch;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetClassNameW, IsWindowVisible, PostMessageW, WM_COMMAND,
    };

    use super::*;

    /// The dialog is an object in this process, and it asks for a folder of the file system. No
    /// window is shown: the dialog is made and looked at, never asked.
    #[test]
    fn the_dialog_is_made_in_this_process_and_asks_for_a_folder() {
        let options = on_own_thread(|| Dialog::new()?.options()).unwrap();
        assert_ne!(options & FOS_PICKFOLDERS, 0, "{options:#x}");
        assert_ne!(options & FOS_FORCEFILESYSTEM, 0, "{options:#x}");
    }

    /// The answer of a closed dialog is the one the system documents.
    #[test]
    fn a_closed_dialog_is_told_by_the_code_of_the_system() {
        assert_eq!(CANCELLED as u32, 0x8007_04C7);
    }

    /// The processes this one started that still run.
    fn children() -> Vec<String> {
        let mut found = Vec::new();
        // SAFETY: a snapshot of the process list, read entry by entry into a struct of the
        // size it names, and closed.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap();
            let me = GetCurrentProcessId();
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                if entry.th32ParentProcessID == me {
                    let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(0);
                    found.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
                }
                more = Process32NextW(snapshot, &mut entry);
            }
            windows_sys::Win32::Foundation::CloseHandle(snapshot);
        }
        found
    }

    /// The visible dialog window of `thread`, if it has one by now.
    fn dialog_window(thread: u32) -> Option<HWND> {
        unsafe extern "system" fn look(window: HWND, found: LPARAM) -> BOOL {
            let mut class = [0u16; 16];
            // SAFETY: the buffer and its length belong together; `found` points to the
            // `AtomicIsize` of the caller, which outlives the enumeration.
            unsafe {
                let len = GetClassNameW(window, class.as_mut_ptr(), 16);
                let name = String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)]);
                if name == "#32770" && IsWindowVisible(window) != 0 {
                    (*(found as *const AtomicIsize)).store(window as isize, Ordering::SeqCst);
                    return 0;
                }
            }
            1
        }
        let found = AtomicIsize::new(0);
        // SAFETY: the callback gets the address of `found`, which lives for the call.
        unsafe { EnumThreadWindows(thread, Some(look), &raw const found as LPARAM) };
        let window = found.load(Ordering::SeqCst);
        (window != 0).then_some(window as HWND)
    }

    /// Show the real dialog in `dir` and press `button` (1: the choice, 2: cancel) once it is
    /// there. Returns the answer and the children this process had while the dialog was open.
    fn drive(dir: &std::path::Path, button: WPARAM) -> (Option<PathBuf>, Vec<String>) {
        let thread = AtomicIsize::new(0);
        std::thread::scope(|scope| {
            let presser = scope.spawn(|| {
                let end = Instant::now() + Duration::from_secs(30);
                loop {
                    assert!(Instant::now() < end, "no dialog appeared");
                    std::thread::sleep(Duration::from_millis(100));
                    let id = u32::try_from(thread.load(Ordering::SeqCst)).unwrap();
                    if let Some(window) = (id != 0).then(|| dialog_window(id)).flatten() {
                        // Let the dialog finish opening its folder.
                        std::thread::sleep(Duration::from_millis(1500));
                        let while_open = children();
                        // SAFETY: a message to a window that was just found; a window that is
                        // gone makes the call fail, nothing else.
                        unsafe { PostMessageW(window, WM_COMMAND, button, 0) };
                        return while_open;
                    }
                }
            });
            let answer = on_own_thread(|| {
                // SAFETY: no arguments.
                thread.store(unsafe { GetCurrentThreadId() } as isize, Ordering::SeqCst);
                let dialog = Dialog::new()?;
                dialog.open_in(dir)?;
                dialog.ask()
            })
            .unwrap();
            (answer, presser.join().unwrap())
        })
    }

    /// By hand, it shows the real dialog for a moment and presses its buttons itself
    /// (`cargo test -p appkit-hub -- --ignored the_real_dialog`): the chosen folder arrives as it
    /// is named, a closed dialog chooses nothing, and no process is started for either.
    #[test]
    #[ignore = "shows the folder dialog of the system"]
    fn the_real_dialog_gives_the_folder_as_it_is_named_and_starts_no_process() {
        let scratch = Scratch::new("folder-dialog");
        let dir = scratch.join("Ordner äöü 日本 😀");
        std::fs::create_dir(&dir).unwrap();
        let before = children();

        let (chosen, while_open) = drive(&dir, 1);
        assert_eq!(chosen.as_deref(), Some(dir.as_path()));
        assert_eq!(while_open, before, "the dialog started a process");

        let (closed, while_open) = drive(&dir, 2);
        assert_eq!(closed, None);
        assert_eq!(while_open, before, "the dialog started a process");
    }
}
