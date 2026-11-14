# Starting processes

An app built on the kit does its work in its own process where it can. Where it needs another
program (the Neovim engine, `git`, `gh`), it starts it through one of two helpers and never by
hand. No app opens a PowerShell, `cmd` or console window for its work, and no process stays
behind when the app is gone.

This is not about hiding processes. A process that is merely invisible is what an endpoint
protection looks for. A child of an app is visible as what it is: it runs under its own program
name, it is tied to the app by the system, its start is in the app's log, and it is listed in
the app's documentation.

## Which helper

| The program | Helper | What it gives |
|---|---|---|
| does work for the app and gives its result back (an engine call, `git status`, a version probe) | `appkit_core::child::run` | output collected, time limit, size limit |
| serves the app as long as it runs (an engine host) | `appkit_core::child::spawn` | an `Owned` with the three pipes |
| belongs to somebody else and is meant to stay (another app, a hub) | `appkit_hub::launch::spawn` | nothing inherited, a window of its own when asked for |
| is the person's browser | `appkit_hub::browser::open` | only `http://127.0.0.1:<port>/…` |

Everything else is a finding of [`tools/check-process-starts.py`](#the-check).

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
- **Limits** (`run`): one time limit for the whole call and a size limit per output stream.
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
keep an exception that is gone. In this repository the list holds the two helpers, the places
that hand something to a program of the person (browser, editor, file manager, folder dialog),
the hub putting itself in the background, and two calls of system tools that have not moved to
the helper yet.

The script knows no repository. An app copies it into its own `tools/`, starts with an allow
file that lists what it has today, and removes the entries as the starts move to the helpers:

```
python tools/check-process-starts.py
```

## What an app documents

Each app lists, in its own documentation, the programs it starts: the name, why, how long the
process lives, and which helper starts it. A process that is not in that list is a bug.
