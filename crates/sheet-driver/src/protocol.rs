//! The commands of `testing-sheet-driver/1` and the steps of a desktop sheet, without the system.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long `wait` looks by default, in milliseconds.
const WAIT_MS: u64 = 2000;
/// `expect` looks this long too: a window draws its change in a later frame, and what the driver
/// did itself must not be a race. Measured: 15 ms typical, 56 ms at worst (analysis of E2).
const EXPECT_MS: u64 = 1000;
const POLL: Duration = Duration::from_millis(20);

/// What a step can ask about an element; every field given has to hold.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Want {
    pub name: String,
    /// `on` or `off`.
    pub state: Option<bool>,
    pub value: Option<f64>,
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
    /// The one element called `name`: `Ok(None)` when there is none now, an error when there are
    /// several (a guess would act on the wrong control).
    fn find(&mut self, name: &str) -> Result<Option<Seen>, String>;
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
    fn find(&mut self, _: &str) -> Result<Option<Seen>, String> {
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

/// Reads what a `wait` or `expect` step wants: a name, or `{ name, state, value }`.
fn want(value: &Value) -> Result<Want, String> {
    if let Some(name) = value.as_str() {
        return Ok(Want {
            name: name.to_owned(),
            ..Want::default()
        });
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "it names no element".to_owned())?;
    let state = match value.get("state").and_then(Value::as_str) {
        None => None,
        Some("on") => Some(true),
        Some("off") => Some(false),
        Some(other) => return Err(format!("state is on or off, not {other}")),
    };
    Ok(Want {
        name: name.to_owned(),
        state,
        value: value.get("value").and_then(Value::as_f64),
    })
}

/// Whether the element holds what is wanted, and otherwise what it shows instead.
fn holds(want: &Want, seen: Option<&Seen>) -> Result<(), String> {
    let Some(seen) = seen else {
        return Err(format!("no element is called {:?}", want.name));
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
        let seen = desktop.find(&want.name)?;
        let result = holds(want, seen.as_ref());
        if result.is_ok() || Instant::now() >= deadline {
            return result.map_err(|why| format!("after {ms} ms: {why}"));
        }
        std::thread::sleep(POLL);
    }
}

fn step(desktop: &mut dyn Desktop, sheet: &Sheet, step: &Value) -> Result<Option<String>, String> {
    let timeout = step.get("timeout").and_then(Value::as_u64);
    if let Some(value) = step.get("wait") {
        await_want(desktop, &want(value)?, timeout.unwrap_or(WAIT_MS))?;
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
        fn find(&mut self, name: &str) -> Result<Option<Seen>, String> {
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
        let dir = std::env::temp_dir().join(format!("sheet-driver-{}", std::process::id()));
        let out = dir.to_string_lossy().replace('\\', "/");
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
        std::fs::remove_dir_all(dir).expect("the folder of the test");
    }

    #[test]
    fn an_unknown_step_is_no_success() {
        let mut f = fake();
        assert_eq!(run(&mut f, &json!({ "press": "Tab" }))["ok"], false);
    }
}
