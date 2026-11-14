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

## A program of somebody else (`appkit_hub::launch`)

A hub that starts another app does not own it: the other app stays when the hub ends. Such a
start goes through `launch::spawn(program, args, &LaunchOpts { cwd, window })`: an absolute
path and an argv, the null device as stdin, stdout and stderr, the standard handles of the hub
made not inheritable first, a process group of its own, and never `CREATE_NO_WINDOW` or
`DETACHED_PROCESS`. A program that is a server in a console gets a visible console of its own
(`Window::NewConsole`).

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
`reg`, `taskkill`, the file manager, the folder dialog) that are being replaced.

The script knows no repository. An app copies it into its own `tools/`, starts with an allow
file that lists what it has today, and removes the entries as the starts move to the helpers:

```
python tools/check-process-starts.py
```

## What an app documents

Each app lists, in its own documentation, the programs it starts: the name, why, how long the
process lives, and which helper starts it. A process that is not in that list is a bug.
