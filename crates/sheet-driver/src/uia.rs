//! The window of a program through Windows UI Automation: what a screen reader sees and does.
//!
//! Measured against `pult-app --demo` (testing.nvim, E2): the window does not need the focus and may
//! be covered or off the screen; a minimized window or one in the tray does not answer. An action
//! goes around the hit test of the mouse, so a covered button "works".
#![allow(unsafe_code)]

use std::time::{Duration, Instant};

use windows::core::{BOOL, BSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationRangeValuePattern, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
    ToggleState_On, TreeScope_Descendants, UIA_InvokePatternId, UIA_NamePropertyId,
    UIA_RangeValuePatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
};

use crate::protocol::{Desktop, Seen};

/// `PW_RENDERFULLCONTENT`: also what the GPU draws, which is all of an egui window.
const FULL_CONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);

/// The title of the invisible window winit keeps next to the real one (E2).
const NOT_THE_WINDOW: &str = "Winit Thread Event Target";

#[derive(Default)]
pub struct Uia {
    automation: Option<IUIAutomation>,
    hwnd: Option<isize>,
    root: Option<IUIAutomationElement>,
}

struct Search {
    /// The process, or `None` for a window of any process.
    pid: Option<u32>,
    title: Option<String>,
    found: Vec<isize>,
    /// Stop at the first hit (the app's window) or collect them all (another program's).
    first: bool,
}

extern "system" fn look_at(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `Search` that `find_window` keeps alive for the whole of EnumWindows,
    // which calls this on the same thread.
    let search = unsafe { &mut *(data.0 as *mut Search) };
    let mut pid = 0;
    // SAFETY: a window handle the system just gave us and a local to write to.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    // SAFETY: the same handle; both calls only read.
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    if search.pid.is_some_and(|p| p != pid) || !visible {
        return true.into();
    }
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer outlives the call and the length is taken from it.
    let length = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    let text = String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap_or(0)]);
    let wanted = search.title.as_deref().is_none_or(|t| text.contains(t));
    if text == NOT_THE_WINDOW || !wanted {
        return true.into();
    }
    search.found.push(hwnd.0 as isize);
    (!search.first).into()
}

fn windows(pid: Option<u32>, title: Option<&str>, first: bool) -> Vec<isize> {
    let mut search = Search {
        pid,
        title: title.map(str::to_owned),
        found: Vec::new(),
        first,
    };
    // SAFETY: the callback above uses the pointer only while EnumWindows runs, and `search` lives
    // past it. EnumWindows reports an error when the callback stops it, which is the hit.
    let _ = unsafe { EnumWindows(Some(look_at), LPARAM(&raw mut search as isize)) };
    search.found
}

fn find_window(pid: u32, title: Option<&str>) -> Option<isize> {
    windows(Some(pid), title, true).first().copied()
}

fn text(e: &windows::core::Error) -> String {
    format!("{} ({:#x})", e.message(), e.code().0)
}

impl Uia {
    fn automation(&mut self) -> Result<IUIAutomation, String> {
        if let Some(a) = &self.automation {
            return Ok(a.clone());
        }
        // SAFETY: first COM use of this thread; a second initialization only reports so.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // SAFETY: a class id and interface of the system, the result is a counted reference.
        let a: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| format!("UI Automation is not there: {}", text(&e)))?;
        self.automation = Some(a.clone());
        Ok(a)
    }

    /// The elements called `name` below the window.
    fn elements(&mut self, name: &str) -> Result<Vec<IUIAutomationElement>, String> {
        let root = self.root.clone().ok_or("the window is not open")?;
        self.elements_in(&root, name)
    }

    /// The elements called `name` below `root`.
    fn elements_in(
        &mut self,
        root: &IUIAutomationElement,
        name: &str,
    ) -> Result<Vec<IUIAutomationElement>, String> {
        let automation = self.automation()?;
        let wanted = VARIANT::from(BSTR::from(name));
        // SAFETY: counted references and a variant that outlive each call.
        unsafe {
            let condition = automation
                .CreatePropertyCondition(UIA_NamePropertyId, &wanted)
                .map_err(|e| text(&e))?;
            let found = root
                .FindAll(TreeScope_Descendants, &condition)
                .map_err(|e| format!("the window does not answer: {}", text(&e)))?;
            let count = found.Length().map_err(|e| text(&e))?;
            (0..count)
                .map(|i| found.GetElement(i).map_err(|e| text(&e)))
                .collect()
        }
    }

    /// The one element called `name`, for an action.
    fn one(&mut self, name: &str) -> Result<IUIAutomationElement, String> {
        let mut found = self.elements(name)?;
        match found.len() {
            0 => Err(format!("no element is called {name:?}")),
            1 => Ok(found.remove(0)),
            n => Err(format!("{n} elements are called {name:?}")),
        }
    }
}

/// What a sheet sees of the one element in `found`: `None` without one, an error with several.
fn seen(mut found: Vec<IUIAutomationElement>, name: &str) -> Result<Option<Seen>, String> {
    match found.len() {
        0 => return Ok(None),
        1 => {}
        n => return Err(format!("{n} elements are called {name:?}")),
    }
    let element = found.remove(0);
    // SAFETY: counted references; a pattern the element does not offer is an error, which here
    // means it has no such state.
    unsafe {
        let state = element
            .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
            .ok()
            .and_then(|p| p.CurrentToggleState().ok())
            .map(|s| s == ToggleState_On);
        let value = element
            .GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId)
            .ok()
            .and_then(|p| p.CurrentValue().ok());
        Ok(Some(Seen { state, value }))
    }
}

impl Desktop for Uia {
    fn open(
        &mut self,
        pid: u32,
        title: Option<&str>,
        cols: u32,
        rows: u32,
        wait: Duration,
    ) -> Result<(), String> {
        let automation = self.automation()?;
        let deadline = Instant::now() + wait;
        let hwnd = loop {
            if let Some(hwnd) = find_window(pid, title) {
                break hwnd;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "process {pid} shows no window{} in {} ms (in the tray, or it is minimized?)",
                    title.map_or(String::new(), |t| format!(" with {t:?} in its title")),
                    wait.as_millis()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let handle = HWND(hwnd as *mut _);
        // SAFETY: a window of the process we were pointed at; size only, never the focus.
        unsafe {
            SetWindowPos(
                handle,
                None,
                0,
                0,
                i32::try_from(cols).unwrap_or(1000),
                i32::try_from(rows).unwrap_or(700),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(|e| format!("cannot size the window: {}", text(&e)))?;
        // SAFETY: the same handle, the result is a counted reference.
        let root = unsafe { automation.ElementFromHandle(handle) }
            .map_err(|e| format!("the window has no automation tree: {}", text(&e)))?;
        self.hwnd = Some(hwnd);
        self.root = Some(root);
        Ok(())
    }

    fn find(&mut self, name: &str) -> Result<Option<Seen>, String> {
        let found = self.elements(name)?;
        seen(found, name)
    }

    fn look(&mut self, window: &str, name: &str) -> Result<Option<Option<Seen>>, String> {
        let automation = self.automation()?;
        let hits = windows(None, Some(window), false);
        let hwnd = match hits.as_slice() {
            [] => return Ok(None),
            [one] => *one,
            many => {
                return Err(format!(
                    "{} windows have {window:?} in their title",
                    many.len()
                ))
            }
        };
        // SAFETY: a window the system just listed; reading its tree neither sizes nor focuses it.
        // It may close between the two calls, which is an error of this look, not of the run.
        let root = unsafe { automation.ElementFromHandle(HWND(hwnd as *mut _)) }
            .map_err(|e| format!("the window {window:?} has no automation tree: {}", text(&e)))?;
        let found = self.elements_in(&root, name)?;
        seen(found, name).map(Some)
    }

    fn toggle(&mut self, name: &str) -> Result<(), String> {
        let element = self.one(name)?;
        // SAFETY: counted references.
        unsafe {
            element
                .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                .and_then(|p| p.Toggle())
        }
        .map_err(|e| format!("{name:?} cannot be toggled: {}", text(&e)))
    }

    fn click(&mut self, name: &str) -> Result<(), String> {
        let element = self.one(name)?;
        // SAFETY: counted references. A tab offers selection where a button offers invoke.
        unsafe {
            if let Ok(p) =
                element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            {
                return p.Invoke().map_err(|e| text(&e));
            }
            element
                .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                    UIA_SelectionItemPatternId,
                )
                .and_then(|p| p.Select())
        }
        .map_err(|e| format!("{name:?} cannot be clicked: {}", text(&e)))
    }

    fn set(&mut self, name: &str, value: f64) -> Result<(), String> {
        let element = self.one(name)?;
        // SAFETY: counted references.
        unsafe {
            element
                .GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId)
                .and_then(|p| p.SetValue(value))
        }
        .map_err(|e| format!("{name:?} cannot be set: {}", text(&e)))
    }

    fn shot(&mut self, file: &str) -> Result<(), String> {
        let hwnd = HWND(self.hwnd.ok_or("the window is not open")? as *mut _);
        let (width, height, mut pixels) = capture(hwnd)?;
        // GDI gives blue, green, red; the alpha of a window is no information.
        for px in pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            px[3] = 255;
        }
        let out = std::fs::File::create(file).map_err(|e| format!("cannot write {file}: {e}"))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(out), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut w| w.write_image_data(&pixels))
            .map_err(|e| format!("cannot write {file}: {e}"))
    }
}

/// The client area as top-down pixels of 4 bytes.
fn capture(hwnd: HWND) -> Result<(u32, u32, Vec<u8>), String> {
    let mut rect = RECT::default();
    // SAFETY: a window handle and a local to write to.
    unsafe { GetClientRect(hwnd, &mut rect) }.map_err(|e| text(&e))?;
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width <= 0 || height <= 0 {
        return Err("the window has no area (minimized?)".to_owned());
    }
    let mut pixels = vec![0u8; usize::try_from(width * height * 4).map_err(|e| e.to_string())?];
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).map_err(|e| e.to_string())?,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    // SAFETY: every handle is made and released here, the buffer is as large as the header says.
    let lines = unsafe {
        let window_dc = GetDC(Some(hwnd));
        let dc = CreateCompatibleDC(Some(window_dc));
        let bitmap = CreateCompatibleBitmap(window_dc, width, height);
        let old = SelectObject(dc, bitmap.into());
        let printed = PrintWindow(hwnd, dc, FULL_CONTENT).as_bool();
        SelectObject(dc, old);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height.unsigned_abs(),
            Some(pixels.as_mut_ptr().cast()),
            &raw mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(Some(hwnd), window_dc);
        if printed {
            lines
        } else {
            0
        }
    };
    if lines == 0 {
        return Err("the window cannot be captured".to_owned());
    }
    Ok((width.unsigned_abs(), height.unsigned_abs(), pixels))
}
