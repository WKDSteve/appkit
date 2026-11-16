//! The commands of `testing-sheet-driver/1` and the steps of a desktop sheet, without the system.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long `wait` looks by default, in milliseconds.
const WAIT_MS: u64 = 2000;
/// `expect` looks this long too: a window draws its change in a later frame, and what the driver
/// did itself must not be a race. Measured: 15 ms typical, 56 ms at worst (analysis of E2).
const EXPECT_MS: u64 = 1000;
/// How long a window that was asked to close may take to go. One that is still there asks something
/// (unsaved work): the step is red, the window is never killed.
const GONE_MS: u64 = 5000;
const POLL: Duration = Duration::from_millis(20);

/// How a step names its element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum By {
    /// The whole accessible name.
    #[default]
    Name,
    /// A part of the name of a text (`contains`): a message whose wording holds paths of the run.
    TextPart,
}

/// What a step can ask about an element; every field given has to hold.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Want {
    /// The name, or the part of it with [`By::TextPart`].
    pub name: String,
    pub by: By,
    /// `on` or `off`.
    pub state: Option<bool>,
    pub value: Option<f64>,
    /// A part of the title of another program's window to look in instead of the app's: the
    /// driver only reads there; the one thing it does to such a window is the `close` step.
    pub window: Option<String>,
}

/// An element as a sheet sees it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Seen {
    pub state: Option<bool>,
    pub value: Option<f64>,
}

/// The window of one program.
pub trait Desktop {
    /// Finds the window of process `pid`, whose title contains `title` when one is given, and sizes it.
    fn open(
        &mut self,
        pid: u32,
        title: Option<&str>,
        cols: u32,
        rows: u32,
        wait: Duration,
    ) -> Result<(), String>;
    /// The one element called `name` (with [`By::TextPart`]: the one text whose name contains `name`):
    /// `Ok(None)` when there is none now, an error when there are several (a guess would act on the
    /// wrong control).
    fn find(&mut self, name: &str, by: By) -> Result<Option<Seen>, String>;
    /// The one element called `name` (as in [`Desktop::find`]) in the one visible window of another
    /// process (not the app's) whose title contains `window`, read only (no size, no focus): `Ok(None)`
    /// when there is no such window now, `Ok(Some(None))` when it has no such element, an error when
    /// several windows or elements fit.
    fn look(&mut self, window: &str, name: &str, by: By) -> Result<Option<Option<Seen>>, String>;
    /// Asks the one visible window of another process (not the app's) whose title contains
    /// `window` to close, as its close button does (no kill, no keys): `Ok(false)` when there is
    /// no such window now, an error when several fit or it offers no closing.
    fn shut(&mut self, window: &str) -> Result<bool, String>;
    /// How many visible windows of other processes (not the app's) have `window` in their title.
    fn count(&mut self, window: &str) -> Result<usize, String>;
    fn toggle(&mut self, name: &str) -> Result<(), String>;
    fn click(&mut self, name: &str) -> Result<(), String>;
    fn set(&mut self, name: &str, value: f64) -> Result<(), String>;
    /// Writes a picture of the window to `file`.
    fn shot(&mut self, file: &str) -> Result<(), String>;
}

/// Where the system has no such thing yet: a reported skip, never a green run.
#[cfg(not(windows))]
pub struct NoDesktop;

#[cfg(not(windows))]
impl Desktop for NoDesktop {
    fn open(&mut self, _: u32, _: Option<&str>, _: u32, _: u32, _: Duration) -> Result<(), String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn find(&mut self, _: &str, _: By) -> Result<Option<Seen>, String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn look(&mut self, _: &str, _: &str, _: By) -> Result<Option<Option<Seen>>, String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn shut(&mut self, _: &str) -> Result<bool, String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn count(&mut self, _: &str) -> Result<usize, String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn toggle(&mut self, _: &str) -> Result<(), String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn click(&mut self, _: &str) -> Result<(), String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn set(&mut self, _: &str, _: f64) -> Result<(), String> {
        Err(UNSUPPORTED.to_owned())
    }
    fn shot(&mut self, _: &str) -> Result<(), String> {
        Err(UNSUPPORTED.to_owned())
    }
}

#[cfg(not(windows))]
const UNSUPPORTED: &str = "this driver has only a Windows part so far";

/// Reads what a `wait` or `expect` step wants: a name, or `{ name | contains, state, value, window }`.
fn want(value: &Value) -> Result<Want, String> {
    if let Some(name) = value.as_str() {
        return Ok(Want {
            name: name.to_owned(),
            ..Want::default()
        });
    }
    let (name, by) = match (value.get("name"), value.get("contains")) {
        (Some(Value::String(name)), None) => (name, By::Name),
        (None, Some(Value::String(part))) if !part.is_empty() => (part, By::TextPart),
        (Some(_), Some(_)) => {
            return Err("it names an element by name or by contains, not both".to_owned())
        }
        (None, Some(_)) => {
            return Err("contains is a part of a text, a non-empty string".to_owned())
        }
        _ => return Err("it names no element".to_owned()),
    };
    let state = match value.get("state").and_then(Value::as_str) {
        None => None,
        Some("on") => Some(true),
        Some("off") => Some(false),
        Some(other) => return Err(format!("state is on or off, not {other}")),
    };
    let window = match value.get("window") {
        None => None,
        Some(Value::String(w)) if !w.is_empty() => Some(w.clone()),
        Some(_) => return Err("window is a part of a title, a non-empty string".to_owned()),
    };
    Ok(Want {
        name: name.clone(),
        by,
        state,
        value: value.get("value").and_then(Value::as_f64),
        window,
    })
}

/// Whether the element holds what is wanted, and otherwise what it shows instead.
fn holds(want: &Want, seen: Option<&Seen>) -> Result<(), String> {
    let Some(seen) = seen else {
        return Err(match want.by {
            By::Name => format!("no element is called {:?}", want.name),
            By::TextPart => format!("no text contains {:?}", want.name),
        });
    };
    if let Some(state) = want.state
        && seen.state != Some(state)
    {
        return Err(format!("{:?} is {:?}", want.name, seen.state));
    }
    if let Some(value) = want.value {
        // A level comes back through f32 and a scale: a hundredth is no difference a person hears.
        match seen.value {
            Some(got) if (got - value).abs() < 0.01 => {}
            got => return Err(format!("{:?} has the value {got:?}", want.name)),
        }
    }
    Ok(())
}

fn await_want(desktop: &mut dyn Desktop, want: &Want, ms: u64) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        let result = match &want.window {
            None => holds(want, desktop.find(&want.name, want.by)?.as_ref()),
            // Another program's windows come and go while it works: two of them for a moment, or one
            // that closes between the listing and the read, is "not yet", said only at the deadline.
            Some(window) => match desktop.look(window, &want.name, want.by) {
                Err(why) => Err(why),
                Ok(None) => Err(format!("no window has {window:?} in its title")),
                Ok(Some(seen)) => holds(want, seen.as_ref())
                    .map_err(|why| format!("in the window {window:?}: {why}")),
            },
        };
        if result.is_ok() || Instant::now() >= deadline {
            return result.map_err(|why| format!("after {ms} ms: {why}"));
        }
        std::thread::sleep(POLL);
    }
}

/// Closes the one window of another program with `window` in its title: waits up to `ms` for it to
/// be there alone, asks it to close, and waits until it is gone.
fn close_other(desktop: &mut dyn Desktop, window: &str, ms: u64) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        // as in a `wait`: a window that is not there yet, or twice there for a moment, is "not yet"
        let why = match desktop.shut(window) {
            Ok(true) => break,
            Ok(false) => format!("no window has {window:?} in its title"),
            Err(why) => why,
        };
        if Instant::now() >= deadline {
            return Err(format!("after {ms} ms: {why}"));
        }
        std::thread::sleep(POLL);
    }
    let gone = Instant::now() + Duration::from_millis(GONE_MS);
    loop {
        if desktop.count(window)? == 0 {
            return Ok(());
        }
        if Instant::now() >= gone {
            return Err(format!(
                "the window {window:?} is still open {GONE_MS} ms after it was asked to close (does it ask something?)"
            ));
        }
        std::thread::sleep(POLL);
    }
}

fn step(desktop: &mut dyn Desktop, sheet: &Sheet, step: &Value) -> Result<Option<String>, String> {
    let timeout = step.get("timeout").and_then(Value::as_u64);
    if let Some(value) = step.get("wait") {
        await_want(desktop, &want(value)?, timeout.unwrap_or(WAIT_MS))?;
    } else if let Some(value) = step.get("close") {
        let window = value
            .as_str()
            .filter(|w| !w.is_empty())
            .ok_or_else(|| "close names a part of a title, a non-empty string".to_owned())?;
        close_other(desktop, window, timeout.unwrap_or(WAIT_MS))?;
    } else if let Some(value) = step.get("expect") {
        await_want(desktop, &want(value)?, EXPECT_MS)?;
    } else if let Some(name) = step.get("toggle").and_then(Value::as_str) {
        desktop.toggle(name)?;
    } else if let Some(name) = step.get("click").and_then(Value::as_str) {
        desktop.click(name)?;
    } else if let Some(name) = step.get("set").and_then(Value::as_str) {
        let value = step
            .get("value")
            .and_then(Value::as_f64)
            .ok_or_else(|| "set needs a value".to_owned())?;
        desktop.set(name, value)?;
    } else if let Some(name) = step.get("shot").and_then(Value::as_str) {
        let file = sheet.shot_file(name)?;
        desktop.shot(&file)?;
        return Ok(Some(file));
    } else {
        return Err("this driver does not know the step".to_owned());
    }
    Ok(None)
}

/// What `open` said about the run; a later `step` writes its pictures with it.
#[derive(Debug, Default)]
struct Sheet {
    id: String,
    out_dir: String,
    tag: String,
}

impl Sheet {
    fn shot_file(&self, name: &str) -> Result<String, String> {
        // The name is checked when the sheet loads; a driver does not take a path on trust.
        if name.is_empty() || name.contains(['/', '\\', '.']) {
            return Err(format!("{name:?} cannot be part of a file name"));
        }
        std::fs::create_dir_all(&self.out_dir)
            .map_err(|e| format!("cannot create {}: {e}", self.out_dir))?;
        Ok(format!(
            "{}/{}.{name}.{}.png",
            self.out_dir, self.id, self.tag
        ))
    }
}

fn answer(fields: &Value) -> Value {
    let mut out = json!({ "schema": "testing-sheet-driver/1" });
    if let (Some(out), Some(fields)) = (out.as_object_mut(), fields.as_object()) {
        for (k, v) in fields {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

thread_local! {
    static SHEET: std::cell::RefCell<Sheet> = std::cell::RefCell::default();
}

fn open(desktop: &mut dyn Desktop, cmd: &Value) -> Value {
    let u32_of = |v: Option<&Value>| {
        v.and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let options = cmd.get("options");
    let Some(pid) = u32_of(options.and_then(|o| o.get("pid"))) else {
        return answer(&json!({ "ok": false, "error": "open names no process (options.pid)" }));
    };
    let title = options
        .and_then(|o| o.get("window"))
        .and_then(Value::as_str);
    let cols = u32_of(cmd.pointer("/size/cols")).unwrap_or(1000);
    let rows = u32_of(cmd.pointer("/size/rows")).unwrap_or(700);
    let text_of = |path: &str| {
        cmd.pointer(path)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    SHEET.with_borrow_mut(|sheet| {
        *sheet = Sheet {
            id: text_of("/sheet/id"),
            out_dir: text_of("/sheet/out_dir"),
            tag: format!("{cols}x{rows}"),
        };
    });
    let wait = Duration::from_millis(
        options
            .and_then(|o| o.get("window_ms"))
            .and_then(Value::as_u64)
            .unwrap_or(30_000),
    );
    match desktop.open(pid, title, cols, rows, wait) {
        Ok(()) => answer(&json!({ "ok": true })),
        Err(why) => answer(&json!({ "ok": false, "error": why })),
    }
}

/// One line in, one answer out, and whether another command may follow.
pub fn handle(desktop: &mut dyn Desktop, line: &str) -> (Value, bool) {
    let cmd: Value = match serde_json::from_str(line) {
        Ok(cmd) => cmd,
        Err(e) => {
            return (
                answer(&json!({ "ok": false, "error": format!("not JSON: {e}") })),
                true,
            )
        }
    };
    match cmd.get("cmd").and_then(Value::as_str) {
        Some("open") => (open(desktop, &cmd), true),
        Some("step") => {
            let Some(body) = cmd.get("step") else {
                return (
                    answer(&json!({ "ok": false, "error": "a step without a step" })),
                    true,
                );
            };
            let done = SHEET.with_borrow(|sheet| step(desktop, sheet, body));
            let reply = match done {
                Ok(Some(file)) => json!({ "ok": true, "shot": file }),
                Ok(None) => json!({ "ok": true }),
                Err(why) => json!({ "ok": false, "reason": why }),
            };
            (answer(&reply), true)
        }
        Some("close") => (answer(&json!({ "ok": true })), false),
        other => (
            answer(&json!({ "ok": false, "error": format!("unknown command {other:?}") })),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// A window in memory: `toggle` takes effect after `lag` looks, as a later frame does.
    #[derive(Default)]
    struct Fake {
        opened: Vec<(u32, Option<String>, u32, u32)>,
        elements: HashMap<String, Seen>,
        twice: Vec<String>,
        lag: u32,
        pending: Option<String>,
        shots: Vec<String>,
        /// Windows of other programs by title, after `appears` looks.
        others: HashMap<String, HashMap<String, Seen>>,
        appears: u32,
        /// Looks that see two fitting windows before the second closes.
        ambiguous: u32,
        /// Windows that ask something when asked to close, and stay.
        stubborn: Vec<String>,
        /// Windows asked to close, by title.
        shut: Vec<String>,
        /// The names of the texts of the window (a message), for `contains`.
        texts: Vec<String>,
    }

    impl Fake {
        fn fitting(&self, window: &str) -> Vec<String> {
            self.others
                .keys()
                .filter(|t| t.contains(window))
                .cloned()
                .collect()
        }
    }

    impl Desktop for Fake {
        fn open(
            &mut self,
            pid: u32,
            title: Option<&str>,
            cols: u32,
            rows: u32,
            _: Duration,
        ) -> Result<(), String> {
            if pid == 404 {
                return Err("no window".to_owned());
            }
            self.opened
                .push((pid, title.map(str::to_owned), cols, rows));
            Ok(())
        }
        fn find(&mut self, name: &str, by: By) -> Result<Option<Seen>, String> {
            if by == By::TextPart {
                return match self.texts.iter().filter(|t| t.contains(name)).count() {
                    0 => Ok(None),
                    1 => Ok(Some(Seen::default())),
                    n => Err(format!("{n} texts contain {name:?}")),
                };
            }
            if self.twice.iter().any(|n| n == name) {
                return Err(format!("2 elements are called {name:?}"));
            }
            if self.lag > 0 {
                self.lag -= 1;
            } else if let Some(pending) = self.pending.take() {
                self.elements.get_mut(&pending).expect("known").state = Some(true);
            }
            Ok(self.elements.get(name).cloned())
        }
        fn look(
            &mut self,
            window: &str,
            name: &str,
            by: By,
        ) -> Result<Option<Option<Seen>>, String> {
            if self.appears > 0 {
                self.appears -= 1;
                return Ok(None);
            }
            if self.ambiguous > 0 {
                self.ambiguous -= 1;
                return Err(format!("2 windows have {window:?} in their title"));
            }
            let fits: Vec<_> = self
                .others
                .iter()
                .filter(|(t, _)| t.contains(window))
                .collect();
            match fits.len() {
                0 => Ok(None),
                1 if by == By::TextPart => Ok(Some(
                    fits[0]
                        .1
                        .iter()
                        .find(|(text, _)| text.contains(name))
                        .map(|(_, seen)| seen.clone()),
                )),
                1 => Ok(Some(fits[0].1.get(name).cloned())),
                n => Err(format!("{n} windows have {window:?} in their title")),
            }
        }
        fn shut(&mut self, window: &str) -> Result<bool, String> {
            if self.appears > 0 {
                self.appears -= 1;
                return Ok(false);
            }
            if self.ambiguous > 0 {
                self.ambiguous -= 1;
                return Err(format!("2 windows have {window:?} in their title"));
            }
            let fits = self.fitting(window);
            match fits.as_slice() {
                [] => Ok(false),
                [one] => {
                    self.shut.push(one.clone());
                    if !self.stubborn.contains(one) {
                        self.others.remove(one);
                    }
                    Ok(true)
                }
                many => Err(format!(
                    "{} windows have {window:?} in their title",
                    many.len()
                )),
            }
        }
        fn count(&mut self, window: &str) -> Result<usize, String> {
            Ok(self.fitting(window).len())
        }
        fn toggle(&mut self, name: &str) -> Result<(), String> {
            self.pending = Some(name.to_owned());
            self.lag = 3;
            Ok(())
        }
        fn click(&mut self, name: &str) -> Result<(), String> {
            if self.elements.contains_key(name) {
                Ok(())
            } else {
                Err(format!("no element is called {name:?}"))
            }
        }
        fn set(&mut self, name: &str, value: f64) -> Result<(), String> {
            self.elements.get_mut(name).ok_or("unknown")?.value = Some(value);
            Ok(())
        }
        fn shot(&mut self, file: &str) -> Result<(), String> {
            self.shots.push(file.to_owned());
            Ok(())
        }
    }

    fn fake() -> Fake {
        let mut f = Fake::default();
        f.elements.insert(
            "mute".to_owned(),
            Seen {
                state: Some(false),
                value: None,
            },
        );
        f.elements.insert(
            "gain".to_owned(),
            Seen {
                state: None,
                value: Some(-3.0),
            },
        );
        f
    }

    fn send(f: &mut Fake, line: &Value) -> Value {
        handle(f, &line.to_string()).0
    }

    fn run(f: &mut Fake, body: &Value) -> Value {
        send(f, &json!({ "cmd": "step", "n": 1, "step": body }))
    }

    #[test]
    fn every_answer_names_the_schema() {
        let mut f = fake();
        let a = send(&mut f, &json!({ "cmd": "close" }));
        assert_eq!(a["schema"], "testing-sheet-driver/1");
        assert_eq!(a["ok"], true);
    }

    #[test]
    fn close_ends_and_nothing_else_does() {
        let mut f = fake();
        assert!(!handle(&mut f, r#"{"cmd":"close"}"#).1);
        assert!(handle(&mut f, r#"{"cmd":"launch"}"#).1);
        let (a, more) = handle(&mut f, "not json");
        assert!(more);
        assert_eq!(a["ok"], false);
    }

    #[test]
    fn open_takes_the_process_the_title_and_the_size() {
        let mut f = fake();
        let a = send(
            &mut f,
            &json!({
                "cmd": "open",
                "sheet": { "id": "a.b", "root": "r", "out_dir": "o" },
                "size": { "cols": 1300, "rows": 800 },
                "options": { "pid": 7, "window": "pult" },
            }),
        );
        assert_eq!(a["ok"], true, "{a}");
        assert_eq!(f.opened, [(7, Some("pult".to_owned()), 1300, 800)]);
    }

    #[test]
    fn open_without_a_process_or_a_window_says_so() {
        let mut f = fake();
        let a = send(&mut f, &json!({ "cmd": "open", "options": {} }));
        assert_eq!(a["ok"], false);
        let a = send(&mut f, &json!({ "cmd": "open", "options": { "pid": 404 } }));
        assert_eq!(a["error"], "no window");
    }

    #[test]
    fn expect_gives_a_later_frame_its_time() {
        let mut f = fake();
        assert_eq!(run(&mut f, &json!({ "toggle": "mute" }))["ok"], true);
        let a = run(
            &mut f,
            &json!({ "expect": { "name": "mute", "state": "on" } }),
        );
        assert_eq!(a["ok"], true, "{a}");
    }

    #[test]
    fn a_wrong_state_or_value_is_a_reason() {
        let mut f = fake();
        let a = run(
            &mut f,
            &json!({ "expect": { "name": "gain", "value": -12 } }),
        );
        assert_eq!(a["ok"], false);
        assert!(a["reason"].as_str().unwrap().contains("-3.0"), "{a}");
        let a = run(
            &mut f,
            &json!({ "wait": { "name": "mute", "state": "on" }, "timeout": 30 }),
        );
        assert!(a["reason"].as_str().unwrap().contains("after 30 ms"), "{a}");
        let a = run(
            &mut f,
            &json!({ "wait": { "name": "mute", "state": "red" } }),
        );
        assert!(a["reason"].as_str().unwrap().contains("on or off"), "{a}");
    }

    #[test]
    fn set_and_read_back() {
        let mut f = fake();
        assert_eq!(
            run(&mut f, &json!({ "set": "gain", "value": -12 }))["ok"],
            true
        );
        let a = run(
            &mut f,
            &json!({ "expect": { "name": "gain", "value": -12 } }),
        );
        assert_eq!(a["ok"], true, "{a}");
        let a = run(&mut f, &json!({ "set": "gain" }));
        assert!(
            a["reason"].as_str().unwrap().contains("needs a value"),
            "{a}"
        );
    }

    #[test]
    fn a_name_that_is_not_there_or_twice_there_never_acts() {
        let mut f = fake();
        f.twice.push("A1".to_owned());
        let a = run(&mut f, &json!({ "wait": { "name": "A1" }, "timeout": 500 }));
        assert!(a["reason"].as_str().unwrap().contains("2 elements"), "{a}");
        let a = run(&mut f, &json!({ "click": "nope" }));
        assert_eq!(a["ok"], false);
    }

    #[test]
    fn a_shot_names_its_file_and_takes_no_path() {
        let scratch = appkit_scratch::Scratch::new("sheet-driver-shot");
        let out = scratch.path().to_string_lossy().replace('\\', "/");
        let mut f = fake();
        send(
            &mut f,
            &json!({
                "cmd": "open",
                "sheet": { "id": "a.b", "root": "r", "out_dir": out },
                "size": { "cols": 10, "rows": 20 },
                "options": { "pid": 1 },
            }),
        );
        let a = run(&mut f, &json!({ "shot": "desk" }));
        assert_eq!(a["shot"], format!("{out}/a.b.desk.10x20.png"), "{a}");
        assert_eq!(f.shots.len(), 1);
        let a = run(&mut f, &json!({ "shot": "../x" }));
        assert_eq!(a["ok"], false);
    }

    /// A message holds paths of the run: `contains` finds the one text with that part, and none or two
    /// such texts are a reason, as with a name.
    #[test]
    fn contains_finds_the_one_text_with_that_part() {
        let mut f = fake();
        f.texts.push(
            "the hub ended before it came up: invalid settings file C:\\t\\config.json: EOF while parsing a value; see C:\\t\\hub.log"
                .to_owned(),
        );
        f.texts.push("OK".to_owned());
        let a = run(
            &mut f,
            &json!({ "wait": { "contains": "EOF while parsing" } }),
        );
        assert_eq!(a["ok"], true, "{a}");
        let a = run(&mut f, &json!({ "expect": { "contains": "hub.log" } }));
        assert_eq!(a["ok"], true, "{a}");
        // not there: the reason names the part
        let a = run(
            &mut f,
            &json!({ "wait": { "contains": "did not come up" }, "timeout": 50 }),
        );
        assert_eq!(a["ok"], false);
        assert!(
            a["reason"]
                .as_str()
                .unwrap()
                .contains("no text contains \"did not come up\""),
            "{a}"
        );
        // twice there: never a guess
        f.texts.push("see the hub.log".to_owned());
        let a = run(
            &mut f,
            &json!({ "wait": { "contains": "hub.log" }, "timeout": 50 }),
        );
        assert_eq!(a["ok"], false);
        assert!(
            a["reason"].as_str().unwrap().contains("2 texts contain"),
            "{a}"
        );
        // a name and a part, or an empty part, is no step
        for body in [
            json!({ "wait": { "name": "OK", "contains": "O" } }),
            json!({ "wait": { "contains": "" } }),
            json!({ "wait": { "contains": 3 } }),
        ] {
            assert_eq!(run(&mut f, &body)["ok"], false, "{body}");
        }
    }

    /// `contains` reads in the window of another program too.
    #[test]
    fn contains_looks_into_the_window_of_another_program() {
        let mut f = fake();
        f.others.insert(
            "Error - App".to_owned(),
            HashMap::from([("it failed: see C:\\x\\hub.log".to_owned(), Seen::default())]),
        );
        let a = run(
            &mut f,
            &json!({ "wait": { "contains": "hub.log", "window": "Error" }, "timeout": 500 }),
        );
        assert_eq!(a["ok"], true, "{a}");
    }

    #[test]
    fn a_wait_looks_into_the_window_of_another_program() {
        let mut f = fake();
        let mut explorer = HashMap::new();
        explorer.insert("report.txt".to_owned(), Seen::default());
        f.others
            .insert("Downloads - File Explorer".to_owned(), explorer);
        f.appears = 3;
        let a = run(
            &mut f,
            &json!({ "wait": { "name": "report.txt", "window": "Downloads" }, "timeout": 2000 }),
        );
        assert_eq!(a["ok"], true, "{a}");
        // the app's own window does not answer for it, nor the other window for the app
        let a = run(
            &mut f,
            &json!({ "expect": { "name": "mute", "window": "Downloads" } }),
        );
        assert!(
            a["reason"]
                .as_str()
                .unwrap()
                .contains("in the window \"Downloads\""),
            "{a}"
        );
        let a = run(&mut f, &json!({ "expect": { "name": "report.txt" } }));
        assert_eq!(a["ok"], false, "{a}");
    }

    #[test]
    fn a_window_that_is_twice_there_for_a_moment_does_not_end_a_wait() {
        let mut f = fake();
        let mut dialog = HashMap::new();
        dialog.insert("OK".to_owned(), Seen::default());
        f.others.insert("Save - Notepad".to_owned(), dialog);
        f.ambiguous = 3;
        let a = run(
            &mut f,
            &json!({ "wait": { "name": "OK", "window": "Notepad" }, "timeout": 2000 }),
        );
        assert_eq!(a["ok"], true, "{a}");
    }

    #[test]
    fn another_window_that_is_not_there_or_twice_there_is_a_reason() {
        let mut f = fake();
        let a = run(
            &mut f,
            &json!({ "wait": { "name": "x", "window": "Outlook" }, "timeout": 50 }),
        );
        assert!(
            a["reason"]
                .as_str()
                .unwrap()
                .contains("no window has \"Outlook\" in its title"),
            "{a}"
        );
        f.others
            .insert("Inbox - Outlook".to_owned(), HashMap::new());
        f.others
            .insert("Calendar - Outlook".to_owned(), HashMap::new());
        let a = run(
            &mut f,
            &json!({ "wait": { "name": "x", "window": "Outlook" } }),
        );
        assert!(a["reason"].as_str().unwrap().contains("2 windows"), "{a}");
        let a = run(&mut f, &json!({ "wait": { "name": "x", "window": "" } }));
        assert!(a["reason"].as_str().unwrap().contains("non-empty"), "{a}");
    }

    #[test]
    fn close_waits_for_the_one_window_and_until_it_is_gone() {
        let mut f = fake();
        f.others
            .insert("clip.mkv - Media Player".to_owned(), HashMap::new());
        f.others
            .insert("Downloads - File Explorer".to_owned(), HashMap::new());
        f.appears = 3;
        let a = run(&mut f, &json!({ "close": "clip.mkv" }));
        assert_eq!(a["ok"], true, "{a}");
        assert_eq!(f.shut, ["clip.mkv - Media Player"]);
        assert!(
            f.others.contains_key("Downloads - File Explorer"),
            "only that one window"
        );
    }

    #[test]
    fn close_without_one_window_or_against_a_question_is_a_reason() {
        let mut f = fake();
        let a = run(&mut f, &json!({ "close": "Player", "timeout": 50 }));
        assert!(
            a["reason"]
                .as_str()
                .unwrap()
                .contains("after 50 ms: no window has \"Player\" in its title"),
            "{a}"
        );
        f.others.insert("a - Player".to_owned(), HashMap::new());
        f.others.insert("b - Player".to_owned(), HashMap::new());
        let a = run(&mut f, &json!({ "close": "Player", "timeout": 50 }));
        assert!(a["reason"].as_str().unwrap().contains("2 windows"), "{a}");
        assert!(f.shut.is_empty(), "two fitting windows: neither is closed");
        f.others.remove("b - Player");
        f.stubborn.push("a - Player".to_owned());
        let a = run(&mut f, &json!({ "close": "Player" }));
        assert!(a["reason"].as_str().unwrap().contains("still open"), "{a}");
        let a = run(&mut f, &json!({ "close": "" }));
        assert!(a["reason"].as_str().unwrap().contains("non-empty"), "{a}");
    }

    #[test]
    fn a_window_twice_there_for_a_moment_does_not_end_a_close() {
        let mut f = fake();
        f.others.insert("x - Player".to_owned(), HashMap::new());
        f.ambiguous = 3;
        let a = run(&mut f, &json!({ "close": "Player" }));
        assert_eq!(a["ok"], true, "{a}");
    }

    #[test]
    fn an_unknown_step_is_no_success() {
        let mut f = fake();
        assert_eq!(run(&mut f, &json!({ "press": "Tab" }))["ok"], false);
    }
}
