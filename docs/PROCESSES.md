# Starting processes

An app built on the kit does its work in its own process where it can. Where it needs another
program (the Neovim engine, `git`, `gh`), it starts it through one of two helpers and never by
hand. No app opens a PowerShell, `cmd` or console window for its work, and no process stays
behind when the app is gone.

This is not about hiding processes. A process that is merely invisible is what an endpoint
protection looks for. A child of an app is visible as what it is: it runs under its own program
name, it is tied to the app by the system, its start is in the app's log, and it is listed in
the app's documentation.

## Three ways, and when each applies

The kit used to have two ways that seemed to contradict each other: `proc::run` started a
child with `CREATE_NO_WINDOW`, and `launch::spawn` says "never `CREATE_NO_WINDOW`". Both are
right, for different programs. What decides is who owns the process:

| | The program | How | Window | Lifetime |
|---|---|---|---|---|
| (a) | **a child of the app**: it does work for the app (an engine call, an engine host, `git`, `gh`) | `appkit_core::child`: `run`, `run_lines`, `spawn` | none: it has nothing to show, its streams are pipes of the app | bound to the app by the system (job object, process group); time and size limits; named in the log |
| (b) | **a program that is not the app's**: another app, a server in its console, an editor the person works in | `appkit_hub::launch::spawn` | its own, visible; never `CREATE_NO_WINDOW` or `DETACHED_PROCESS` | outlives the app on purpose; nothing of the app is inherited |
| (c) | **something the system opens**: a link in the default browser | `appkit_hub::browser::open`: Windows `ShellExecuteW`, no program is started; macOS and Linux `/usr/bin/open` and `/usr/bin/xdg-open`, started the way (b) starts a program (it may become the browser) | the browser's | no process of the app at all |

So a process without a window is always one that is bound to the app and ends with it, and a
process that outlives the app always has a window of its own or is the person's own program.
An invisible process that stays is what neither way allows.

A hub that has no console itself (a program of the GUI subsystem, a hub in the background)
makes this visible: a console program it starts by hand gets a console of its own and a window
flashes up. Started through `child` it gets none; `tests/child.rs` of `appkit-testkit` shows
both.

Everything else is a finding of [`tools/check-process-starts.py`](#the-check).

### Named exceptions

These starts are written by hand on purpose and stay; the allow file of the check names them:

- **Neovim as the editor**, in a visible console of its own (`appkit-hub/src/editor.rs`,
  `CREATE_NEW_CONSOLE`): a window the person asked for and works in.
- **Another app as a server in its console** (`launch::spawn` with `Window::NewConsole`): the
  console is how that app is operated.
- **The hub in the background** (`background::start`): the hub puts itself in the background.
  It is not a child of an app; when it ends is its own matter (see the README, "A hub in the
  background ends itself").

## A child of the app (`appkit_core::child`)

```rust
use std::time::Duration;
use appkit_core::child::{self, Environment, Leftovers, Run, Start};

// A call that ends by itself.
let out = child::run(Run {
    start: Start::new("tasks-hub git status", git)   // who asks, and an absolute path
        .args(["status", "--porcelain"])
        .cwd(&repo)
        .env(Environment::Only(vec![("HOME".into(), home.into())])),
    stdin: Vec::new(),
    timeout: Duration::from_secs(10),
    max_stdout: 1024 * 1024,
    max_stderr: 64 * 1024,
    leftovers: Leftovers::End,
})
.await?;

// A call whose output is shown while it runs: every line as it arrives.
let code = child::run_lines(run, |stream, line| progress.push(stream, line)).await?;

// A child that stays.
let mut host = child::spawn(&Start::new("tasks-hub engine host", nvim).args(args))?;
let stdin = host.take_stdin();
let stdout = host.take_stdout();
let stderr = host.take_stderr();   // read both, or the child stops when a pipe is full
// ...
host.end().await;                  // or drop it
```

Both need a tokio runtime.

What every child gets:

- **No console window.** On Windows the child is started with `CREATE_NO_WINDOW`. Its three
  standard streams are pipes of the app; it gets no console, terminal or log handle of its
  parent. Windows still gives a console program a console without a window, so a `conhost.exe`
  runs beside the child and ends with it.
- **It ends with the app.** On Windows the child is put into a job object with
  `KILL_ON_JOB_CLOSE`. The app holds the only handle; when the app ends, crashes or is killed,
  the system closes the job and ends the child and everything it started. On Linux and macOS
  the child leads a process group of its own, and the group is killed when the `Owned` is
  dropped, when the time is over and when the output is too large. On Linux the child also gets
  the parent-death signal (`PR_SET_PDEATHSIG`, `SIGKILL`), so the system ends it when the app
  is killed.
- **An argv, never a shell line.** The program is an absolute path to a file on this machine
  (no name searched on `PATH`, no network share, no `..`), the arguments reach it one by one
  as they are. Nothing is quoted, joined or split.
- **Its own name.** The program runs under the name it has on disk. It is never copied,
  renamed or wrapped in a script host to look like something else.
- **A line in the log.** `child started by <caller>: <program> (pid <n>)` is written for every
  child that stays and for the first call of each pair of caller and program (a hub asks its
  engine thousands of times, and its log is not cut while it runs). A child that had to be
  ended always writes `child of <caller>: <program> (pid <n>) was ended: <why>`. The lines go
  to stderr, which is `hub.log` for a hub in the background; `child::set_log` names another
  place. The arguments are not logged: they can hold paths and data of the person.
- **Limits** (`run`, `run_lines`): one time limit for the whole call and a size limit per
  output stream (for `run_lines`: all bytes of the stream, nothing is kept).
  When one is passed, the child and what it started are ended and the call fails with
  `ProcError::Timeout` or `ProcError::TooLarge`.
- **An environment that is named.** `Environment::Only(list)` is the whole environment;
  `Environment::Inherited { set, remove }` is the app's with changes. The default is empty.

`Leftovers` says what happens to a process the child started and left running when it ended by
itself: `End` (the usual case, nothing stays behind a call) or `Keep` (the child started it on
purpose for the person).

`Start::as_given` takes a program path as it is. It exists for the path of Neovim a person put
into a settings file; new code uses `Start::new`.

`appkit_core::proc::run` is the older face of the same code (empty environment, path as given,
`Leftovers::Keep`, no caller name). It stays for the apps that call it.

`appkit_hub::background::start_from` (`install` brings the hub back from the installed program)
is such a call: `<installed program> start` is a child while it runs, and the hub it puts in the
background stays (`Leftovers::Keep`). The log line of the child goes to stderr, so a person who
runs `install` in a terminal sees it there.

### Limits

- Windows: the child runs for a moment before it is in its job. A process it starts in that
  first moment is not in the job.
- Linux: when the app is killed, the parent-death signal ends the child itself. What the child
  started is not signalled; it ends when it follows its parent (a pipe that closes). While the
  app lives, the whole group is ended.
- macOS has no parent-death signal. A child ends with the app when the app ends in an orderly
  way (the runtime is shut down, the `Owned` is dropped). After a crash of the app the child
  ends when it next reads or writes one of its pipes: an engine host that waits for a question
  ends at once, a program that only computes runs to its own end.
- A process that leaves its group (`setsid`) or its job on purpose is not ended.
- With `Leftovers::Keep`, a process the child left running that holds one of the child's pipes
  keeps the call waiting until the time is over.

### The folder dialog

`FileOpener::pick_folder` of `appkit_hub::editor` asks the person for a folder. On Windows the
hub shows the dialog itself (`IFileOpenDialog` with `FOS_PICKFOLDERS`, on a thread of its own):
no program is started, and in particular no PowerShell with a script in its arguments. macOS and
Linux have no such call without a toolkit of windows in the hub, so there the dialog program of
the system stays (`/usr/bin/osascript`; `zenity` or `kdialog` from `PATH`), as a child of the hub
through `child::run`: bound to the hub, named in the log, closed after thirty minutes.

### No program for what the system does itself

A program of the system is not started where the system has a call for the same thing:

- **Stopping the hub** (`background::stop`): the hub is asked to end itself over its control
  route (`POST /api/control/stop`, with the control secret); an app that waits for
  `background::left_alone` ends by that, also a hub in a terminal. A hub that does not end
  within ten seconds, or that cannot be asked (an older version), is ended by the system:
  `OpenProcess` and `TerminateProcess` on Windows, `kill(2)` with `SIGTERM` elsewhere, on exactly
  the process of `hub.json` that answered as this hub. No `taskkill.exe`, no `/bin/kill`.
- **Showing a folder** (`FileOpener::reveal`): Windows hands the folder to the shell
  (`ShellExecuteW`, verb `explore`, a folder only), no `explorer.exe` is started by the hub.
  macOS and Linux start `/usr/bin/open` and `xdg-open` the way (b) starts a program.

## A program of somebody else (`appkit_hub::launch`)

A hub that starts another app does not own it: the other app stays when the hub ends. Such a
start goes through `launch::spawn(program, args, &LaunchOpts { cwd, window })`: an absolute
path and an argv, the null device as stdin, stdout and stderr, the standard handles of the hub
made not inheritable first, a process group of its own, and never `CREATE_NO_WINDOW` or
`DETACHED_PROCESS`. A program that is a server in a console gets a visible console of its own
(`Window::NewConsole`).

## A hub is a window program

A console program gets a console window from Windows whenever it is not started from a terminal:
the link (`<hub> start --open`), the login entry (`<hub> start`) and a double click each showed
one. So the release build of every hub is a **window program** (GUI subsystem), and
`appkit_core::console` gives it back what a console program has for free:

```rust
// main.rs of the hub: a debug build (`cargo run`) stays a console program.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use appkit_core::console::{self, Console};

#[tokio::main]
async fn main() {
    // Before the first line is written.
    let console = console::attach_parent();
    console::report_panics("tasks-hub");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let by_click = args.is_empty();
    if let Err(message) = Box::pin(run(args)).await {
        console::report("tasks-hub", &message); // stderr, or a message box when nobody reads it
        if by_click && console == Console::Own {
            wait_for_enter(); // a console a double click opened (a debug build) closes with the program
        }
        std::process::exit(1);
    }
}
```

What `attach_parent` does, by how the program was started:

| Started | What it gets | Console |
|---|---|---|
| from a terminal (PowerShell, `cmd`, Windows Terminal) | it attaches to the terminal's console; `status`, `--help`, errors are written there | `Parent` |
| with all three streams going somewhere (a pipe, a file, the null device: a script that captures the output, a test, the hub `start` puts in the background) | nothing is attached; the output goes where it was sent | `None` |
| by a link, the login entry, a double click | no console, no window | `None` |
| a debug build, every program on macOS and Linux | its own console, as before | `Own` |

A stream that goes somewhere is never replaced by the console: `tasks-hub status > out.txt` from a
terminal writes the file and still shows its errors in the terminal. A program whose three
streams all go somewhere does not attach at all: the hub in the background (`start` hands it
`hub.log` and the null device) would otherwise end when the terminal it was started from is
closed.

**The shell does not wait for a window program.** PowerShell and `cmd` show the prompt at once;
the output of the hub follows after it, and `$LASTEXITCODE` and `%ERRORLEVEL%` are not the
hub's. A script that needs the exit code or the output waits for the program: `Start-Process -Wait
-NoNewWindow -PassThru` (its `ExitCode`), or it pipes the call (`$out = (& tasks-hub status |
Out-String)`, `& tasks-hub install | Out-Host`), for which PowerShell 7 and Windows PowerShell 5.1
wait. A bare assignment does not: `$out = (& tasks-hub status)` closes the pipe while the program
still runs (it dies writing to it) and leaves `$LASTEXITCODE` unset, as `install.ps1` of the
release kit did before `v0.56.1` (`appkit-testkit/tests/install_ps1.rs`). A `.cmd` script waits by
itself. Closing the terminal ends
a hub that runs in it (`serve`); `start` puts the hub in the background, which outlives it.

**Questions on stdin go to a window of their own.** In the terminal the shell reads the same
keys as the hub, and without a terminal there is nobody to ask. The questions of a first start
(`appkit_hub::setup`, a hub run without arguments that is not set up yet) are asked in a console
window the program opens only for them (`console::open_own`) and closes when they are answered
(`console::close_own`); from a terminal a line there says so. Answers piped in (a spec with
`<PREFIX>_ASSUME_TERMINAL`) are still read from the pipe. A hub that is set up asks nothing
and opens nothing. A double click on a hub that is not set up gets that window; one that is set up
starts the hub and opens the page.

**Without a terminal nobody reads stderr.** `console::report` writes an error to stderr and, when
stderr goes nowhere, shows it in a message box: a link whose hub fails to start says why instead
of ending without a word. `console::report_panics` does the same for a panic. The box waits for
the person, so it is for the end of a command; the hub in the background writes to `hub.log`.

**Console programs the hub starts.** A window program has no console that a console program it
starts could share, so such a child gets one of its own, with a window. Every child goes through
`appkit_core::child`, which starts it with `CREATE_NO_WINDOW`; the system tools the kit still
starts by hand (`reg.exe` for the login entry, `icacls.exe` for the control secret) carry the same
flag. Programs of somebody else (`launch`) and Neovim in its console are visible on purpose.

The link keeps its `SW_SHOWMINNOACTIVE` (7) for apps that are still console programs; a window
program has no window for it to minimize.

An app tests the attribute on its release build (`cargo test --release` in its CI): the header of
the program says which subsystem it is (`appkit_testkit::window_program::subsystem`, `WINDOW` or
`CONSOLE`). appkit tests the behaviour with a window program of its own
(`appkit-testkit/src/bin/window-probe.rs`, `tests/window_program.rs`): the streams, the exit
code, the console of a parent with and without one, and (by hand, it shows a window) the window
for the questions.

## The check

`tools/check-process-starts.py` reads every tracked `*.rs` file and fails on a line that starts
a process itself: `Command::new`, `.creation_flags(`, `CreateProcess…`, `ShellExecute…`. It
does not look at comment lines, at files in a `tests`, `benches` or `examples` folder, at
`build.rs`, or at the rest of a file from a `#[cfg(test)]` module on. It does not see a
`Command` that was imported under another name.

A file that may start processes itself is listed in `tools/process-starts.allow`:

```
<path> <count> <reason>
```

`<count>` is the number of lines found in the file. More or fewer lines than the entry says
fail, and so does an entry for a file without any, so the list can neither hide a new start nor
keep an exception that is gone. In this repository the list holds the two helpers, the named
exceptions above, the link handed to the browser, and the calls of system tools (`icacls`,
`reg`) that are being replaced.

The script knows no repository. An app copies it into its own `tools/`, starts with an allow
file that lists what it has today, and removes the entries as the starts move to the helpers:

```
python tools/check-process-starts.py
```

## What an app documents

Each app lists, in its own documentation, the programs it starts: the name, why, how long the
process lives, and which helper starts it. A process that is not in that list is a bug.
