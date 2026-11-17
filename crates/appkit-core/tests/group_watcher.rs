//! The watcher of a child's process group (`src/child/unix.rs`): when the app is killed (SIGKILL, so none of its own
//! code runs), the group of its child ends, the child and what the child started; a group that was released
//! (`Leftovers::Keep`) lives on; a `kill 0` of the child (SIGTERM, SIGHUP, SIGINT to its own group) does not end the
//! watcher; and the watcher holds none of the app's other pipes.
//!
//! Both ways a watcher comes to be: the app's own program started as the watcher (the app called
//! `child::watcher_main` first, mode `exec`), and on Linux the fork of the app (no such call, mode `fork`).
//!
//! A program of its own (`harness = false`): `watcher_main` must be the first call of `main`. The app is this
//! program again, started with `APPKIT_WATCHER_APP` naming a folder and `APPKIT_WATCHER_ROLE` what to do: it starts
//! the child through `child`, the child writes the process ids to check into the folder, and the app waits to be
//! killed.

#[cfg(unix)]
fn main() {
    unix::main();
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
#[allow(unsafe_code)]
mod unix {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use appkit_core::child::{self, Environment, Leftovers, Run, Start};

    const APP: &str = "APPKIT_WATCHER_APP";
    const ROLE: &str = "APPKIT_WATCHER_ROLE";
    const MODE: &str = "APPKIT_WATCHER_MODE";
    const ARG: &str = "--appkit-child-watcher";

    pub fn main() {
        let mode = std::env::var(MODE).unwrap_or_else(|_| "exec".to_owned());
        // The fork mode is an app that does not make the call.
        if mode == "exec" {
            child::watcher_main();
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        if let Some(dir) = std::env::var_os(APP).map(PathBuf::from) {
            let role = std::env::var(ROLE).unwrap();
            runtime.block_on(app(&dir, &role));
            return;
        }
        // The `cargo test` filter and flags are not ours to read: every case runs.
        let mut modes = vec!["exec"];
        if cfg!(target_os = "linux") {
            modes.push("fork");
        }
        for mode in modes {
            for (name, case) in [
                (
                    "a killed app takes the group of its child with it",
                    killed as fn(&Path, &str),
                ),
                ("a released group outlives the killed app", released),
                ("a kill 0 of the child does not end its watcher", kill_zero),
                ("the watcher of one child holds no pipe of another", pipes),
            ] {
                let scratch = appkit_scratch::Scratch::new("group-watcher");
                case(scratch.path(), mode);
                println!("test {name} ({mode}) ... ok");
            }
        }
    }

    fn sh() -> Start {
        Start::new("appkit test group watcher", "/bin/sh").env(Environment::Only(vec![(
            "PATH".into(),
            "/usr/bin:/bin".into(),
        )]))
    }

    /// The child writes `<its pid> <the grandchild's pid>` into `pids`, whole or not at all.
    fn write_pids(dir: &Path) -> String {
        let d = dir.display();
        format!("echo $$ $! > '{d}/pids.tmp' && mv '{d}/pids.tmp' '{d}/pids'")
    }

    /// The app's side.
    async fn app(dir: &Path, role: &str) {
        let pids = write_pids(dir);
        let _held = match role {
            "spawn" => Some(
                child::spawn(&sh().args(["-c", &format!("sleep 600 & {pids}; wait")]))
                    .expect("the child starts"),
            ),
            // The child tells its own group to stop, and ignores that itself, as does the grandchild (an ignored
            // signal stays ignored in a child); only the watcher would end.
            "kill-zero" => Some(
                child::spawn(&sh().args([
                    "-c",
                    &format!(
                        "trap '' TERM HUP INT; sleep 600 & kill -TERM 0; kill -HUP 0; kill -INT 0; sleep 1; {pids}; wait"
                    ),
                ]))
                .expect("the child starts"),
            ),
            "keep" => {
                // A call that leaves a process running on purpose: once it ended, the group is the person's.
                child::run(Run {
                    start: sh().args(["-c", &format!("sleep 600 > /dev/null 2>&1 & {pids}")]),
                    stdin: Vec::new(),
                    timeout: Duration::from_secs(30),
                    max_stdout: 1024,
                    max_stderr: 1024,
                    leftovers: Leftovers::Keep,
                })
                .await
                .expect("the call ends");
                None
            }
            "pipes" => {
                pipes_in_process().await;
                std::fs::write(dir.join("ready"), "").unwrap();
                return;
            }
            other => panic!("{ROLE}={other}"),
        };
        std::fs::write(dir.join("ready"), "").unwrap();
        tokio::time::sleep(Duration::from_secs(600)).await;
    }

    /// Whether `pid` still runs (a zombie, ended but not yet reaped by whoever adopted it, does not).
    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only asks whether the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => state(&stat).is_some_and(|s| s.first() != Some(&"Z")),
            // No /proc (macOS): the process exists.
            Err(_) => true,
        }
    }

    /// The fields of a `/proc/<pid>/stat` after the name (which is in parentheses and may hold spaces).
    fn state(stat: &str) -> Option<Vec<&str>> {
        stat.rsplit_once(')')
            .map(|(_, rest)| rest.split_whitespace().collect())
    }

    /// The command lines of the processes of `group` (Linux; empty elsewhere).
    fn group_commands(group: i32) -> Vec<String> {
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        dir.filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
            .filter(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|s| state(&s).and_then(|f| f.get(2)?.parse::<i32>().ok()))
                    == Some(group)
            })
            .filter_map(|pid| std::fs::read(format!("/proc/{pid}/cmdline")).ok())
            .map(|c| String::from_utf8_lossy(&c).replace('\0', " "))
            .collect()
    }

    fn wait_for(path: &Path, what: &str) {
        let end = Instant::now() + Duration::from_secs(30);
        while !path.exists() {
            assert!(Instant::now() < end, "{what} did not appear");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The app's process: killed when the case ends early, so a failed case leaves no app (and no pipe of the test
    /// run) behind.
    struct App(std::process::Child);

    impl Drop for App {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Start this program as the app with `role` in `mode`, until the call or the child is under way.
    fn start_app(dir: &Path, role: &str, mode: &str) -> App {
        let app = std::process::Command::new(std::env::current_exe().unwrap())
            .env(APP, dir)
            .env(ROLE, role)
            .env(MODE, mode)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        // Both: `ready` says the start (or the call) returned, `pids` that the child wrote; either may come first.
        let app = App(app);
        wait_for(&dir.join("ready"), "the app's child");
        app
    }

    /// The two process ids the child wrote: its own (the group) and the grandchild's.
    fn pids(dir: &Path) -> (i32, i32) {
        wait_for(&dir.join("pids"), "the child's process ids");
        let text = std::fs::read_to_string(dir.join("pids")).unwrap();
        let pids: Vec<i32> = text
            .split_whitespace()
            .map(|p| p.parse().unwrap())
            .collect();
        assert_eq!(pids.len(), 2, "{text:?}");
        (pids[0], pids[1])
    }

    /// SIGKILL: nothing of the app runs any more, no drop, no cleanup.
    fn kill(mut app: App) {
        app.0.kill().unwrap();
        app.0.wait().unwrap();
    }

    fn all_gone(pids: &[i32], what: &str) {
        let end = Instant::now() + Duration::from_secs(10);
        while pids.iter().any(|p| alive(*p)) {
            let left: Vec<_> = pids.iter().filter(|p| alive(**p)).collect();
            assert!(Instant::now() < end, "{what}: still running {left:?}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn killed(dir: &Path, mode: &str) {
        let app = start_app(dir, "spawn", mode);
        let (child, grandchild) = pids(dir);
        // The watcher is the app's own program with its argument, or (fork) a copy of the app.
        let commands = group_commands(child);
        if cfg!(target_os = "linux") {
            let loaded = commands.iter().any(|c| c.contains(ARG));
            assert_eq!(
                loaded,
                mode == "exec",
                "{mode}: the processes of the group {commands:?}"
            );
        }
        kill(app);
        all_gone(&[child, grandchild], "after the app was killed");
    }

    fn released(dir: &Path, mode: &str) {
        let app = start_app(dir, "keep", mode);
        let (_, left) = pids(dir);
        kill(app);
        std::thread::sleep(Duration::from_secs(1));
        let lives = alive(left);
        // SAFETY: the process the call left running on purpose, ended by the test that made it.
        unsafe { libc::kill(left, libc::SIGKILL) };
        assert!(
            lives,
            "{mode}: the process the call kept was ended with the app"
        );
    }

    fn kill_zero(dir: &Path, mode: &str) {
        let app = start_app(dir, "kill-zero", mode);
        let (child, grandchild) = pids(dir);
        assert!(
            alive(child) && alive(grandchild),
            "{mode}: the child ignores the signals"
        );
        kill(app);
        all_gone(
            &[child, grandchild],
            "after the child's kill 0 and the app was killed (the watcher died of the signals)",
        );
    }

    fn pipes(dir: &Path, mode: &str) {
        let mut app = start_app(dir, "pipes", mode);
        let status = app.0.wait().unwrap();
        assert!(status.success(), "{mode}: {status}");
    }

    /// The watcher of a second child is made while the app holds the stdin of the first: it does not hold it, so the
    /// first child still sees the end of its input when the app closes its end.
    async fn pipes_in_process() {
        let mut first = child::spawn(&sh().args(["-c", "cat > /dev/null"])).unwrap();
        let stdin = first.take_stdin().unwrap();
        let second = child::spawn(&sh().args(["-c", "sleep 600"])).unwrap();
        drop(stdin);
        let status = tokio::time::timeout(Duration::from_secs(10), first.wait()).await;
        drop(second);
        let status = status.expect("the first child did not see the end of its input");
        assert!(status.unwrap().success());
    }
}
