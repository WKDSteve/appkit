//! `appkit_core::child` from outside: what a child gets, that it has no console, that the
//! limits end it, and that it ends with the app, also when the app is killed.
//!
//! The child is `child-probe` (`src/bin/child-probe.rs`). A process that lives is seen by a
//! file it appends to (`beat`): no test looks into the process list.

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use appkit_core::child::{self, Environment, Leftovers, ProcError, Run, Start, Stream};
use appkit_scratch::Scratch;

const PROBE: &str = env!("CARGO_BIN_EXE_child-probe");

fn inherited() -> Environment {
    Environment::Inherited {
        set: Vec::new(),
        remove: Vec::new(),
    }
}

fn probe(role: &[&str]) -> Start {
    Start::new("child spec", PROBE).args(role).env(inherited())
}

fn call(start: Start) -> Run {
    Run {
        start,
        stdin: Vec::new(),
        timeout: Duration::from_secs(30),
        max_stdout: 64 * 1024,
        max_stderr: 64 * 1024,
        leftovers: Leftovers::End,
    }
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn size(file: &Path) -> u64 {
    std::fs::metadata(file).map(|m| m.len()).unwrap_or(0)
}

/// The file grows: the process that writes it lives.
async fn beats(file: &Path) -> bool {
    let before = size(file);
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if size(file) > before {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// The file stays as it is for a second and a half (thirty beats): nobody writes it any more.
async fn stops(file: &Path) -> bool {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        let before = size(file);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if size(file) == before {
            return true;
        }
    }
    false
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a path of plain text")
}

#[tokio::test]
async fn the_arguments_arrive_as_they_are() {
    let given = [
        "two words",
        "quo\"te",
        "&calc",
        "%PATH%",
        "$(id)",
        "semi;colon",
        "back\\slash\\",
        "",
    ];
    let mut role = vec!["args"];
    role.extend(given);
    let out = child::run(call(probe(&role))).await.unwrap();
    assert_eq!(out.code, Some(0));
    assert_eq!(lines(&out.stdout), given);
}

/// What the helper wrote to the log, for every test of this file (the place is set once per
/// process); a test looks for the lines of a caller only it uses.
fn logged() -> &'static std::sync::Mutex<Vec<String>> {
    static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    child::set_log(|line| {
        eprintln!("{line}");
        LINES.lock().unwrap().push(line.to_owned());
    });
    &LINES
}

#[tokio::test]
async fn the_log_names_caller_and_program_once_for_calls_and_always_for_a_child_that_stays() {
    let lines = logged();
    let of = |caller: &str| -> Vec<String> {
        let all = lines.lock().unwrap();
        all.iter().filter(|l| l.contains(caller)).cloned().collect()
    };
    let call_as = |caller: &str| {
        call(
            Start::new(caller, PROBE)
                .args(["args", "secret word"])
                .env(inherited()),
        )
    };

    for _ in 0..3 {
        child::run(call_as("log spec call")).await.unwrap();
    }
    let calls = of("log spec call");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].starts_with("child started by log spec call: "),
        "{calls:?}"
    );
    assert!(calls[0].contains("child-probe"), "{calls:?}");
    assert!(calls[0].contains("(pid "), "{calls:?}");
    assert!(!calls[0].contains("secret"), "an argument is in the log");

    for _ in 0..2 {
        let start = Start::new("log spec stay", PROBE)
            .arg("sleep")
            .env(inherited());
        child::spawn(&start).unwrap().end().await;
    }
    assert_eq!(of("log spec stay").len(), 2);

    // A call that had to be ended is in the log every time, also when its start is not.
    for _ in 0..2 {
        let start = Start::new("log spec late", PROBE)
            .arg("sleep")
            .env(inherited());
        let mut run = call(start);
        run.timeout = Duration::from_millis(300);
        assert!(matches!(child::run(run).await, Err(ProcError::Timeout(_))));
    }
    let late = of("log spec late");
    assert_eq!(late.len(), 3, "{late:?}");
    assert_eq!(late.iter().filter(|l| l.contains("was ended: ")).count(), 2);
}

#[test]
fn a_start_without_a_runtime_is_refused() {
    let outcome = child::spawn(&probe(&["sleep"]));
    assert!(matches!(outcome, Err(ProcError::Spawn(_))), "{outcome:?}");
}

#[tokio::test]
async fn stdin_is_written_and_closed() {
    let mut run = call(probe(&["cat"]));
    run.stdin = b"one\ntwo\n".to_vec();
    let out = child::run(run).await.unwrap();
    assert_eq!(out.stdout, b"one\ntwo\n");
}

#[tokio::test]
async fn the_environment_is_the_one_named() {
    let only = |list: Vec<(OsString, OsString)>| Environment::Only(list);
    let ask = |name: &str, env: Environment| {
        let start = Start::new("child spec", PROBE).args(["env", name]).env(env);
        async move { lines(&child::run(call(start)).await.unwrap().stdout) }
    };
    assert_eq!(ask("PATH", only(Vec::new())).await, ["(unset)"]);
    assert_eq!(
        ask(
            "APPKIT_CHILD_SPEC",
            only(vec![("APPKIT_CHILD_SPEC".into(), "1".into())])
        )
        .await,
        ["1"]
    );
    assert_ne!(ask("PATH", inherited()).await, ["(unset)"]);
    let changed = Environment::Inherited {
        set: vec![("APPKIT_CHILD_SPEC".into(), "2".into())],
        remove: vec!["PATH".into()],
    };
    assert_eq!(ask("PATH", changed.clone()).await, ["(unset)"]);
    assert_eq!(ask("APPKIT_CHILD_SPEC", changed).await, ["2"]);
}

#[tokio::test]
async fn a_program_that_is_no_absolute_path_and_a_caller_without_a_name_are_refused() {
    let dir = Scratch::new("child-refused");
    let relative = Start::new("child spec", "child-probe").arg("sleep");
    assert!(matches!(child::spawn(&relative), Err(ProcError::Spawn(_))));
    let missing = Start::new("child spec", dir.join("nope.exe"));
    assert!(matches!(child::spawn(&missing), Err(ProcError::Spawn(_))));
    let folder = Start::new("child spec", dir.path());
    assert!(matches!(child::spawn(&folder), Err(ProcError::Spawn(_))));
    let nameless = Start::new("  ", PROBE).arg("sleep");
    assert!(matches!(child::spawn(&nameless), Err(ProcError::Spawn(_))));
    let no_folder = probe(&["sleep"]).cwd(dir.join("gone"));
    assert!(matches!(child::spawn(&no_folder), Err(ProcError::Spawn(_))));
}

#[tokio::test]
async fn a_call_that_takes_too_long_is_ended_with_what_it_started() {
    let dir = Scratch::new("child-timeout");
    let file = dir.join("beat");
    let mut run = call(probe(&["tree", text(&file)]));
    run.timeout = Duration::from_secs(3);
    let began = Instant::now();
    let outcome = child::run(run).await;
    assert!(matches!(outcome, Err(ProcError::Timeout(_))), "{outcome:?}");
    assert!(began.elapsed() < Duration::from_secs(20));
    assert!(size(&file) > 0, "the grandchild never ran");
    assert!(stops(&file).await, "the grandchild outlived the time limit");
}

#[tokio::test]
async fn a_call_that_writes_too_much_is_ended() {
    let began = Instant::now();
    let outcome = child::run(call(probe(&["flood"]))).await;
    assert!(
        matches!(
            outcome,
            Err(ProcError::TooLarge {
                stream: "stdout",
                limit: 65536
            })
        ),
        "{outcome:?}"
    );
    assert!(began.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn the_lines_of_a_call_arrive_one_by_one_while_it_runs() {
    let mut seen: Vec<(Stream, String, Duration)> = Vec::new();
    let began = Instant::now();
    let code = child::run_lines(call(probe(&["lines"])), |stream, line| {
        seen.push((stream, line.to_owned(), began.elapsed()));
    })
    .await
    .unwrap();
    assert_eq!(code, Some(0));
    let of = |which: Stream| -> Vec<&str> {
        let lines = seen.iter().filter(|(s, _, _)| *s == which);
        lines.map(|(_, line, _)| line.as_str()).collect()
    };
    assert_eq!(of(Stream::Stdout), ["out 1", "out 2", "tail"]);
    assert_eq!(of(Stream::Stderr), ["err 1"]);
    // The first line was handed over before the child wrote the second: it is not collected.
    let at = |text: &str| seen.iter().find(|(_, l, _)| l == text).unwrap().2;
    assert!(at("out 2") - at("out 1") >= Duration::from_millis(200));
}

#[tokio::test]
async fn the_limits_hold_for_a_call_read_line_by_line() {
    let outcome = child::run_lines(call(probe(&["flood"])), |_, _| {}).await;
    assert!(
        matches!(
            outcome,
            Err(ProcError::TooLarge {
                stream: "stdout",
                ..
            })
        ),
        "{outcome:?}"
    );
    let dir = Scratch::new("child-lines-timeout");
    let file = dir.join("beat");
    let mut run = call(probe(&["tree", text(&file)]));
    run.timeout = Duration::from_secs(3);
    let outcome = child::run_lines(run, |_, _| {}).await;
    assert!(matches!(outcome, Err(ProcError::Timeout(_))), "{outcome:?}");
    assert!(stops(&file).await, "the grandchild outlived the time limit");
}

#[tokio::test]
async fn what_a_call_left_running_is_ended() {
    let dir = Scratch::new("child-leftover");
    let file = dir.join("beat");
    let out = child::run(call(probe(&["leave", text(&file)])))
        .await
        .unwrap();
    assert_eq!(out.code, Some(0));
    assert!(stops(&file).await, "the leftover outlived the call");
}

#[tokio::test]
async fn what_a_call_left_running_on_purpose_stays() {
    let dir = Scratch::new("child-kept");
    let file = dir.join("beat");
    let mut run = call(probe(&["launch", text(&file)]));
    run.leftovers = Leftovers::Keep;
    child::run(run).await.unwrap();
    assert!(beats(&file).await, "the kept process was ended");
    assert!(beats(&file).await, "the kept process was ended");
    // Nothing else ends it: tell it to stop before the folder is removed.
    std::fs::write(dir.join("beat.stop"), "").unwrap();
    assert!(stops(&file).await);
}

#[tokio::test]
async fn dropping_the_child_ends_it_with_what_it_started() {
    let dir = Scratch::new("child-drop");
    let file = dir.join("beat");
    let owned = child::spawn(&probe(&["tree", text(&file)])).unwrap();
    assert!(owned.id().is_some());
    assert!(beats(&file).await, "the grandchild never ran");
    drop(owned);
    assert!(stops(&file).await, "the grandchild outlived its owner");
}

#[tokio::test]
async fn ending_the_child_ends_it_with_what_it_started() {
    let dir = Scratch::new("child-end");
    let file = dir.join("beat");
    let mut owned = child::spawn(&probe(&["tree", text(&file)])).unwrap();
    assert!(beats(&file).await, "the grandchild never ran");
    owned.end().await;
    assert!(matches!(owned.try_wait(), Ok(Some(_))), "the child lives");
    assert!(stops(&file).await, "the grandchild outlived its owner");
}

/// An app (`child-probe app`) that is killed, not asked to end: nothing of it cleans up.
/// Windows ends the job of the child, Linux sends the parent-death signal. macOS has neither
/// (see the limits in `appkit_core::child`), so this is not tested there.
#[cfg(any(windows, target_os = "linux"))]
async fn killed_app_ends(role: &str, name: &str) {
    let dir = Scratch::new(name);
    let file = dir.join("beat");
    let log = dir.join("app.log");
    let mut app = std::process::Command::new(PROBE)
        .args(["app", role, text(&file)])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let ran = beats(&file).await;
    app.kill().unwrap();
    app.wait().unwrap();
    assert!(ran, "the child of the app never ran");
    assert!(stops(&file).await, "the {role} outlived the app");
    // The start is in the log of the app: who asked, and which program.
    let said = std::fs::read_to_string(&log).unwrap();
    let line = said
        .lines()
        .find(|l| l.starts_with("child started by child-probe app: "))
        .unwrap_or_else(|| panic!("no line of the start in: {said}"));
    assert!(line.contains("child-probe"), "{line}");
    assert!(line.contains("(pid "), "{line}");
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn a_child_ends_when_the_app_is_killed() {
    killed_app_ends("beat", "child-app-killed").await;
}

/// Windows ends the whole job; on Linux only the child itself gets the signal.
#[cfg(windows)]
#[tokio::test]
async fn a_grandchild_ends_when_the_app_is_killed() {
    killed_app_ends("tree", "child-app-killed-tree").await;
}

/// The control start shows that the probe can tell: started with a console of its own (as
/// `launch`'s `Window::NewConsole` does) that console has a window; started by the helper it has
/// none. The test's own console says nothing: a test run may have a window or not.
#[cfg(windows)]
#[tokio::test]
async fn a_child_has_no_console_window() {
    use std::os::windows::process::CommandExt;

    /// `CREATE_NEW_CONSOLE`.
    const NEW_CONSOLE: u32 = 0x0000_0010;
    let plain = std::process::Command::new(PROBE)
        .arg("console")
        .creation_flags(NEW_CONSOLE)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        lines(&plain.stdout),
        ["window"],
        "the control start has no console window, so the test proves nothing"
    );
    let out = child::run(call(probe(&["console"]))).await.unwrap();
    assert_eq!(lines(&out.stdout), ["none"]);
}

/// A hub without a console (a program of the GUI subsystem, or one put in the background): a
/// console program it starts by hand gets a console of its own, with a window. Through the
/// helper it gets none.
#[cfg(windows)]
#[tokio::test]
async fn a_child_of_a_hub_without_a_console_opens_no_console_window() {
    use std::os::windows::process::CommandExt;

    /// `DETACHED_PROCESS`: the hub of this test has no console at all.
    const DETACHED: u32 = 0x0000_0008;
    let dir = Scratch::new("child-hub-console");
    let ask = |how: &str| {
        let out = dir.join(how);
        let status = std::process::Command::new(PROBE)
            .args(["hub", how, text(&out)])
            .creation_flags(DETACHED)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "the hub of the test failed ({how})");
        lines(&std::fs::read(&out).unwrap())
    };
    assert_eq!(
        ask("plain"),
        ["window"],
        "a start by hand opened no console window, so the test proves nothing"
    );
    assert_eq!(ask("helper"), ["none"]);
}
