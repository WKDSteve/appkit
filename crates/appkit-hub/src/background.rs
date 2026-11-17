//! The hub as a background program: `start`, `stop` and the start with the
//! login (`autostart`).
//!
//! `start` runs `serve` again as a program of its own that has no window and
//! does not end with the terminal; its output goes to `hub.log` in the config
//! folder. A hub is only believed (and only stopped) when the process on its
//! port says the instance of `hub.json`: a stale file never makes `stop` kill
//! a process that merely got the same number. `stop` asks the hub to end itself
//! and ends it by a call of the system when it does not; no program is started.
//!
//! A hub in the background has no window, so nobody would notice one that is left over. It ends
//! by itself once no page has been connected for a while ([`left_alone`]), unless the person gave
//! it an order to stay ([`has_order`]): the start with the login, `keep_running` in the settings,
//! or a paired phone. A hub that runs in a terminal (`serve`) ends with Ctrl-C as it always did.

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use appkit_core::child::{self, Environment, Leftovers, Run, Start, Stream};
use appkit_core::family;
use appkit_core::identity::AppIdentity;

use crate::env::Env;
use crate::instance;
use crate::loginitems::{self, LoginApp};
use crate::presence::Presence;
use crate::process::Process;

/// What the commands need to know about the app: its names and the files it keeps besides the kit's.
#[derive(Debug, Clone, Copy)]
pub struct Background<'a> {
    pub login: LoginApp<'a>,
    /// The files of the app in the config folder that `uninstall --purge-settings` removes in
    /// addition to those of the kit (`config.json`, `usage.json`, `hub.json`, `control.secret`,
    /// `hub.log`). Nothing else is ever removed there.
    pub extra_files: &'a [&'a str],
    /// What `uninstall` tells the person to run to delete the program file, which a running
    /// program cannot delete (`scripts/uninstall`): the script is the app's, shipped with it.
    pub uninstall_script: &'a str,
}

impl Background<'_> {
    fn identity(&self) -> &AppIdentity {
        self.login.identity
    }

    /// The name of the value under the `Run` key of the registry (what earlier versions wrote; it is only read and
    /// removed now) and of the link in the Startup folder.
    fn run_value(&self) -> &str {
        self.identity().app
    }
}

/// The `Run` key of the current user (no administrator rights needed).
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
/// The log is cut at the start when it is larger than this.
const LOG_LIMIT: u64 = 256 * 1024;
/// How long a hub in the background stays when no page is connected and it has no order to stay.
const ALONE_GRACE: Duration = Duration::from_secs(10 * 60);
/// What `start` prints first when it started a hub (and not when it found one running).
const STARTED: &str = "the hub runs in the background";

/// The variable `start` sets for the hub it puts in the background: that hub has no terminal and
/// no window, so it ends itself when it is left alone.
fn detached_var(app: &Background<'_>) -> String {
    app.identity().var("DETACHED")
}

/// Whether this process is a hub that `start` put in the background.
#[must_use]
pub fn runs_detached(app: &Background<'_>, env: &Env) -> bool {
    env.get(&detached_var(app)).is_some()
}

/// How long a hub in the background stays without a page: ten minutes, or the seconds the variable
/// `<APP>_ALONE_SECS` names (1 to 86400; anything else is ignored).
#[must_use]
pub fn alone_grace(app: &Background<'_>, env: &Env) -> Duration {
    env.get(&app.identity().var("ALONE_SECS"))
        .and_then(|v| v.to_str()?.trim().parse::<u64>().ok())
        .filter(|secs| (1..=86_400).contains(secs))
        .map_or(ALONE_GRACE, Duration::from_secs)
}

/// Whether the person gave the hub an order to run without a page: `keep_running` of the settings,
/// a paired device (a phone reaches the hub when no page is open on the PC), the start with the
/// login, or the starter said so ([`stay_var`]: hub-desktop for the hubs it starts with itself). The
/// login entries of the machine are not asked when the tests of the program say they are not theirs
/// (`NO_LOGIN_SYNC`).
#[must_use]
pub fn has_order(
    app: &Background<'_>,
    env: &Env,
    keep_running: bool,
    paired_devices: usize,
) -> bool {
    keep_running
        || paired_devices > 0
        || env.get(&stay_var(app.identity())).is_some()
        || (env.get(&no_login_sync(app)).is_none()
            && crate::loginstate::is_on(app.identity(), env, &crate::loginstate::SystemLogin)
                == Some(true))
}

/// The variable a starter sets for `<program> start` when the hub it puts in the background is to
/// stay without a page (`<APP>_STAY`, any value): hub-desktop does for the hubs of `start_with_hub`.
/// `start` passes its environment on to the hub, so the hub sees it in [`has_order`].
#[must_use]
pub fn stay_var(identity: &AppIdentity) -> String {
    identity.var("STAY")
}

/// Resolves when a hub should end itself. A hub in the background: no page has been connected for
/// the grace time and `stay` (see [`has_order`]) says no. Every hub, also one that has a terminal
/// (which the person there ends otherwise): `<program> stop` asked it to end (see [`stop`]).
pub async fn left_alone<F: Fn() -> bool>(
    app: &Background<'_>,
    env: &Env,
    presence: &Presence,
    stay: F,
) {
    if !runs_detached(app, env) {
        return presence.asked_to_end().await;
    }
    presence.left_alone(alone_grace(app, env), stay).await;
}

/// Whether what `start` (or [`start_from`]) printed says that it started a hub now, as opposed to
/// finding one that ran already. Who started a hub for a window ends it when the window never opens.
#[must_use]
pub fn started_now(printed: &str) -> bool {
    printed.starts_with(STARTED)
}

/// `autostart on | off | status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autostart {
    On,
    Off,
    Status,
}

/// The command that is registered: the program (quoted) and `start`.
#[must_use]
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\" start", exe.display())
}

fn log_file(dir: &Path) -> Result<std::fs::File, String> {
    let path = dir.join("hub.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Start a hub in the background unless one runs. Returns what to print.
///
/// # Errors
///
/// A text for the person at the terminal.
pub async fn start(app: &Background<'_>) -> Result<String, String> {
    start_as(app, "serve", Program::Console).await
}

/// What kind of program the hub is, for [`start_as`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    /// A console program (a debug build, or a hub that is not a window program yet): it is started
    /// with a console that has no window (`CREATE_NO_WINDOW`), see [`start`].
    Console,
    /// A window program (GUI subsystem): Windows opens no console for it, so nothing has to be
    /// hidden and it is started as any program a person starts, in a process group of its own.
    /// Such a hub shows itself (a window, an icon in the notification area).
    Window,
}

/// [`start`] with the verb the program runs in the background (`serve`, or a verb of the app that
/// serves and shows itself, such as `tray`) and what kind of program it is. The verb is one plain
/// word of lower case letters and `-`.
///
/// # Errors
///
/// A text for the person at the terminal.
pub async fn start_as(
    app: &Background<'_>,
    verb: &str,
    program: Program,
) -> Result<String, String> {
    if !plain_verb(verb) {
        return Err(format!("not a verb of the program: {verb:?}"));
    }
    let env = Env::from_process();
    let dir = instance::config_folder(&env, app.identity())?;
    if let Some(record) = instance::running_hub(&dir, app.identity()).await {
        return Ok(format!(
            "a hub is already running (port {}, process {})\n",
            record.port, record.pid
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create the config folder: {e}"))?;
    if let Some(warning) = private_storage_warning(&env, &dir) {
        eprintln!("{warning}");
    }
    let exe = std::env::current_exe().map_err(|e| format!("own program not found: {e}"))?;
    remove_old_start_script(&dir);
    let (child, log_from) = spawn_detached(app, &exe, &dir, verb, program)?;
    wait_for_hub(app, &dir, child, log_from, start_limit(app, &env)).await
}

/// How long `start` waits for the hub to come up: 20 seconds, or what `<APP>_START_SECS` names (1 to 3600;
/// anything else is ignored), for the tests of an app on a machine under load.
#[must_use]
pub fn start_limit(app: &Background<'_>, env: &Env) -> Duration {
    env.get(&app.identity().var("START_SECS"))
        .and_then(|v| v.to_str()?.trim().parse::<u64>().ok())
        .filter(|secs| (1..=3600).contains(secs))
        .map_or(START_LIMIT, Duration::from_secs)
}

/// How long `start` waits for the hub by default.
const START_LIMIT: Duration = Duration::from_secs(20);
/// How long `start` still looks for a hub after the one it started failed: a hub started at the same time
/// by someone else (a second double click) may be the one that comes up.
const FAILED_GRACE: Duration = Duration::from_secs(2);

/// Waits until the hub says it is there (its record and its answer). A hub that ends before that (with a
/// failure or with exit code 0) is reported at once, with the last line it wrote to `hub.log` after `log_from`: a broken settings
/// file ends it in a moment, and nobody should wait the whole limit for a message that does not say why.
async fn wait_for_hub(
    app: &Background<'_>,
    dir: &Path,
    mut child: std::process::Child,
    log_from: u64,
    limit: Duration,
) -> Result<String, String> {
    let log = dir.join("hub.log");
    let mut deadline = Instant::now() + limit;
    let mut failed = None;
    loop {
        if let Some(record) = instance::running_hub(dir, app.identity()).await {
            return Ok(format!(
                "{STARTED} (port {}, process {})\n",
                record.port, record.pid
            ));
        }
        // Any end counts, also one with exit code 0: a `stop` that came first, or a window hub that was quit,
        // brings no hub either, and nobody should wait the whole limit for it.
        if failed.is_none()
            && let Ok(Some(status)) = child.try_wait()
        {
            failed = Some(status);
            deadline = deadline.min(Instant::now() + FAILED_GRACE);
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    match failed {
        Some(status) => {
            let cause = last_log_line(&log, log_from).unwrap_or_else(|| {
                status.code().map_or_else(
                    || format!("it ended with {status}"),
                    |code| format!("it ended with exit code {code}"),
                )
            });
            Err(format!(
                "the hub ended before it came up: {cause}; see {}",
                log.display()
            ))
        }
        None => Err(format!(
            "the hub did not come up within {} seconds; see {}",
            limit.as_secs(),
            log.display()
        )),
    }
}

/// The last line in `log` from byte `from` on that says something (not a note of the runtime such as
/// "note: run with `RUST_BACKTRACE=1`"), without control characters and cut to 300 characters.
fn last_log_line(log: &Path, from: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 64 * 1024;
    let mut file = std::fs::File::open(log).ok()?;
    let len = file.metadata().ok()?.len();
    // A log cut by another start in between is read from its beginning.
    let from = if from > len { 0 } else { from };
    file.seek(SeekFrom::Start(from.max(len.saturating_sub(TAIL))))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let line = text
        .lines()
        .map(|l| l.chars().filter(|c| !c.is_control()).collect::<String>())
        .map(|l| l.trim().to_owned())
        .rfind(|l| !l.is_empty() && !l.starts_with("note: ") && l != "stack backtrace:")?;
    let mut cut: String = line.chars().take(300).collect();
    if cut.len() < line.len() {
        cut.push_str("...");
    }
    Some(cut)
}

/// Whether `verb` is one plain word (`serve`, `tray`): nothing a program could take as an option.
fn plain_verb(verb: &str) -> bool {
    !verb.is_empty()
        && verb.len() <= 32
        && !verb.starts_with('-')
        && verb.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

/// Run the verb (`serve`) as a program of its own: no console window, its own process group, the
/// output in the log. On Windows the standard handles of this process are made not inheritable
/// first, so the hub does not hold the pipe of a caller (`<program> start | ...`) open (see
/// `handles`). It is started directly, not through the script host: an endpoint protection stops a
/// program that starts `wscript.exe`.
fn spawn_detached(
    app: &Background<'_>,
    exe: &Path,
    dir: &Path,
    verb: &str,
    program: Program,
) -> Result<(std::process::Child, u64), String> {
    #[cfg(windows)]
    crate::handles::keep_standard_handles();
    let log = log_file(dir)?;
    let log_from = log.metadata().map_or(0, |m| m.len());
    let err = log
        .try_clone()
        .map_err(|e| format!("cannot write the log: {e}"))?;
    let mut command = detached_command(app, exe, verb);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    detach(&mut command, program);
    let child = command
        .spawn()
        .map_err(|e| format!("the hub could not be started: {e}"))?;
    Ok((child, log_from))
}

/// The program, the one verb and the variable that says "started by `start`": nothing else.
fn detached_command(app: &Background<'_>, exe: &Path, verb: &str) -> Command {
    let mut command = Command::new(exe);
    command.arg(verb).env(detached_var(app), "1");
    command
}

/// The Windows creation flags of a hub in the background.
///
/// `CREATE_NEW_PROCESS_GROUP` always: a Ctrl-C of the terminal does not reach it.
/// `CREATE_NO_WINDOW` for a console program only: a console of its own that has no window, not the
/// one of the terminal (which may close) and not none (`DETACHED_PROCESS`): a hub with no console
/// makes every console program it runs by hand create one, which flashes a window and fails where
/// no console can be made (`STATUS_DLL_INIT_FAILED`). A window program gets no console from
/// Windows anyway; the flag would only hide what is not there, so it is not given.
#[must_use]
pub fn detach_flags(program: Program) -> u32 {
    const NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const NO_WINDOW: u32 = 0x0800_0000;
    match program {
        Program::Console => NEW_PROCESS_GROUP | NO_WINDOW,
        Program::Window => NEW_PROCESS_GROUP,
    }
}

#[cfg(windows)]
fn detach(command: &mut Command, program: Program) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(detach_flags(program));
}

#[cfg(unix)]
fn detach(command: &mut Command, _program: Program) {
    use std::os::unix::process::CommandExt;
    // Its own process group: a Ctrl-C of the terminal does not reach it.
    command.process_group(0);
}

#[cfg(not(any(windows, unix)))]
fn detach(_command: &mut Command, _program: Program) {}

/// How long a hub that was asked to end may take before it is ended the hard way.
const END_GRACE: Duration = Duration::from_secs(10);
/// How long a process that was ended the hard way may take to be gone.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// Stop the hub that runs (the one `hub.json` names and that answers as it).
///
/// The hub is asked to end itself first, over its control route (`/api/control/stop`, with the
/// control secret only this person can read): it closes what it has open and goes. A hub that
/// does not know the route (an older one), does not listen for it or takes too long is ended by
/// the system (`TerminateProcess`, `kill(2)` with `SIGTERM`): exactly the process of `hub.json`
/// that answered as this hub, and no program is started for it.
///
/// # Errors
///
/// A text for the person at the terminal.
pub async fn stop(app: &Background<'_>) -> Result<String, String> {
    let env = Env::from_process();
    let dir = instance::config_folder(&env, app.identity())?;
    stop_in(app.identity(), &dir, END_GRACE).await
}

/// [`stop`] for the hub of `identity` whose files are in `dir`; a hub that was asked to end has
/// `grace` to do so.
///
/// # Errors
///
/// A text for the person at the terminal.
pub async fn stop_in(
    identity: &AppIdentity,
    dir: &Path,
    grace: Duration,
) -> Result<String, String> {
    let Some(record) = instance::running_hub(dir, identity).await else {
        return Ok("no hub is running\n".to_owned());
    };
    let process = Process::open(record.pid)?;
    // The process is held now (Windows: its id is not given away while the handle is open). That
    // it is the hub is asked once more, so that a process which got the id between the answer
    // and the opening is never ended.
    let still = instance::running_hub(dir, identity).await;
    if still.is_none_or(|now| now.pid != record.pid || now.instance != record.instance) {
        return Ok("no hub is running\n".to_owned());
    }
    let pid = process.pid();
    if ask_to_end(dir, record.port).await {
        gone_within(&process, grace).await;
    }
    if !process.is_gone() {
        process.end()?;
        if !gone_within(&process, KILL_GRACE).await {
            return Err(format!("the process {pid} could not be stopped"));
        }
    }
    // The record of a hub that was ended the hard way stays behind; take it away.
    let _ = std::fs::remove_file(dir.join("hub.json"));
    let _ = std::fs::remove_file(dir.join("control.secret"));
    Ok(format!("the hub (process {pid}) is stopped\n"))
}

/// Ask the hub on `port` to end itself. False: it could not be asked (no secret, an older hub
/// without the route, no answer), so there is nothing to wait for.
async fn ask_to_end(dir: &Path, port: u16) -> bool {
    let Ok(Some(secret)) = appkit_core::registry::read_secret(dir) else {
        return false;
    };
    let auth = format!("Bearer {secret}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("Content-Type", "application/json"),
    ];
    crate::client::request(
        port,
        "POST",
        crate::guard::CONTROL_STOP_PATH,
        &headers,
        "{}",
    )
    .await
    .is_ok_and(|reply| reply.status == 200)
}

/// Wait until `process` is gone, for `limit` at most. Whether it is.
async fn gone_within(process: &Process, limit: Duration) -> bool {
    let end = Instant::now() + limit;
    while !process.is_gone() {
        if Instant::now() >= end {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    true
}

/// Start the hub with the login of the user (Windows: a link in the Startup folder; Linux: an
/// autostart entry; macOS: a launch agent).
///
/// # Errors
///
/// A text for the person at the terminal.
///
/// The start with the login is one state, which hub-desktop shows and switches too
/// ([`crate::loginstate`]): for an app that hub-desktop takes over, `on` puts it into the list of
/// hub-desktop while hub-desktop starts with the login, `off` takes it out of that list as well, and
/// `status` says which of the two starts it.
pub fn autostart(app: &Background<'_>, what: Autostart) -> Result<String, String> {
    let env = Env::from_process();
    // The login entry is in the registry, which a packaged app redirects whatever
    // the config folder is; the folder of AppData always exists to be probed.
    if let Some(warning) = env
        .path("LOCALAPPDATA")
        .and_then(|local| private_storage_warning(&env, &local))
    {
        eprintln!("{warning}");
    }
    let own_program = || std::env::current_exe().map_err(|e| format!("own program not found: {e}"));
    let said = autostart_with(
        app,
        what,
        own_program,
        &env,
        &crate::loginstate::SystemLogin,
    )?;
    // The own entry as the system shows it (the command it runs).
    if what == Autostart::Status
        && said.starts_with("on: its own")
        && let Ok(exe) = own_program()
        && let Ok(own) = autostart_for(app, Autostart::Status, &exe)
    {
        return Ok(own);
    }
    Ok(said)
}

/// [`autostart`] with the program the own entry starts, the environment and the login entries given.
///
/// # Errors
///
/// A text for the person at the terminal.
pub fn autostart_with(
    app: &Background<'_>,
    what: Autostart,
    program: impl FnOnce() -> Result<PathBuf, String>,
    env: &Env,
    login: &dyn crate::loginstate::LoginEntries,
) -> Result<String, String> {
    use crate::loginstate::{self, Via};
    match what {
        Autostart::On => loginstate::set(app.identity(), true, program, env, login),
        Autostart::Off => loginstate::set(app.identity(), false, program, env, login),
        Autostart::Status => match loginstate::state(app.identity(), env, login) {
            Some(Some(Via::Own)) => Ok("on: its own login entry\n".to_owned()),
            Some(Some(Via::HubDesktop)) => {
                Ok("on: hub-desktop starts it with your login (start_with_hub)\n".to_owned())
            }
            Some(None) => Ok("off\n".to_owned()),
            None => Err("the login entries of this system cannot be read".to_owned()),
        },
    }
}

/// [`autostart`] for the program at `exe` (what `install` registers).
///
/// # Errors
///
/// A text for the person at the terminal.
pub fn autostart_for(app: &Background<'_>, what: Autostart, exe: &Path) -> Result<String, String> {
    if !cfg!(windows) {
        let platform = loginitems::Platform::current().ok_or_else(|| {
            format!(
                "autostart is not built for this system (put `{} start` into its login items)",
                app.identity().binary
            )
        })?;
        let action = match what {
            Autostart::On => loginitems::Action::On,
            Autostart::Off => loginitems::Action::Off,
            Autostart::Status => loginitems::Action::Status,
        };
        return loginitems::login_entry(&app.login, platform, action, exe, &Env::from_process());
    }
    startup_entry(app, what, exe, &Env::from_process(), &SystemRegistry)
}

/// The start with the login on Windows: a link in the Startup folder of the user (`<app>.lnk`, it runs `<program>
/// start`), not a value in the `Run` key, which endpoint protections watch as a way to stay on a machine. A value an
/// earlier version wrote there still counts as on, and every change takes it away: on writes the link in its place,
/// off removes both. Only a link to this program is ours; a link of the person's own with that name stays.
fn startup_entry(
    app: &Background<'_>,
    what: Autostart,
    exe: &Path,
    env: &Env,
    reg: &dyn Registry,
) -> Result<String, String> {
    let dir = shell_folder(env, reg, &LinkPlace::Startup)?;
    let link = dir.join(format!("{}.lnk", app.identity().app));
    let program = format!("{}.exe", app.identity().binary);
    match what {
        Autostart::On => {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("the link in the Startup folder could not be made: {e}"))?;
            crate::shell_link::write(&link, &startup_link(app.identity().app, exe))
                .map_err(|e| format!("the link in the Startup folder could not be made: {e}"))?;
            let mut out = format!(
                "the hub starts with your login: {} start (link {})\n",
                exe.display(),
                link.display()
            );
            if let Err(e) = run_key_for(app, Autostart::Off, exe, reg) {
                out.push_str(&format!("the old login entry in the Run key stays: {e}\n"));
            }
            Ok(out)
        }
        Autostart::Off => {
            crate::shell_link::remove_if_target(&link, &program)?;
            run_key_for(app, Autostart::Off, exe, reg)
        }
        Autostart::Status => {
            if crate::shell_link::is_link_to(&link, &program) {
                return Ok(format!("on: {}\n", link.display()));
            }
            let old = run_key_for(app, Autostart::Status, exe, reg)?;
            Ok(if old.starts_with("on") {
                format!(
                    "{} (an old login entry in the Run key; switch it off and on to make the link)\n",
                    old.trim_end()
                )
            } else {
                old
            })
        }
    }
}

/// The link in the Startup folder: `<program> start`, minimized, in the folder of the program.
fn startup_link(app: &str, exe: &Path) -> crate::shell_link::Link {
    crate::shell_link::Link {
        arguments: "start".to_owned(),
        description: format!("{app}: start the hub with the login"),
        ..app_link(app, exe, LinkStart::OpenPage)
    }
}

/// Whether the program of another app starts with the login of the current user: its link in the
/// Startup folder, or an old value in the `Run` key, both named by the app (`identity.app`). Read only,
/// and never through a program of the other app.
///
/// `None`: it cannot be told (not Windows, or the registry could not be asked). A key or a value that is not
/// there, or that a policy hides, is `Some(false)`.
#[must_use]
pub fn autostart_of(identity: &AppIdentity) -> Option<bool> {
    if !cfg!(windows) {
        return None;
    }
    autostart_in(identity, &Env::from_process(), &SystemRegistry)
}

/// [`autostart_of`] with the environment and the registry given.
fn autostart_in(identity: &AppIdentity, env: &Env, reg: &dyn Registry) -> Option<bool> {
    let other = Background {
        login: LoginApp {
            identity,
            summary: "",
        },
        extra_files: &[],
        uninstall_script: "",
    };
    startup_entry(&other, Autostart::Status, Path::new(""), env, reg)
        .ok()
        .map(|said| said.starts_with("on"))
}

/// The login entry of the app in the `Run` key, through `reg`: the value is the app's name.
fn run_key_for(
    app: &Background<'_>,
    what: Autostart,
    exe: &Path,
    reg: &dyn Registry,
) -> Result<String, String> {
    run_key(app.run_value(), what, exe, reg)
}

/// What one question to the registry came to (in the form `reg.exe` answered it): whether it succeeded, what it
/// gave and what it complained of.
struct RegOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

impl RegOutput {
    /// `": <the first line of the complaint>"`, or nothing. The reason of a refusal (rule RS-06) is
    /// not thrown away; it goes to the terminal of the person only, cleaned of control characters.
    fn reason(&self) -> String {
        let line = self
            .stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let clean: String = line
            .trim()
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(200)
            .collect();
        if clean.is_empty() {
            String::new()
        } else {
            format!(": {clean}")
        }
    }
}

/// The registry, as far as the kit needs it: one question in the vocabulary of `reg.exe` (`query <key> [/v <value>]`,
/// `add <key> /v <value> /t REG_SZ /d <data> /f`, `delete <key> /v <value> /f`). The seam lets the tests stand in for
/// the real registry, which they must never change.
trait Registry {
    /// # Errors
    ///
    /// The question could not be asked at all (a refusal is no error here).
    fn run(&self, args: &[OsString]) -> Result<RegOutput, String>;
}

/// The real thing: the registry API of the system ([`crate::winreg`]), never `reg.exe`, which a hub that starts it to
/// change the login entries makes an endpoint protection read as an attack.
struct SystemRegistry;

impl Registry for SystemRegistry {
    fn run(&self, args: &[OsString]) -> Result<RegOutput, String> {
        let words: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        system_registry(&words)
    }
}

#[cfg(windows)]
fn system_registry(words: &[&str]) -> Result<RegOutput, String> {
    use crate::winreg;
    let answer = |success: bool, stdout: String, stderr: String| {
        Ok(RegOutput {
            success,
            stdout,
            stderr,
        })
    };
    match words {
        ["query", key] => answer(winreg::key_exists(key)?, String::new(), String::new()),
        ["query", key, "/v", value] => match winreg::get(key, value)? {
            Some(data) => answer(
                true,
                format!("\n{key}\n    {value}    REG_SZ    {data}\n"),
                String::new(),
            ),
            None => answer(false, String::new(), "the value is not there".to_owned()),
        },
        ["add", key, "/v", value, "/t", "REG_SZ", "/d", data, "/f"] => {
            match winreg::set(key, value, data) {
                Ok(()) => answer(true, String::new(), String::new()),
                Err(e) => answer(false, String::new(), e),
            }
        }
        ["delete", key, "/v", value, "/f"] => match winreg::delete(key, value) {
            Ok(true) => answer(true, String::new(), String::new()),
            Ok(false) => answer(false, String::new(), "the value is not there".to_owned()),
            Err(e) => answer(false, String::new(), e),
        },
        _ => Err(format!(
            "not a question the kit asks the registry: {words:?}"
        )),
    }
}

#[cfg(not(windows))]
fn system_registry(_words: &[&str]) -> Result<RegOutput, String> {
    Err("this system has no registry".to_owned())
}

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// The login entry in the `Run` key: on, off or its state.
fn run_key(value: &str, what: Autostart, exe: &Path, reg: &dyn Registry) -> Result<String, String> {
    match what {
        Autostart::On => {
            let mut add = args(&["add", RUN_KEY, "/v", value, "/t", "REG_SZ", "/d"]);
            add.push(OsString::from(run_command(exe)));
            add.push(OsString::from("/f"));
            let added = reg.run(&add)?;
            if added.success {
                Ok(format!(
                    "the hub starts with your login: {}\n",
                    run_command(exe)
                ))
            } else {
                Err(format!("the entry could not be written{}", added.reason()))
            }
        }
        Autostart::Off => {
            let deleted = reg.run(&args(&["delete", RUN_KEY, "/v", value, "/f"]))?;
            // A missing value is not an error: it is what was asked for. One that
            // is still there is (a policy or an ACL on the key). `reg delete` exits
            // the same for both, so the key is asked.
            if !deleted.success && login_value_present(value, reg)? {
                return Err(format!(
                    "the entry could not be removed{}",
                    deleted.reason()
                ));
            }
            Ok("the hub does not start with your login\n".to_owned())
        }
        Autostart::Status => {
            let out = reg.run(&args(&["query", RUN_KEY, "/v", value]))?;
            if out.success {
                Ok(format!(
                    "on: {}\n",
                    out.stdout
                        .lines()
                        .find(|l| l.contains(value))
                        .unwrap_or("")
                        .trim()
                ))
            } else {
                Ok("off\n".to_owned())
            }
        }
    }
}

/// Whether the `Run` key holds the value of the hub (`reg query` finds it).
fn login_value_present(value: &str, reg: &dyn Registry) -> Result<bool, String> {
    Ok(reg.run(&args(&["query", RUN_KEY, "/v", value]))?.success)
}

/// The `Uninstall` keys a setup of the app writes its entry for "Apps and features" under, named by
/// the product name (the name of the app): the current user (NSIS, mode current user), the machine
/// (mode all users) and the 32-bit view of the machine (a setup made as a 32-bit program).
const UNINSTALL_KEYS: [&str; 3] = [
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
];

/// An installation made by the setup of the app (not by `install`), as far as it can be found out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupInstall {
    /// The folder the setup installed to (`InstallLocation`), when it says one.
    pub location: Option<PathBuf>,
}

/// The data of a value in the output of `reg query`: `    <name>    REG_SZ    <data>`.
fn reg_value(stdout: &str, name: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(name)?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let (_, data) = rest.trim_start().split_once(char::is_whitespace)?;
        let data = data.trim();
        (!data.is_empty()).then(|| data.to_owned())
    })
}

/// Whether the setup of the app installed it: its entry under one of the `Uninstall` keys, found
/// through `reg` (the value `InstallLocation` is read when there is one). The first key that has
/// the entry answers.
fn setup_install_in(
    app: &Background<'_>,
    reg: &dyn Registry,
) -> Result<Option<SetupInstall>, String> {
    for root in UNINSTALL_KEYS {
        let key = format!(r"{root}\{}", app.identity().app);
        let found = reg.run(&args(&["query", &key, "/v", "InstallLocation"]))?;
        if found.success {
            let location = reg_value(&found.stdout, "InstallLocation").map(PathBuf::from);
            return Ok(Some(SetupInstall { location }));
        }
        // The key without that value is an installation all the same.
        if reg.run(&args(&["query", &key]))?.success {
            return Ok(Some(SetupInstall { location: None }));
        }
    }
    Ok(None)
}

/// Whether the setup of the app installed it (Windows: the `Uninstall` keys of the user and of the machine; Linux: a
/// program of the package manager at `/usr/bin/<binary>`). `install` finds it and does not build a
/// second one beside it.
///
/// # Errors
///
/// The registry could not be asked at all.
pub fn setup_install(app: &Background<'_>) -> Result<Option<SetupInstall>, String> {
    if cfg!(windows) {
        return setup_install_in(app, &SystemRegistry);
    }
    let packaged = Path::new("/usr/bin").join(app.identity().binary);
    Ok(packaged.is_file().then_some(SetupInstall {
        location: packaged.parent().map(Path::to_path_buf),
    }))
}

/// What `install` says when the setup installed the app: the way to repair or remove it.
fn setup_refusal(found: &SetupInstall) -> String {
    let at = found
        .location
        .as_ref()
        .map_or(String::new(), |p| format!(" ({})", p.display()));
    format!(
        "the app is installed by its setup{at}: run the setup again to repair it, or its \
         uninstaller to remove it; `install` would build a second copy beside it"
    )
}

/// What `uninstall` says and does after the hub is stopped and the login entry and the link are gone: the settings
/// (kept, or removed with `--purge-settings`), the program file that is left, and the files of a setup installation
/// that are the uninstaller's. `setup` is what was found of a setup (the caller asks for it).
fn uninstall_closing(
    app: &Background<'_>,
    env: &Env,
    opts: UninstallOptions,
    exe: &Path,
    setup: Option<&SetupInstall>,
    out: &mut String,
    failed: &mut Vec<String>,
) {
    if opts.purge_settings {
        match instance::config_folder(env, app.identity()) {
            Ok(dir) => match purge_settings(app, &dir) {
                Ok(text) => out.push_str(&text),
                Err(e) => failed.push(e),
            },
            Err(e) => failed.push(e),
        }
    } else if let Ok(dir) = instance::config_folder(env, app.identity()) {
        out.push_str(&format!(
            "your settings stay in {} (`uninstall --purge-settings` removes them)\n",
            dir.display()
        ));
    }
    out.push_str(&program_left_text(app, exe));
    push_setup_note(out, setup);
}

/// The closing line of `uninstall` when a setup installed the app (nothing when none did).
fn push_setup_note(out: &mut String, found: Option<&SetupInstall>) {
    if let Some(found) = found {
        out.push_str(&setup_left_text(found));
    }
}

/// What `uninstall` says about a setup installation: its files are the uninstaller's to remove.
fn setup_left_text(found: &SetupInstall) -> String {
    let at = found
        .location
        .as_ref()
        .map_or(String::new(), |p| format!(" at {}", p.display()));
    format!("the setup's installation{at} is left: remove it with the uninstaller of the setup\n")
}

/// Windows keeps what a *packaged* app (an MSIX, like the Claude app) and its
/// child processes write below `AppData`, and into the registry, in a private
/// store of that app. A terminal of the user does not see any of it: the
/// program, the settings and the login entry would be there for the app and
/// missing for the person. Nothing in the environment says so, so a probe
/// file is written and looked for in the stores of the packages.
///
/// Returns the folder of the package that holds the probe.
fn private_storage_of(env: &Env, dir: &Path) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let local = env.path("LOCALAPPDATA")?;
    let roaming = env.path("APPDATA");
    let (kind, rel) = if let Some(rel) = strip_prefix_ignore_case(dir, &local) {
        ("Local", rel)
    } else {
        let rel = roaming
            .as_deref()
            .and_then(|r| strip_prefix_ignore_case(dir, r))?;
        ("Roaming", rel)
    };
    let name = format!(".probe-{}", std::process::id());
    let probe = dir.join(&name);
    std::fs::write(&probe, b"").ok()?;
    let found = find_in_packages(&local, kind, &rel, &name);
    let _ = std::fs::remove_file(&probe);
    found
}

/// `path` below `base`, the way Windows compares names (case does not matter).
fn strip_prefix_ignore_case(path: &Path, base: &Path) -> Option<PathBuf> {
    let mut rest = path.components();
    for part in base.components() {
        let own = rest.next()?;
        let same = |c: Component| c.as_os_str().to_string_lossy().to_lowercase();
        if same(own) != same(part) {
            return None;
        }
    }
    Some(rest.as_path().to_path_buf())
}

/// The package whose private store holds `<kind>/<rel>/<name>`.
fn find_in_packages(local: &Path, kind: &str, rel: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(local.join("Packages"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|pkg| {
            pkg.join("LocalCache")
                .join(kind)
                .join(rel)
                .join(name)
                .exists()
        })
}

/// A line to put in front of the person when this program runs inside a
/// packaged app.
fn private_storage_warning(env: &Env, dir: &Path) -> Option<String> {
    let package = private_storage_of(env, dir)?;
    Some(format!(
        "warning: this program runs inside a packaged app ({}). What it writes below AppData \
         and into the registry (the program, the settings, the login entry) stays in that \
         app's private storage, and a terminal of your own does not see it. Run this command \
         in a normal terminal.",
        package.display()
    ))
}

/// Where `install` puts the program: a folder of this user that is not
/// roaming (the settings are; a program is not).
fn install_dir(app: &Background<'_>, env: &Env) -> Result<PathBuf, String> {
    let var = app.identity().var("INSTALL_DIR");
    if let Some(dir) = env.path(&var) {
        // A relative folder would mean whatever the working folder is.
        if !dir.is_absolute() {
            return Err(format!("{var} must be an absolute path"));
        }
        return Ok(dir);
    }
    let base = if cfg!(windows) {
        env.path("LOCALAPPDATA")
            .ok_or_else(|| format!("no LOCALAPPDATA: set {var}"))?
    } else if let Some(x) = env.path("XDG_DATA_HOME") {
        x
    } else {
        env.path("HOME")
            .map(|h| h.join(".local").join("share"))
            .ok_or_else(|| format!("no HOME: set {var}"))?
    };
    Ok(base.join(app.identity().app).join("bin"))
}

/// Where `install` puts this program: the install folder and the file name of the running program.
///
/// # Errors
///
/// The reason there is no install folder (no `LOCALAPPDATA`, `HOME`, or a relative `<PREFIX>_INSTALL_DIR`).
pub fn installed_program(app: &Background<'_>, env: &Env) -> Result<PathBuf, String> {
    Ok(install_dir(app, env)?.join(app.identity().program_file()))
}

/// What the link of the app is called.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LinkName {
    /// The title of the app (`Rules.lnk`).
    #[default]
    Title,
    /// The name of the app (`hub-desktop.lnk`).
    App,
}

/// How the link of the app starts it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LinkStart {
    /// `start --open`: in the background, and the browser opens the page. The console closes by itself.
    #[default]
    OpenPage,
    /// No argument, as a double click on the program: what that does is the app's to say. It opens the page itself and
    /// says what went wrong, which a bare `serve` in a minimized window would not.
    AsClicked,
}

/// What `install` was asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOptions {
    /// What the link is called, when one is made (on Windows; a launcher elsewhere keeps its own name).
    pub link_name: LinkName,
    /// How the link starts the app, when one is made (on Windows; a launcher elsewhere runs `start --open`).
    pub link_start: LinkStart,
    /// Register the installed program with the login of the user.
    pub autostart: bool,
    /// Also put a link (named by the title of the app) on the desktop (Windows; a launcher elsewhere). The link in
    /// the Start menu is made by every install on Windows, with or without this (unless the tests of the program
    /// ask for no changes to the machine, see `NO_LOGIN_SYNC`).
    pub shortcut: bool,
}

/// With this variable set, `install` leaves an existing login entry alone
/// (it does not point it at the installed copy). The tests of the program set
/// it: the entry of the machine they run on is not theirs to change.
fn no_login_sync(app: &Background<'_>) -> String {
    app.identity().var("NO_LOGIN_SYNC")
}

/// Copy this program to a fixed place, so that a build in the repository
/// does not fight the running hub for the file, and start the hub from there
/// again when one was running.
///
/// # Errors
///
/// A text for the person at the terminal. What was done before it is printed
/// (the caller prints a text only when all went well).
pub async fn install(app: &Background<'_>, opts: InstallOptions) -> Result<String, String> {
    let mut out = String::new();
    match install_steps(app, opts, &mut out).await {
        Ok(()) => Ok(out),
        Err(e) => {
            print!("{out}");
            Err(e)
        }
    }
}

/// How often the program file is moved over the old one, and the pause between the tries.
const RENAME_TRIES: u32 = 3;
const RENAME_PAUSE: Duration = Duration::from_millis(200);

/// Whether a failed rename is the file being held for a moment (a virus scanner or the indexer after the program
/// ended): "access denied" (5) or "in use" (32, 33) on Windows. Nothing else is tried again, and no other system.
fn file_is_held(error: &std::io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == std::io::ErrorKind::PermissionDenied
            || matches!(error.raw_os_error(), Some(5 | 32 | 33)))
}

/// Run `attempt` up to `tries` times, `pause` apart, again only while `held` says the error is a moment's. On failure
/// the error comes back with the number of tries made.
async fn retry_while_held<F>(
    mut attempt: F,
    held: fn(&std::io::Error) -> bool,
    tries: u32,
    pause: Duration,
) -> Result<(), (std::io::Error, u32)>
where
    F: FnMut() -> std::io::Result<()>,
{
    let mut made = 0;
    loop {
        made += 1;
        match attempt() {
            Ok(()) => return Ok(()),
            Err(e) if made < tries && held(&e) => tokio::time::sleep(pause).await,
            Err(e) => return Err((e, made)),
        }
    }
}

/// What `install` says when the new program could not be moved over the old one: the tries, if more than one was
/// made, and the hint that fits.
fn replace_failure(target: &Path, made: u32, hint: &str, error: &std::io::Error) -> String {
    let tries = if made > 1 {
        format!(" after {made} tries")
    } else {
        String::new()
    };
    format!("cannot replace {}{tries}{hint}: {error}", target.display())
}

async fn install_steps(
    app: &Background<'_>,
    opts: InstallOptions,
    out: &mut String,
) -> Result<(), String> {
    install_steps_beside(app, opts, out, setup_install(app)?).await
}

/// The steps of `install` when `setup` is what a setup of the app left (found by the caller): that is a
/// refusal before anything is touched.
async fn install_steps_beside(
    app: &Background<'_>,
    opts: InstallOptions,
    out: &mut String,
    setup: Option<SetupInstall>,
) -> Result<(), String> {
    if let Some(found) = setup {
        return Err(setup_refusal(&found));
    }
    let env = Env::from_process();
    let dir = install_dir(app, &env)?;
    let from = std::env::current_exe().map_err(|e| format!("own program not found: {e}"))?;
    let name = std::ffi::OsString::from(app.identity().program_file());
    let target = dir.join(&name);

    // A program that is the one being installed (run from the install
    // folder) has nothing to copy.
    let same = match (std::fs::canonicalize(&from), std::fs::canonicalize(&target)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    let config = instance::config_folder(&env, app.identity())?;
    let was_running = instance::running_hub(&config, app.identity())
        .await
        .is_some();
    if !same {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        // Copy beside, then move over: a copy that is cut short never
        // replaces a program that works. The hub runs until the move: nothing
        // before it needs it stopped.
        let part = dir.join(format!("{}.new", name.to_string_lossy()));
        if let Err(e) = std::fs::copy(&from, &part) {
            let _ = std::fs::remove_file(&part);
            return Err(format!("cannot copy to {}: {e}", part.display()));
        }
        if was_running {
            match stop(app).await {
                Ok(text) => out.push_str(&text),
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(e);
                }
            }
        }
        if let Err((e, made)) = retry_while_held(
            || std::fs::rename(&part, &target),
            file_is_held,
            RENAME_TRIES,
            RENAME_PAUSE,
        )
        .await
        {
            let _ = std::fs::remove_file(&part);
            // A hub that was stopped for this is not left down: it comes back
            // from the old installed program, or from this one.
            let hint = if was_running {
                String::new()
            } else {
                format!(
                    " (is a hub running from it? `{} stop`)",
                    app.identity().binary
                )
            };
            let mut message = replace_failure(&target, made, &hint, &e);
            if was_running {
                let back = if target.is_file() {
                    start_from(&target)
                } else {
                    start(app).await
                };
                match back {
                    Ok(text) => out.push_str(&text),
                    Err(e) => message.push_str(&format!("; the hub is down: {e}")),
                }
            }
            return Err(message);
        }
    }
    out.push_str(&format!("installed: {}\n", target.display()));
    out.push_str(&register_in_family(app, &env, &target));
    if let Some(warning) = private_storage_warning(&env, &dir) {
        // On stderr and at once: a later error must not take it away.
        eprintln!("{warning}");
    }
    if was_running && !same {
        // The hub comes back from the installed program, before the steps that
        // do not need it: one of them failing must not leave it down.
        out.push_str(&start_from(&target)?);
    }

    if cfg!(windows) {
        out.push_str(&entry_and_links(
            app,
            &target,
            opts,
            &env,
            Ok(&SystemRegistry as &dyn Registry),
        )?);
    } else {
        // It may be registered for another copy: then it is pointed here.
        let sync = env.get(&no_login_sync(app)).is_none();
        let was_on = (opts.autostart || sync) && autostart_is_on(app);
        let entry = |what| autostart_for(app, what, &target);
        let entry_made = login_entry_step(opts, was_on, sync, entry, out)?;
        if opts.shortcut {
            let made = shortcut(
                app,
                &target,
                opts.link_name,
                opts.link_start,
                &LinkPlace::ALL,
            );
            out.push_str(&undo_entry_on_failure(made, entry_made, entry)?);
        }
    }
    out.push_str(&how_to_start(
        app,
        &target,
        env.get("PATH").map(OsString::as_os_str),
    ));
    Ok(())
}

/// The login entry of `install`: made when asked for, pointed at the installed copy when it was on for another
/// one. Returns whether this install made an entry that was not there before (the one to take back when a later
/// step fails). `was_on` is asked before anything is written, and it is what was really there: an entry of the
/// person is never this install's to take back, whether or not it may be pointed here (`sync`: not for the tests of
/// the program, `NO_LOGIN_SYNC`).
fn login_entry_step(
    opts: InstallOptions,
    was_on: bool,
    sync: bool,
    entry: impl Fn(Autostart) -> Result<String, String>,
    out: &mut String,
) -> Result<bool, String> {
    if opts.autostart || (sync && was_on) {
        out.push_str(&entry(Autostart::On)?);
    }
    Ok(opts.autostart && !was_on)
}

/// A step after the login entry failed: an entry this install made goes away again, so that an install which
/// did not finish leaves nothing behind that starts a program at the next login. The error says which it was.
fn undo_entry_on_failure(
    step: Result<String, String>,
    entry_made: bool,
    entry: impl Fn(Autostart) -> Result<String, String>,
) -> Result<String, String> {
    match step {
        Err(e) if entry_made => Err(match entry(Autostart::Off) {
            Ok(_) => format!("{e}; the login entry this install made is taken back"),
            Err(left) => format!("{e}; the login entry this install made is still there: {left}"),
        }),
        other => other,
    }
}

/// The login entry (the link in the Startup folder) and the links of `install` on Windows, through `reg` (or why it
/// cannot be asked).
fn entry_and_links(
    app: &Background<'_>,
    target: &Path,
    opts: InstallOptions,
    env: &Env,
    reg: Result<&dyn Registry, String>,
) -> Result<String, String> {
    let mut out = String::new();
    // The tests of the program: an entry of the machine is not theirs to point somewhere else.
    let sync = env.get(&no_login_sync(app)).is_none();
    let mut entry_made = false;
    // A registry that cannot be asked is an error only when an entry is wanted; else there is none to point here.
    let asked = if opts.autostart {
        Some(reg.clone()?)
    } else {
        reg.clone().ok().filter(|_| sync)
    };
    if let Some(asked) = asked {
        let was_on = startup_entry(app, Autostart::Status, target, env, asked)
            .is_ok_and(|said| said.starts_with("on"));
        // It may be registered for another copy: then it is pointed here.
        entry_made = login_entry_step(
            opts,
            was_on,
            sync,
            |what| startup_entry(app, what, target, env, asked),
            &mut out,
        )?;
    }
    let links = install_links(app, target, opts, env, reg.clone());
    out.push_str(&undo_entry_on_failure(links, entry_made, |what| {
        startup_entry(app, what, target, env, reg.clone()?)
    })?);
    Ok(out)
}

/// The last lines of `install`: the whole call that starts the installed program, and whether its folder is on
/// `PATH`. Install does not put it there: a program of the user is not added to the path of every shell behind
/// their back. Windows compares the folders without case and without a trailing separator.
#[must_use]
pub fn how_to_start(app: &Background<'_>, target: &Path, path_var: Option<&OsStr>) -> String {
    let dir = target.parent().unwrap_or(target);
    let same = |p: &Path| {
        if cfg!(windows) {
            p.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .eq_ignore_ascii_case(dir.to_string_lossy().trim_end_matches(['\\', '/']))
        } else {
            p == dir
        }
    };
    let on_path = path_var.is_some_and(|v| std::env::split_paths(v).any(|p| same(&p)));
    let binary = app.identity().binary;
    let mut out = format!("start it: \"{}\" start --open\n", target.display());
    if on_path {
        out.push_str(&format!(
            "its folder is on PATH: `{binary} start --open` works in any shell\n"
        ));
    } else {
        let menu = if cfg!(windows) {
            format!(
                " (the Start menu entry {} starts it too)",
                app.identity().title
            )
        } else {
            String::new()
        };
        out.push_str(&format!(
            "its folder is not on PATH; add {} to call `{binary}` by name{menu}\n",
            dir.display()
        ));
    }
    out
}

/// The links `install` makes on Windows: with `shortcut` on the desktop and in the Start menu (a link that
/// cannot be made is an error); without it only in the Start menu, where the program is found, which is not
/// left to a flag (a link that cannot be made is said, and the install, which is done, still succeeds);
/// nothing automatic when the tests of the program ask for no changes to the machine (`NO_LOGIN_SYNC`).
/// `reg` is the registry, or why it cannot be asked.
fn install_links(
    app: &Background<'_>,
    target: &Path,
    opts: InstallOptions,
    env: &Env,
    reg: Result<&dyn Registry, String>,
) -> Result<String, String> {
    if opts.shortcut {
        return shortcut_in(
            app,
            target,
            opts.link_name,
            opts.link_start,
            &LinkPlace::ALL,
            env,
            reg?,
        );
    }
    if env.get(&no_login_sync(app)).is_some() {
        return Ok(String::new());
    }
    let made = reg.and_then(|reg| {
        shortcut_in(
            app,
            target,
            opts.link_name,
            opts.link_start,
            &[LinkPlace::StartMenu],
            env,
            reg,
        )
    });
    Ok(made.unwrap_or_else(|e| format!("no link in the Start menu: {e}\n")))
}

/// How long `<program> start` may take. It waits twenty seconds for its hub itself.
const START_FROM_LIMIT: Duration = Duration::from_secs(60);
/// The most `<program> start` may print, per stream: a line or two, and a warning.
const START_FROM_OUTPUT: usize = 64 * 1024;

/// Run `<program> start` and return what it printed. Its complaint (on
/// stderr, a warning included) goes to the terminal, line by line as it comes.
///
/// The program is a child of this process while it runs (`appkit_core::child`: no console window,
/// an absolute path, named in the log); the hub it puts in the background is not, and stays
/// ([`Leftovers::Keep`]). The call blocks, as it always did: it runs on a thread of its own, so
/// it may be called inside a runtime and outside of one.
///
/// # Errors
///
/// A text for the person at the terminal.
pub fn start_from(program: &Path) -> Result<String, String> {
    // The program inherits every inheritable handle of this process, not only its stdio, and
    // hands them on to the hub it starts: the standard handles of this process (a pipe of the
    // caller) are kept here, as in `spawn_detached`.
    #[cfg(windows)]
    crate::handles::keep_standard_handles();
    let run = Run {
        start: Start::new(start_from_caller(), program)
            .arg("start")
            .env(Environment::Inherited {
                set: Vec::new(),
                remove: Vec::new(),
            }),
        stdin: Vec::new(),
        timeout: START_FROM_LIMIT,
        max_stdout: START_FROM_OUTPUT,
        max_stderr: START_FROM_OUTPUT,
        leftovers: Leftovers::Keep,
    };
    let (code, printed) = crate::blocking::block_on(|| async move {
        let mut printed = String::new();
        let code = child::run_lines(run, |stream, line| match stream {
            Stream::Stdout => {
                printed.push_str(line);
                printed.push('\n');
            }
            Stream::Stderr => eprintln!("{line}"),
        })
        .await;
        (code, printed)
    })
    .map_err(|e| format!("the installed program did not start: {e}"))?;
    let code = code.map_err(|e| format!("the installed program did not start: {e}"))?;
    if code == Some(0) {
        Ok(printed)
    } else {
        Err("the hub did not come back from the installed program (see above)".to_owned())
    }
}

/// Who asks, for the log of the helper: the name of this program and why.
fn start_from_caller() -> String {
    let me = std::env::current_exe().ok();
    let name = me
        .as_deref()
        .and_then(Path::file_stem)
        .map_or_else(|| "hub".to_owned(), |n| n.to_string_lossy().into_owned());
    format!("{name} start of the installed hub")
}

fn autostart_is_on(app: &Background<'_>) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    matches!(autostart_for(app, Autostart::Status, &exe), Ok(text) if text.starts_with("on"))
}

/// What `uninstall` was asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct UninstallOptions {
    /// Also remove the settings of the app in the config folder (never the vault).
    pub purge_settings: bool,
}

/// The files every hub keeps in the config folder; those of the app are added by [`Background`].
/// Nothing else is ever removed there.
const KIT_FILES: [&str; 5] = [
    "config.json",
    "usage.json",
    "hub.json",
    "control.secret",
    "hub.log",
];

/// Remove the files of the app from the config folder, and the folder when nothing else is in it.
fn purge_settings(app: &Background<'_>, dir: &Path) -> Result<String, String> {
    let mut removed = Vec::new();
    for name in KIT_FILES.iter().chain(app.extra_files) {
        let file = dir.join(name);
        match std::fs::remove_file(&file) {
            Ok(()) => removed.push(*name),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {e}", file.display())),
        }
    }
    let mut text = if removed.is_empty() {
        format!("no settings were in {}\n", dir.display())
    } else {
        format!(
            "settings removed from {}: {}\n",
            dir.display(),
            removed.join(", ")
        )
    };
    // `remove_dir` takes an empty folder only: a folder with other files stays.
    match std::fs::remove_dir(dir) {
        Ok(()) => text.push_str("the settings folder is removed\n"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => text.push_str("the settings folder holds other files and stays\n"),
    }
    Ok(text)
}

/// Take the integration of the program with the system away: stop the hub, the login entry, the
/// link or launcher, and (when asked) the settings. The program file itself is left: it is the
/// one that runs, and a running program cannot delete itself on every system; the script the app
/// names ([`Background::uninstall_script`]) removes the install folder after this program has
/// ended. The vault and the documents in it are never touched.
///
/// # Errors
///
/// A text for the person at the terminal. Every step is tried; the steps that failed are named
/// together at the end (what went right is printed first).
pub fn uninstall<'a>(
    app: &'a Background<'a>,
    opts: UninstallOptions,
) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
    // On the heap, and made here: the future of the steps is large, and in a debug build a large
    // future that is made in the `match` of `main` takes room in the stack of the main thread for
    // every command (it overflowed it, `serve` included).
    Box::pin(uninstall_steps(app, opts))
}

async fn uninstall_steps(app: &Background<'_>, opts: UninstallOptions) -> Result<String, String> {
    let env = Env::from_process();
    let mut out = String::new();
    let mut failed: Vec<String> = Vec::new();
    let mut step = |result: Result<String, String>, out: &mut String| match result {
        Ok(text) => out.push_str(&text),
        Err(e) => failed.push(e),
    };

    step(stop(app).await, &mut out);
    if let Ok(dir) = instance::config_folder(&env, app.identity()) {
        remove_old_start_script(&dir);
    }
    let exe = std::env::current_exe().map_err(|e| format!("own program not found: {e}"))?;
    if env.get(&no_login_sync(app)).is_some() {
        // The tests of the program: the login entry and the link of the machine are not theirs.
        out.push_str(&format!(
            "the login entry and the link are left alone ({})\n",
            no_login_sync(app)
        ));
    } else {
        step(autostart(app, Autostart::Off), &mut out);
        step(remove_shortcut(app, &env), &mut out);
    }
    unregister_from_family(app, &env);
    uninstall_closing(
        app,
        &env,
        opts,
        &exe,
        setup_install(app).ok().flatten().as_ref(),
        &mut out,
        &mut failed,
    );
    if failed.is_empty() {
        Ok(out)
    } else {
        print!("{out}");
        Err(failed.join("; "))
    }
}

/// The family folder that `install` registers the app in and `uninstall` takes it out of
/// (`appkit_core::family`). The tests of the program (`NO_LOGIN_SYNC`) leave the folder of the machine
/// alone unless they name their own (`APPKIT_FAMILY_DIR`).
fn family_folder(app: &Background<'_>, env: &Env) -> Option<PathBuf> {
    if env.get(&no_login_sync(app)).is_some() && env.get(family::FAMILY_DIR_VAR).is_none() {
        return None;
    }
    family::family_dir(&|name: &str| env.path(name))
}

/// Tell the family that the app is installed at `target` (the manifest that other apps read to
/// know they can hand something over to it). A failure is said, never fatal: the app works without it.
fn register_in_family(app: &Background<'_>, env: &Env, target: &Path) -> String {
    let Some(folder) = family_folder(app, env) else {
        return String::new();
    };
    let manifest = family::Manifest {
        schema: family::MANIFEST_SCHEMA.to_owned(),
        app: app.identity().app.to_owned(),
        version: None,
        program: target.to_path_buf(),
        env_prefix: app.identity().env_prefix.to_owned(),
    };
    match family::write_manifest(&folder, &manifest) {
        Ok(()) => String::new(),
        Err(e) => format!(
            "the other apps of the family will not find this one: cannot write {}: {e}\n",
            family::manifest_path(&folder, &manifest.app).display()
        ),
    }
}

/// Take the app out of the family folder again.
fn unregister_from_family(app: &Background<'_>, env: &Env) {
    if let Some(folder) = family_folder(app, env) {
        family::remove_manifest(&folder, app.identity().app);
    }
}

/// The script an older version started the hub through (the script host): nothing reads it any more, and a script
/// in a settings folder is what an endpoint protection looks for. `start` removes it too; a hub that is never
/// started again would keep it.
fn remove_old_start_script(dir: &Path) {
    let _ = std::fs::remove_file(dir.join("start-hub.vbs"));
}

/// What `uninstall` says about the program file it leaves, and who deletes it.
fn program_left_text(app: &Background<'_>, exe: &Path) -> String {
    format!(
        "the program file is left at {}: delete it ({} does)\n",
        exe.display(),
        app.uninstall_script
    )
}

/// The key that names the folders of the user; `Desktop` there is the desktop, also one that was
/// moved (to OneDrive, by a policy).
const SHELL_FOLDERS_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";

/// `%NAME%` in `text` replaced by the variable of `env`; `None` when one is not set (or the text
/// is cut).
fn expand_vars(text: &str, env: &Env) -> Option<PathBuf> {
    let mut out = OsString::new();
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        out.push(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after.find('%')?;
        out.push(env.get(&after[..end])?);
        rest = &after[end + 1..];
    }
    out.push(rest);
    Some(PathBuf::from(out))
}

/// Where a link of the app is put: the desktop and the Start menu (`Programs` of the user, the
/// list Windows shows under "All apps" and searches).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkPlace {
    Desktop,
    StartMenu,
    /// The Startup folder of the user: what is linked there starts with the login.
    Startup,
}

impl LinkPlace {
    const ALL: [Self; 2] = [Self::Desktop, Self::StartMenu];

    /// The value of the folder under [`SHELL_FOLDERS_KEY`].
    fn value(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop",
            Self::StartMenu => "Programs",
            Self::Startup => "Startup",
        }
    }

    /// The folder when the registry names none.
    fn fallback(self, env: &Env) -> Option<PathBuf> {
        match self {
            Self::Desktop => env.path("USERPROFILE").map(|p| p.join("Desktop")),
            Self::StartMenu => env
                .path("APPDATA")
                .map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs")),
            Self::Startup => env
                .path("APPDATA")
                .map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs\Startup")),
        }
    }

    /// How the texts name the place: "on the desktop", "in the Start menu".
    fn said(self) -> &'static str {
        match self {
            Self::Desktop => "on the desktop",
            Self::StartMenu => "in the Start menu",
            Self::Startup => "in the Startup folder",
        }
    }
}

/// A folder of the user: its value under [`SHELL_FOLDERS_KEY`] (`Desktop`, `Programs`) with its
/// variables expanded (`%OneDrive%\Desktop`: the variable comes from the environment, whatever its
/// characters), else its usual place in the profile when the registry has no such value. A value
/// that is there but cannot be used is an error, not a guess: `reg.exe` prints in the code page of
/// the console, so a path with a character outside it comes back broken, and a link put in another
/// folder would not be where the person looks.
fn shell_folder(env: &Env, reg: &dyn Registry, place: &LinkPlace) -> Result<PathBuf, String> {
    let name = place.value();
    let out = reg.run(&args(&["query", SHELL_FOLDERS_KEY, "/v", name]))?;
    let named = if out.success {
        reg_value(&out.stdout, name)
    } else {
        None
    };
    if let Some(value) = named {
        let dir = if value.contains('\u{FFFD}') {
            None
        } else {
            expand_vars(&value, env)
        };
        return dir.filter(|p| p.is_absolute()).ok_or_else(|| {
            format!(
                "the folder {} the registry names cannot be used: {value}",
                place.said()
            )
        });
    }
    place
        .fallback(env)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| format!("no folder {} found (no profile)", place.said()))
}

/// Take the links of the desktop and the Start menu (named by the title of the app) away
/// (Windows), or the launcher (Linux). Both places are tried; the first failure is the answer.
fn remove_shortcut(app: &Background<'_>, env: &Env) -> Result<String, String> {
    if !cfg!(windows) {
        let Some(platform) = loginitems::Platform::current() else {
            return Ok(String::new());
        };
        return loginitems::remove_launcher(&app.login, platform, env);
    }
    let reg = SystemRegistry;
    let mut out = String::new();
    let mut failed = None;
    // Under either name: the program does not know how it was installed. Only a link to this program is taken away.
    for name in [app.identity().title, app.identity().app] {
        for place in LinkPlace::ALL {
            let removed = shell_folder(env, &reg, &place)
                .and_then(|dir| remove_link_in(&dir, place, name, &app.identity().program_file()));
            match removed {
                Ok(text) => out.push_str(&text),
                Err(e) => {
                    failed.get_or_insert(e);
                }
            }
        }
    }
    failed.map_or(Ok(out), Err)
}

/// Delete `<title>.lnk` in `dir` when it points at the program `program`: a link of the
/// person's own that has the same name is not ours to delete.
fn remove_link_in(
    dir: &Path,
    place: LinkPlace,
    title: &str,
    program: &str,
) -> Result<String, String> {
    crate::shell_link::remove_if_target(&dir.join(desktop_link_name(title)), program)
        .map_err(|e| format!("the link {} could not be checked: {e}", place.said()))?;
    // A link of the person's own with that name is not ours and stays: the text does not claim
    // that the folder is free of any link of that name.
    Ok(format!(
        "no link \"{title}\" of ours is {} (one that was is removed)\n",
        place.said()
    ))
}

/// The file name of the desktop link of the app: its title and `.lnk`.
#[must_use]
pub fn desktop_link_name(title: &str) -> String {
    format!("{title}.lnk")
}

/// The link of the app: it starts `target` the way `start` says, minimized; the description names the app.
#[must_use]
pub fn app_link(app: &str, target: &Path, start: LinkStart) -> crate::shell_link::Link {
    let target = target.to_string_lossy().into_owned();
    // The folder by the text of the path: a Windows path, whatever system builds the link.
    let working_dir = target
        .rsplit_once(['\\', '/'])
        .map(|(dir, _)| dir.to_owned())
        .unwrap_or_default();
    crate::shell_link::Link {
        target,
        arguments: match start {
            LinkStart::OpenPage => "start --open",
            LinkStart::AsClicked => "",
        }
        .to_owned(),
        working_dir,
        description: match start {
            LinkStart::OpenPage => format!("{app}: start the hub and open the page"),
            LinkStart::AsClicked => format!("{app}: start the hub and open the page"),
        },
        show_command: 7,
    }
}

/// A link in each of `places` (named by the title of the app) that starts the hub and opens the page.
/// It is written as a file (`shell_link`), not through the script host, which an endpoint
/// protection stops when an unsigned program starts it.
fn shortcut(
    app: &Background<'_>,
    exe: &Path,
    name: LinkName,
    start: LinkStart,
    places: &[LinkPlace],
) -> Result<String, String> {
    if !cfg!(windows) {
        let platform =
            loginitems::Platform::current().ok_or("a launcher is not built for this system")?;
        return loginitems::launcher(&app.login, platform, exe, &Env::from_process());
    }
    let reg = SystemRegistry;
    shortcut_in(app, exe, name, start, places, &Env::from_process(), &reg)
}

/// [`shortcut`] on Windows, with the environment and the registry it reads the folders from.
fn shortcut_in(
    app: &Background<'_>,
    exe: &Path,
    name: LinkName,
    start: LinkStart,
    places: &[LinkPlace],
    env: &Env,
    reg: &dyn Registry,
) -> Result<String, String> {
    let mut out = String::new();
    for &place in places {
        let dir = shell_folder(env, reg, &place)?;
        let link_name = match name {
            LinkName::Title => app.identity().title,
            LinkName::App => app.identity().app,
        };
        // A link to this program under the other name is from an earlier install: two links would do different things.
        let other = match name {
            LinkName::Title => app.identity().app,
            LinkName::App => app.identity().title,
        };
        if other != link_name {
            let _ = remove_link_in(&dir, place, other, &app.identity().program_file());
        }
        out.push_str(&make_link_in(
            &dir,
            place,
            link_name,
            app.identity().app,
            exe,
            start,
        )?);
    }
    Ok(out)
}

fn make_link_in(
    dir: &Path,
    place: LinkPlace,
    title: &str,
    app: &str,
    exe: &Path,
    start: LinkStart,
) -> Result<String, String> {
    // The Start menu folder of a fresh profile may not be there yet.
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("the link {} could not be made: {e}", place.said()))?;
    crate::shell_link::write(
        &dir.join(desktop_link_name(title)),
        &app_link(app, exe, start),
    )
    .map_err(|e| format!("the link {} could not be made: {e}", place.said()))?;
    Ok(format!("a link \"{title}\" is {}\n", place.said()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_console_hub_is_started_without_a_window_and_a_window_program_as_any_program() {
        assert_eq!(detach_flags(Program::Console), 0x0000_0200 | 0x0800_0000);
        // CREATE_NEW_PROCESS_GROUP only: never CREATE_NO_WINDOW, never DETACHED_PROCESS.
        assert_eq!(detach_flags(Program::Window), 0x0000_0200);
    }

    #[test]
    fn the_hub_in_the_background_gets_its_verb_and_the_variable_of_start_and_nothing_else() {
        let command = detached_command(&APP, Path::new("/apps/rules-hub"), "tray");
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(args, [OsStr::new("tray")]);
        let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
        assert_eq!(
            envs,
            [(OsStr::new("RULES_DESKTOP_DETACHED"), Some(OsStr::new("1")))]
        );
    }

    #[test]
    fn only_a_plain_word_is_a_verb() {
        for good in ["serve", "tray", "run-here"] {
            assert!(plain_verb(good), "{good:?}");
        }
        for bad in [
            "",
            "-x",
            "--open",
            "Serve",
            "serve now",
            "a/b",
            "serve\u{0}",
            &"x".repeat(33),
        ] {
            assert!(!plain_verb(bad), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn start_as_refuses_a_verb_that_is_not_a_plain_word_before_it_looks_at_anything() {
        let err = start_as(&APP, "--evil", Program::Window).await.unwrap_err();
        assert!(err.contains("not a verb"), "{err}");
    }
    use appkit_scratch::Scratch;

    /// An app that is not the one this kit was made from.
    const IDENTITY: AppIdentity = AppIdentity {
        app: "rules-desktop",
        binary: "rules-hub",
        env_prefix: "RULES_DESKTOP",
        title: "Rules",
        default_port: 17616,
        route_ns: "rules",
    };
    const EXTRA: [&str; 2] = ["rules.json", "waivers.json"];
    const APP: Background<'static> = Background {
        login: LoginApp {
            identity: &IDENTITY,
            summary: "the rule overview",
        },
        extra_files: &EXTRA,
        uninstall_script: "scripts/rules-uninstall",
    };
    const RUN_VALUE: &str = "rules-desktop";

    fn run_key(what: Autostart, exe: &Path, reg: &dyn Registry) -> Result<String, String> {
        super::run_key(RUN_VALUE, what, exe, reg)
    }

    fn purge_settings(dir: &Path) -> Result<String, String> {
        super::purge_settings(&APP, dir)
    }

    #[test]
    fn install_registers_the_app_in_the_family_and_uninstall_takes_it_out() {
        let root = Scratch::new("hub-family-register");
        let program = root.join("rules-hub.exe");
        std::fs::create_dir_all(&*root).unwrap();
        std::fs::write(&program, b"x").unwrap();
        let family_dir = root.join("family");
        let env = Env::from_pairs([
            (family::FAMILY_DIR_VAR, family_dir.clone()),
            ("RULES_DESKTOP_NO_LOGIN_SYNC", PathBuf::from("1")),
        ]);
        assert_eq!(register_in_family(&APP, &env, &program), "");
        let manifest =
            family::read_manifest(&family::manifest_path(&family_dir, "rules-desktop")).unwrap();
        assert_eq!(manifest.program, program);
        assert_eq!(manifest.env_prefix, "RULES_DESKTOP");
        let vars = |name: &str| env.path(name);
        assert!(family::family_apps(&vars, &[])[0].installed);
        unregister_from_family(&APP, &env);
        assert!(family::family_apps(&vars, &[]).is_empty());
    }

    #[test]
    fn the_tests_of_a_program_leave_the_family_folder_of_the_machine_alone() {
        let tests = Env::from_pairs([
            ("RULES_DESKTOP_NO_LOGIN_SYNC", "1"),
            ("LOCALAPPDATA", "C:/local"),
            ("XDG_DATA_HOME", "/data"),
        ]);
        assert_eq!(family_folder(&APP, &tests), None);
        let person = Env::from_pairs([("LOCALAPPDATA", "C:/local"), ("XDG_DATA_HOME", "/data")]);
        if !cfg!(target_os = "macos") {
            assert!(family_folder(&APP, &person).is_some());
        }
    }

    #[test]
    fn a_link_can_be_named_by_the_app_and_start_it_as_a_click_does() {
        let desktop = Scratch::new("hub-link-app");
        let exe = Path::new(r"C:\Apps\rules-hub.exe");
        make_link_in(
            &desktop,
            LinkPlace::Desktop,
            IDENTITY.app,
            IDENTITY.app,
            exe,
            LinkStart::AsClicked,
        )
        .unwrap();
        let link = desktop.join(format!("{}.lnk", IDENTITY.app));
        let bytes = std::fs::read(&link).unwrap();
        assert_eq!(
            crate::shell_link::target_path(&bytes).as_deref(),
            Some(r"C:\Apps\rules-hub.exe")
        );
        let made = app_link(IDENTITY.app, exe, LinkStart::AsClicked);
        assert_eq!(made.arguments, "");
        assert_eq!(made.show_command, 7);
        // Taken away under that name as under the title: it points at this program.
        remove_link_in(&desktop, LinkPlace::Desktop, IDENTITY.app, "rules-hub.exe").unwrap();
        assert!(!link.exists());
    }

    /// A profile of the test: `APPDATA` in a folder of its own (the Startup folder below it), never the person's.
    fn profile(name: &str) -> (Scratch, Env, PathBuf) {
        let scratch = Scratch::new(name);
        let env = Env::from_pairs([("APPDATA", scratch.path().as_os_str().to_owned())]);
        let startup = scratch
            .path()
            .join(r"Microsoft\Windows\Start Menu\Programs\Startup");
        (scratch, env, startup)
    }

    #[test]
    fn the_login_entry_of_another_app_is_read_by_its_name_and_nothing_is_written() {
        let other = AppIdentity {
            app: "rules-desktop",
            ..IDENTITY
        };
        let (_s, env, _) = profile("hub-login-other");
        // An old value in the Run key counts.
        let on = FakeRegistry::new(Some(r"C:\a\rules-desktop.exe start"));
        assert_eq!(autostart_in(&other, &env, &on), Some(true));
        let calls = on.calls();
        assert!(calls.iter().all(|c| c[0] == "query"), "{calls:?}");
        let run = calls.iter().find(|c| c[1] == RUN_KEY).unwrap();
        let at = run.iter().position(|a| a == "/v").unwrap();
        assert_eq!(run[at + 1], "rules-desktop");

        assert_eq!(
            autostart_in(&other, &env, &FakeRegistry::new(None)),
            Some(false)
        );
        // A value that a policy hides reads like one that is not there.
        let mut locked = FakeRegistry::new(None);
        locked.locked = true;
        assert_eq!(autostart_in(&other, &env, &locked), Some(false));
        // No registry to ask: nothing is known, which is not "off".
        let mut gone = FakeRegistry::new(None);
        gone.missing = true;
        assert_eq!(autostart_in(&other, &env, &gone), None);
    }

    /// On writes a link in the Startup folder (`<program> start`) and takes an old value of the Run key away; off removes
    /// the link and any old value; a link of the person's own with that name is never ours.
    #[test]
    fn the_start_with_the_login_is_a_link_in_the_startup_folder_and_an_old_run_value_goes() {
        const RULES_EXE: &str = r"C:\Apps\rules-hub.exe";
        let (_s, env, startup) = profile("hub-login-startup");
        let link = startup.join("rules-desktop.lnk");
        let reg = FakeRegistry::new(Some(r#""C:\old\rules-hub.exe" start"#));
        // An old value is on, and says how to make the link.
        let old = startup_entry(&APP, Autostart::Status, Path::new(RULES_EXE), &env, &reg).unwrap();
        assert!(
            old.starts_with("on") && old.contains("old login entry"),
            "{old}"
        );

        let said = startup_entry(&APP, Autostart::On, Path::new(RULES_EXE), &env, &reg).unwrap();
        assert!(said.contains("starts with your login"), "{said}");
        assert!(reg.value.borrow().is_none(), "the old value went");
        let bytes = std::fs::read(&link).unwrap();
        assert_eq!(
            crate::shell_link::target_path(&bytes).as_deref(),
            Some(RULES_EXE)
        );
        let made = startup_link(IDENTITY.app, Path::new(RULES_EXE));
        assert_eq!(made.arguments, "start");
        let on = startup_entry(&APP, Autostart::Status, Path::new(RULES_EXE), &env, &reg).unwrap();
        assert_eq!(on, format!("on: {}\n", link.display()));
        assert!(
            reg.calls().iter().all(|c| c[0] != "add"),
            "nothing is written to the Run key: {:?}",
            reg.calls()
        );

        startup_entry(&APP, Autostart::Off, Path::new(RULES_EXE), &env, &reg).unwrap();
        assert!(!link.exists());
        let off = startup_entry(&APP, Autostart::Status, Path::new(RULES_EXE), &env, &reg).unwrap();
        assert_eq!(off, "off\n");

        // A link of the person's own with the name of the app is not ours: not on, and off leaves it.
        crate::shell_link::write(
            &link,
            &app_link(
                "mine",
                Path::new(r"C:\Apps\editor.exe"),
                LinkStart::OpenPage,
            ),
        )
        .unwrap();
        let theirs =
            startup_entry(&APP, Autostart::Status, Path::new(RULES_EXE), &env, &reg).unwrap();
        assert_eq!(theirs, "off\n");
        startup_entry(&APP, Autostart::Off, Path::new(RULES_EXE), &env, &reg).unwrap();
        assert!(link.is_file());
    }

    #[test]
    fn the_names_come_from_the_app_and_nothing_says_tasks() {
        assert_eq!(APP.run_value(), "rules-desktop");
        let env = Env::from_pairs([
            (
                "RULES_DESKTOP_INSTALL_DIR",
                if cfg!(windows) { "C:/i" } else { "/i" },
            ),
            ("TASKS_DESKTOP_INSTALL_DIR", "/wrong"),
        ]);
        let program = installed_program(&APP, &env).unwrap();
        assert!(program.ends_with(IDENTITY.program_file()), "{program:?}");
        assert!(!program.to_string_lossy().to_lowercase().contains("tasks"));
        assert_eq!(no_login_sync(&APP), "RULES_DESKTOP_NO_LOGIN_SYNC");
        // The error names the variable of this app, not another.
        let bad = Env::from_pairs([("RULES_DESKTOP_INSTALL_DIR", "relative")]);
        let why = installed_program(&APP, &bad).unwrap_err();
        assert!(why.contains("RULES_DESKTOP_INSTALL_DIR"), "{why}");
    }

    /// The path from `autostart_for` to the registry: the value is the name of the app that was
    /// given, not a literal (the other tests of the `Run` key pass the value themselves).
    #[test]
    fn the_login_entry_is_written_under_the_name_of_the_app() {
        let reg = FakeRegistry::new(None);
        run_key_for(&APP, Autostart::On, Path::new(EXE), &reg).unwrap();
        let calls = reg.calls();
        let at = calls[0].iter().position(|a| a == "/v").unwrap();
        assert_eq!(calls[0][at + 1], "rules-desktop");
        let status = run_key_for(&APP, Autostart::Status, Path::new(EXE), &reg).unwrap();
        assert!(status.starts_with("on: "), "{status}");
        assert!(
            reg.calls()
                .iter()
                .flatten()
                .all(|a| !a.to_lowercase().contains("tasks-desktop")),
            "{:?}",
            reg.calls()
        );
    }

    /// The link says the names of the app and points at its program; only a link to the program of
    /// the app is removed.
    #[test]
    fn the_desktop_link_comes_from_the_app_and_only_ours_is_removed() {
        let desktop = Scratch::new("hub-desk");
        let exe = Path::new(r"C:\Apps\rules-hub.exe");
        let text = make_link_in(
            &desktop,
            LinkPlace::Desktop,
            IDENTITY.title,
            IDENTITY.app,
            exe,
            LinkStart::OpenPage,
        )
        .unwrap();
        assert!(text.contains("\"Rules\""), "{text}");
        let link = desktop.join("Rules.lnk");
        let bytes = std::fs::read(&link).unwrap();
        assert_eq!(
            crate::shell_link::target_path(&bytes).as_deref(),
            Some(r"C:\Apps\rules-hub.exe")
        );
        let made = app_link(IDENTITY.app, exe, LinkStart::OpenPage);
        assert_eq!(made.arguments, "start --open");
        assert_eq!(made.working_dir, r"C:\Apps");
        assert!(made.description.starts_with("rules-desktop:"));
        assert!(!made.description.to_lowercase().contains("tasks"));

        // A link of the person's own with the same name stays.
        // The name of the program on Windows, where the link is made (`program_file` has no
        // `.exe` on the system that runs the test elsewhere).
        let program = format!("{}.exe", IDENTITY.binary);
        crate::shell_link::write(
            &link,
            &app_link(
                "mine",
                Path::new(r"C:\Apps\editor.exe"),
                LinkStart::OpenPage,
            ),
        )
        .unwrap();
        remove_link_in(&desktop, LinkPlace::Desktop, IDENTITY.title, &program).unwrap();
        assert!(link.is_file());
        // Ours goes.
        make_link_in(
            &desktop,
            LinkPlace::Desktop,
            IDENTITY.title,
            IDENTITY.app,
            exe,
            LinkStart::OpenPage,
        )
        .unwrap();
        remove_link_in(&desktop, LinkPlace::Desktop, IDENTITY.title, &program).unwrap();
        assert!(!link.exists());
    }

    /// The Start menu gets the same link, also when its folder is not there yet, and says so.
    #[test]
    fn the_start_menu_link_is_the_same_link_in_a_folder_that_may_be_new() {
        let root = Scratch::new("hub-start");
        let programs = root.join("Start Menu").join("Programs");
        let exe = Path::new(r"C:\Apps\rules-hub.exe");
        let text = make_link_in(
            &programs,
            LinkPlace::StartMenu,
            IDENTITY.title,
            IDENTITY.app,
            exe,
            LinkStart::OpenPage,
        )
        .unwrap();
        assert_eq!(text, "a link \"Rules\" is in the Start menu\n");
        let bytes = std::fs::read(programs.join("Rules.lnk")).unwrap();
        assert_eq!(
            crate::shell_link::target_path(&bytes).as_deref(),
            Some(r"C:\Apps\rules-hub.exe")
        );
        let program = format!("{}.exe", IDENTITY.binary);
        let said =
            remove_link_in(&programs, LinkPlace::StartMenu, IDENTITY.title, &program).unwrap();
        assert!(said.contains("in the Start menu"), "{said}");
        assert!(!programs.join("Rules.lnk").exists());
    }

    #[cfg(windows)]
    fn desktop_folder(env: &Env, reg: &dyn Registry) -> Result<PathBuf, String> {
        shell_folder(env, reg, &LinkPlace::Desktop)
    }

    /// `reg query` answers with this value for the folder it is asked about.
    #[cfg(windows)]
    struct DesktopRegistry(Option<&'static str>);

    #[cfg(windows)]
    impl Registry for DesktopRegistry {
        fn run(&self, args: &[OsString]) -> Result<RegOutput, String> {
            let name = args
                .last()
                .and_then(|a| a.to_str())
                .unwrap_or_default()
                .to_owned();
            Ok(match self.0 {
                Some(data) => RegOutput {
                    success: true,
                    stdout: format!(
                        "\r\n{SHELL_FOLDERS_KEY}\r\n    {name}    REG_EXPAND_SZ    {data}\r\n\r\n"
                    ),
                    stderr: String::new(),
                },
                None => RegOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "ERROR: not found".to_owned(),
                },
            })
        }
    }

    /// Windows paths: only absolute on Windows.
    #[cfg(windows)]
    #[test]
    fn the_desktop_is_the_folder_the_registry_names_with_its_variables() {
        let env = Env::from_pairs([("USERPROFILE", r"C:\Users\a b"), ("OneDrive", r"D:\OD")]);
        let moved = desktop_folder(&env, &DesktopRegistry(Some(r"%OneDrive%\Desktop"))).unwrap();
        assert_eq!(moved, PathBuf::from(r"D:\OD\Desktop"));
        let plain = desktop_folder(&env, &DesktopRegistry(Some(r"C:\Desk"))).unwrap();
        assert_eq!(plain, PathBuf::from(r"C:\Desk"));
        // No value: the profile.
        assert_eq!(
            desktop_folder(&env, &DesktopRegistry(None)).unwrap(),
            PathBuf::from(r"C:\Users\a b").join("Desktop")
        );
        // A value that cannot be used is an error, not the profile: a variable that is not set,
        // a value reg.exe could not print, one that is not absolute.
        for reg in [
            DesktopRegistry(Some(r"%NOPE%\Desktop")),
            DesktopRegistry(Some("C:\\M\u{FFFD}ller\\Desktop")),
            DesktopRegistry(Some(r"Desktop")),
        ] {
            let err = desktop_folder(&env, &reg).unwrap_err();
            assert!(err.contains("cannot be used"), "{err}");
        }
        assert!(desktop_folder(&Env::default(), &DesktopRegistry(None)).is_err());
    }

    /// The Start menu is `Programs` of the same key, else under `%APPDATA%`; a value that cannot
    /// be used is an error that names the place.
    #[cfg(windows)]
    #[test]
    fn the_start_menu_is_the_programs_folder_the_registry_names() {
        let env = Env::from_pairs([("APPDATA", r"C:\Users\a\AppData\Roaming"), ("X", r"D:\X")]);
        let named = shell_folder(
            &env,
            &DesktopRegistry(Some(r"%X%\Programs")),
            &LinkPlace::StartMenu,
        );
        assert_eq!(named.unwrap(), PathBuf::from(r"D:\X\Programs"));
        assert_eq!(
            shell_folder(&env, &DesktopRegistry(None), &LinkPlace::StartMenu).unwrap(),
            PathBuf::from(r"C:\Users\a\AppData\Roaming\Microsoft\Windows\Start Menu\Programs")
        );
        let err = shell_folder(
            &env,
            &DesktopRegistry(Some(r"%NOPE%\P")),
            &LinkPlace::StartMenu,
        )
        .unwrap_err();
        assert!(
            err.contains("in the Start menu") && err.contains("cannot be used"),
            "{err}"
        );
        assert!(shell_folder(
            &Env::default(),
            &DesktopRegistry(None),
            &LinkPlace::StartMenu
        )
        .is_err());
    }

    /// `reg query` knows only the value `name` (`data`) and answers "not found" for any other.
    #[cfg(windows)]
    struct OnlyRegistry(&'static str, &'static str);

    #[cfg(windows)]
    impl Registry for OnlyRegistry {
        fn run(&self, args: &[OsString]) -> Result<RegOutput, String> {
            let asked = args.last().and_then(|a| a.to_str()) == Some(self.0);
            Ok(RegOutput {
                success: asked,
                stdout: if asked {
                    format!(
                        "\r\n{SHELL_FOLDERS_KEY}\r\n    {}    REG_EXPAND_SZ    {}\r\n\r\n",
                        self.0, self.1
                    )
                } else {
                    String::new()
                },
                stderr: String::new(),
            })
        }
    }

    /// Each place asks the registry for its own value: the Start menu is never the desktop.
    #[cfg(windows)]
    #[test]
    fn each_place_asks_for_its_own_folder() {
        let env = Env::from_pairs([
            ("APPDATA", r"C:\Users\a\AppData\Roaming"),
            ("USERPROFILE", r"C:\Users\a"),
        ]);
        let programs = OnlyRegistry("Programs", r"D:\Programs");
        assert_eq!(
            shell_folder(&env, &programs, &LinkPlace::StartMenu).unwrap(),
            PathBuf::from(r"D:\Programs")
        );
        assert_eq!(
            shell_folder(&env, &programs, &LinkPlace::Desktop).unwrap(),
            PathBuf::from(r"C:\Users\a\Desktop")
        );
        let desktop = OnlyRegistry("Desktop", r"D:\Desk");
        assert_eq!(
            shell_folder(&env, &desktop, &LinkPlace::StartMenu).unwrap(),
            PathBuf::from(r"C:\Users\a\AppData\Roaming\Microsoft\Windows\Start Menu\Programs")
        );
    }

    #[test]
    fn the_script_that_deletes_the_program_is_the_apps() {
        let said = program_left_text(&APP, Path::new("/x/rules-hub"));
        assert!(said.contains("(scripts/rules-uninstall does)"), "{said}");
        assert!(!said.contains("scripts/uninstall"), "{said}");
    }

    fn io_error(code: i32) -> std::io::Error {
        std::io::Error::from_raw_os_error(code)
    }

    #[tokio::test]
    async fn a_held_file_is_tried_again_and_the_tries_are_counted() {
        let held = |_: &std::io::Error| true;
        let mut calls = 0;
        // Held twice, free the third time.
        let ok = retry_while_held(
            || {
                calls += 1;
                if calls < 3 {
                    Err(io_error(5))
                } else {
                    Ok(())
                }
            },
            held,
            3,
            Duration::from_millis(1),
        )
        .await;
        assert!(ok.is_ok());
        assert_eq!(calls, 3);
        // Held every time: it gives up after the tries and says how many.
        let mut calls = 0;
        let (_, made) = retry_while_held(
            || {
                calls += 1;
                Err(io_error(5))
            },
            held,
            3,
            Duration::from_millis(1),
        )
        .await
        .unwrap_err();
        assert_eq!((calls, made), (3, 3));
        // Another error is reported at once.
        let mut calls = 0;
        let (_, made) = retry_while_held(
            || {
                calls += 1;
                Err(io_error(2))
            },
            |e| e.raw_os_error() == Some(5),
            3,
            Duration::from_millis(1),
        )
        .await
        .unwrap_err();
        assert_eq!((calls, made), (1, 1));
    }

    #[test]
    fn the_failure_names_the_tries_when_there_were_several() {
        let e = io_error(5);
        let once = replace_failure(Path::new("p"), 1, " (hint)", &e);
        assert!(once.starts_with("cannot replace p (hint): "), "{once}");
        assert!(!once.contains("tries"), "{once}");
        let three = replace_failure(Path::new("p"), 3, "", &e);
        assert!(
            three.starts_with("cannot replace p after 3 tries: "),
            "{three}"
        );
    }

    #[test]
    fn only_a_held_file_on_windows_is_a_moment() {
        if cfg!(windows) {
            assert!(file_is_held(&io_error(5)));
            assert!(file_is_held(&io_error(32)));
            assert!(!file_is_held(&io_error(2)));
        } else {
            assert!(!file_is_held(&io_error(13)));
        }
    }

    #[test]
    fn a_probe_is_found_in_the_store_of_a_package_and_only_there() {
        let base = Scratch::new("hub-pkg");
        let rel = Path::new("appkit-hub");
        let store = base
            .join("Packages")
            .join("Some_pkg")
            .join("LocalCache")
            .join("Roaming")
            .join(rel);
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(base.join("Packages").join("Other_pkg")).unwrap();
        assert_eq!(find_in_packages(&base, "Roaming", rel, ".probe-1"), None);
        std::fs::write(store.join(".probe-1"), b"").unwrap();
        let found = find_in_packages(&base, "Roaming", rel, ".probe-1").unwrap();
        assert!(found.ends_with("Some_pkg"));
        // Another kind of store is not this one.
        assert_eq!(find_in_packages(&base, "Local", rel, ".probe-1"), None);
    }

    #[test]
    fn a_prefix_is_taken_off_whatever_the_case() {
        let base = Path::new("a").join("Users").join("Me");
        let path = Path::new("a").join("users").join("me").join("x").join("y");
        assert_eq!(
            strip_prefix_ignore_case(&path, &base),
            Some(Path::new("x").join("y"))
        );
        // Not only ASCII: a name with an umlaut is the same in any case.
        assert_eq!(
            strip_prefix_ignore_case(
                &Path::new("a").join("MÜLLER").join("x"),
                &Path::new("a").join("müller")
            ),
            Some(PathBuf::from("x"))
        );
        assert_eq!(
            strip_prefix_ignore_case(&path, &Path::new("b").join("users")),
            None
        );
        // The base itself: nothing is left (the folder of AppData that
        // `autostart` probes).
        assert_eq!(strip_prefix_ignore_case(&base, &base), Some(PathBuf::new()));
    }

    #[cfg(windows)]
    #[test]
    fn a_packaged_app_is_noticed_in_the_local_and_the_roaming_store() {
        let base = Scratch::new("hub-warn");
        let local = base.join("Local");
        let config = base.join("Roaming").join("appkit-hub");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let env = Env::from_pairs([
            ("LOCALAPPDATA", local.clone()),
            ("APPDATA", base.join("Roaming")),
        ]);
        let probe = format!(".probe-{}", std::process::id());

        // No package holds the probe: no warning (and no probe is left behind).
        assert_eq!(private_storage_warning(&env, &local), None);
        assert_eq!(private_storage_warning(&env, &config), None);
        assert!(!local.join(&probe).exists());

        // The folder of AppData itself (what `autostart` probes: the rest is
        // empty) is looked for in the Local store of the package.
        let store = local.join("Packages").join("P_one").join("LocalCache");
        std::fs::create_dir_all(store.join("Local")).unwrap();
        std::fs::write(store.join("Local").join(&probe), b"").unwrap();
        let warning = private_storage_warning(&env, &local).unwrap();
        assert!(warning.contains("P_one"), "{warning}");
        assert!(!local.join(&probe).exists());

        // A folder below the roaming AppData in the Roaming store.
        let store = local.join("Packages").join("Q_two").join("LocalCache");
        std::fs::create_dir_all(store.join("Roaming").join("appkit-hub")).unwrap();
        std::fs::write(store.join("Roaming").join("appkit-hub").join(&probe), b"").unwrap();
        let warning = private_storage_warning(&env, &config).unwrap();
        assert!(warning.contains("Q_two"), "{warning}");
    }

    #[test]
    fn the_registered_command_quotes_the_program() {
        let cmd = run_command(Path::new(r"C:\Program Files\tasks\tasks-hub.exe"));
        assert_eq!(cmd, r#""C:\Program Files\tasks\tasks-hub.exe" start"#);
    }

    fn settings_dir(name: &str) -> Scratch {
        Scratch::new(&format!("hub-purge-{name}"))
    }

    #[test]
    fn purging_removes_the_files_of_the_app_and_the_folder_when_it_is_empty() {
        let dir = settings_dir("empty");
        for name in KIT_FILES.iter().chain(&EXTRA) {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let text = purge_settings(&dir).unwrap();
        assert!(
            text.contains("config.json") && text.contains("usage.json"),
            "{text}"
        );
        // The files the app names go too, and no other app's file is in the list.
        assert!(
            text.contains("rules.json") && text.contains("waivers.json"),
            "{text}"
        );
        assert!(!text.contains("areas.json"), "{text}");
        assert!(text.contains("folder is removed"), "{text}");
        assert!(!dir.exists());
        // Nothing there at all: told, and no error.
        assert!(purge_settings(&dir).unwrap().contains("no settings"));
    }

    #[test]
    fn purging_leaves_a_file_that_is_not_the_apps_and_so_the_folder() {
        let dir = settings_dir("other");
        std::fs::write(dir.join("config.json"), "x").unwrap();
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        let text = purge_settings(&dir).unwrap();
        assert!(text.contains("holds other files"), "{text}");
        assert!(!dir.join("config.json").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("notes.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn a_hub_in_the_background_is_told_so_and_one_in_a_terminal_is_not() {
        assert_eq!(detached_var(&APP), "RULES_DESKTOP_DETACHED");
        assert!(!runs_detached(&APP, &Env::default()));
        assert!(runs_detached(
            &APP,
            &Env::from_pairs([("RULES_DESKTOP_DETACHED", "1")])
        ));
    }

    #[test]
    fn start_waits_twenty_seconds_unless_a_sane_number_of_seconds_is_named() {
        let limit =
            |v: &str| start_limit(&APP, &Env::from_pairs([("RULES_DESKTOP_START_SECS", v)]));
        assert_eq!(start_limit(&APP, &Env::default()), Duration::from_secs(20));
        assert_eq!(limit("90"), Duration::from_secs(90));
        assert_eq!(limit(" 3600 "), Duration::from_secs(3600));
        for ignored in ["0", "3601", "-1", "soon", ""] {
            assert_eq!(limit(ignored), Duration::from_secs(20), "{ignored:?}");
        }
    }

    /// A hub that fails at once (a broken `config.json`) is reported at once, with what it wrote to its log;
    /// what an earlier start wrote there is not taken for the cause.
    #[tokio::test]
    async fn a_hub_that_ends_with_a_failure_is_reported_at_once_with_its_cause() {
        let scratch = Scratch::new("bg-start-fails");
        let dir = scratch.path().to_path_buf();
        let log_path = dir.join("hub.log");
        std::fs::write(&log_path, "error: an old start\n").unwrap();
        let log = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        let from = log.metadata().unwrap().len();
        let said =
            "invalid settings file config.json: EOF while parsing a value at line 1 column 0";
        #[cfg(windows)]
        let mut command = {
            let mut c = Command::new("cmd");
            c.args(["/d", "/c", &format!("echo {said}\x1b[0m 1>&2 & exit /b 1")]);
            c
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut c = Command::new("sh");
            let script = format!("printf '%s\\033[0m\\nnote: run with it\\n' '{said}' >&2; exit 1");
            c.args(["-c", &script]);
            c
        };
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let began = Instant::now();
        let got = wait_for_hub(&APP, &dir, child, from, Duration::from_secs(60)).await;
        let err = got.unwrap_err();
        assert!(
            began.elapsed() < Duration::from_secs(15),
            "{:?}",
            began.elapsed()
        );
        assert!(
            err.starts_with("the hub ended before it came up: "),
            "{err}"
        );
        assert!(err.contains(said), "{err}");
        assert!(!err.contains('\x1b') && !err.contains("old start"), "{err}");
        assert!(err.contains(&log_path.display().to_string()), "{err}");
    }

    /// A hub that ends with exit code 0 before it came up (a `stop` that came first, a window hub that was
    /// quit) is reported at once too, not after the whole limit; with nothing in its log the exit code is the cause.
    #[tokio::test]
    async fn a_hub_that_ends_with_exit_code_0_is_reported_at_once() {
        let scratch = Scratch::new("bg-start-ends-0");
        let dir = scratch.path().to_path_buf();
        let log_path = dir.join("hub.log");
        let log = std::fs::File::create(&log_path).unwrap();
        #[cfg(windows)]
        let mut command = {
            let mut c = Command::new("cmd");
            c.args(["/d", "/c", "exit /b 0"]);
            c
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut c = Command::new("sh");
            c.args(["-c", "exit 0"]);
            c
        };
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let began = Instant::now();
        let got = wait_for_hub(&APP, &dir, child, 0, Duration::from_secs(60)).await;
        let err = got.unwrap_err();
        assert!(
            began.elapsed() < Duration::from_secs(15),
            "{:?}",
            began.elapsed()
        );
        assert!(
            err.starts_with("the hub ended before it came up: it ended with exit code 0; see "),
            "{err}"
        );
        assert!(err.contains(&log_path.display().to_string()), "{err}");
    }

    #[test]
    fn the_cause_is_the_last_line_that_says_something_cut_and_plain() {
        let scratch = Scratch::new("bg-last-line");
        let log = scratch.join("hub.log");
        let long = "x".repeat(400);
        let text = format!("before\nerror: one\r\n{long}\n\nnote: run with `RUST_BACKTRACE=1`\n");
        std::fs::write(&log, text).unwrap();
        let got = last_log_line(&log, 7).unwrap();
        assert_eq!(got, format!("{}...", "x".repeat(300)));
        std::fs::write(&log, "a\u{7}b\n").unwrap();
        assert_eq!(last_log_line(&log, 0).as_deref(), Some("ab"));
        // A log that was cut since is read from its start; an empty one says nothing.
        assert_eq!(last_log_line(&log, 1000).as_deref(), Some("ab"));
        std::fs::write(&log, "").unwrap();
        assert_eq!(last_log_line(&log, 0), None);
        assert_eq!(last_log_line(&scratch.join("none"), 0), None);
    }

    #[test]
    fn the_time_alone_is_ten_minutes_unless_a_sane_number_of_seconds_is_named() {
        let grace =
            |v: &str| alone_grace(&APP, &Env::from_pairs([("RULES_DESKTOP_ALONE_SECS", v)]));
        assert_eq!(alone_grace(&APP, &Env::default()), Duration::from_secs(600));
        assert_eq!(grace("2"), Duration::from_secs(2));
        assert_eq!(grace(" 86400 "), Duration::from_secs(86_400));
        for ignored in ["0", "86401", "-1", "soon", ""] {
            assert_eq!(grace(ignored), Duration::from_secs(600), "{ignored:?}");
        }
    }

    /// What the person switched on is an order; the login entry of the machine the tests run on is not asked.
    #[test]
    fn the_setting_and_a_paired_device_are_orders_to_stay() {
        let env = Env::from_pairs([("RULES_DESKTOP_NO_LOGIN_SYNC", "1")]);
        assert!(!has_order(&APP, &env, false, 0));
        assert!(has_order(&APP, &env, true, 0));
        assert!(has_order(&APP, &env, false, 1));
    }

    /// A hub that hub-desktop starts with itself is told to stay by the variable of its starter, which
    /// `start` passes on (the login entries are not asked here either).
    #[test]
    fn the_word_of_the_starter_is_an_order_to_stay() {
        assert_eq!(stay_var(APP.identity()), "RULES_DESKTOP_STAY");
        let env = Env::from_pairs([
            ("RULES_DESKTOP_NO_LOGIN_SYNC", "1"),
            ("RULES_DESKTOP_STAY", "1"),
        ]);
        assert!(has_order(&APP, &env, false, 0));
    }

    #[test]
    fn only_a_start_that_started_a_hub_says_so() {
        assert!(started_now(
            "the hub runs in the background (port 1, process 2)\n"
        ));
        assert!(!started_now(
            "a hub is already running (port 1, process 2)\n"
        ));
        assert!(!started_now(""));
    }

    #[tokio::test]
    async fn a_hub_in_a_terminal_is_never_left_alone() {
        let presence = Presence::new();
        let env = Env::from_pairs([("RULES_DESKTOP_ALONE_SECS", "1")]);
        let waiting = left_alone(&APP, &env, &presence, || false);
        assert!(tokio::time::timeout(Duration::from_millis(1500), waiting)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_hub_in_the_background_is_left_alone_after_the_grace_unless_it_has_an_order() {
        let env = Env::from_pairs([
            ("RULES_DESKTOP_DETACHED", "1"),
            ("RULES_DESKTOP_ALONE_SECS", "1"),
        ]);
        let presence = Presence::new();
        let began = Instant::now();
        tokio::time::timeout(
            Duration::from_secs(10),
            left_alone(&APP, &env, &presence, || false),
        )
        .await
        .expect("nobody came");
        assert!(began.elapsed() >= Duration::from_secs(1));
        let staying = left_alone(&APP, &env, &presence, || true);
        assert!(tokio::time::timeout(Duration::from_millis(1500), staying)
            .await
            .is_err());
    }

    #[test]
    fn install_takes_back_only_an_entry_it_made() {
        fn entry(
            calls: &std::cell::RefCell<Vec<Autostart>>,
        ) -> impl Fn(Autostart) -> Result<String, String> + '_ {
            move |what: Autostart| {
                calls.borrow_mut().push(what);
                Ok("entry\n".to_owned())
            }
        }
        let with = InstallOptions {
            autostart: true,
            ..InstallOptions::default()
        };
        let calls = std::cell::RefCell::new(Vec::new());
        let mut out = String::new();
        // Asked for and not there before: made here.
        assert!(login_entry_step(with, false, true, entry(&calls), &mut out).unwrap());
        // There before (for another copy): pointed here, and not this install's to take back.
        assert!(!login_entry_step(with, true, true, entry(&calls), &mut out).unwrap());
        assert!(!login_entry_step(
            InstallOptions::default(),
            true,
            true,
            entry(&calls),
            &mut out
        )
        .unwrap());
        assert_eq!(calls.borrow().len(), 3);
        // Not asked for and not there: nothing is written.
        assert!(!login_entry_step(
            InstallOptions::default(),
            false,
            true,
            entry(&calls),
            &mut out
        )
        .unwrap());
        assert_eq!(calls.borrow().len(), 3);

        // The tests of the program (no sync): an entry that is there is not pointed here, and is still not made here.
        assert!(!login_entry_step(with, true, false, entry(&calls), &mut out).unwrap());
        assert_eq!(
            calls.borrow().len(),
            4,
            "--autostart writes it all the same"
        );
        assert!(!login_entry_step(
            InstallOptions::default(),
            true,
            false,
            entry(&calls),
            &mut out
        )
        .unwrap());
        assert_eq!(calls.borrow().len(), 4);

        calls.borrow_mut().clear();
        let failed = undo_entry_on_failure(Err("no link".to_owned()), true, entry(&calls));
        assert_eq!(*calls.borrow(), [Autostart::Off]);
        assert!(failed.unwrap_err().contains("taken back"));
        calls.borrow_mut().clear();
        assert_eq!(
            undo_entry_on_failure(Err("no link".to_owned()), false, entry(&calls)),
            Err("no link".to_owned())
        );
        assert_eq!(
            undo_entry_on_failure(Ok("link\n".to_owned()), true, entry(&calls)),
            Ok("link\n".to_owned())
        );
        assert!(
            calls.borrow().is_empty(),
            "nothing failed, nothing is undone"
        );
    }

    /// `install --autostart --shortcut` where the link on the desktop cannot be made (no profile folder for it): the
    /// install fails, and the link in the Startup folder it made a moment before is gone again.
    #[cfg(windows)]
    #[test]
    fn an_install_that_fails_after_the_login_entry_leaves_no_startup_link() {
        let opts = InstallOptions {
            autostart: true,
            shortcut: true,
            ..InstallOptions::default()
        };
        let target = Path::new(r"C:\apps\rules-hub.exe");
        // APPDATA (the Startup folder and the Start menu) but no USERPROFILE (the desktop).
        let (_s, env, startup) = profile("hub-install-undo");
        let link = startup.join("rules-desktop.lnk");
        let reg = FakeRegistry::new(None);
        let failed = entry_and_links(&APP, target, opts, &env, Ok(&reg));
        let why = failed.expect_err("no folder for the link on the desktop");
        assert!(why.contains("taken back"), "{why}");
        assert!(
            !link.exists(),
            "the link of the login went with the failure"
        );
        assert!(
            reg.calls().iter().all(|c| c[0] != "add"),
            "{:?}",
            reg.calls()
        );

        // An entry that was there before the install stays: it is the person's, pointed at the new copy.
        crate::shell_link::write(
            &link,
            &startup_link(IDENTITY.app, Path::new(r"C:\old\rules-hub.exe")),
        )
        .unwrap();
        assert!(entry_and_links(&APP, target, opts, &env, Ok(&reg)).is_err());
        let bytes = std::fs::read(&link).unwrap();
        assert_eq!(
            crate::shell_link::target_path(&bytes).as_deref(),
            Some(r"C:\apps\rules-hub.exe")
        );
    }

    /// With `NO_LOGIN_SYNC` too: the entry that was there before a failed install is still there after it.
    #[cfg(windows)]
    #[test]
    fn a_failed_install_never_removes_a_login_entry_that_was_there_before() {
        let opts = InstallOptions {
            autostart: true,
            shortcut: true,
            ..InstallOptions::default()
        };
        let (scratch, _, startup) = profile("hub-install-kept");
        let env = Env::from_pairs([
            ("APPDATA", scratch.path().as_os_str().to_owned()),
            ("RULES_DESKTOP_NO_LOGIN_SYNC", std::ffi::OsString::from("1")),
        ]);
        std::fs::create_dir_all(&startup).unwrap();
        let link = startup.join("rules-desktop.lnk");
        crate::shell_link::write(
            &link,
            &startup_link(IDENTITY.app, Path::new(r"C:\old\rules-hub.exe")),
        )
        .unwrap();
        let reg = FakeRegistry::new(None);
        let failed = entry_and_links(
            &APP,
            Path::new(r"C:\apps\rules-hub.exe"),
            opts,
            &env,
            Ok(&reg),
        );
        let why = failed.expect_err("no folder for the link on the desktop");
        assert!(!why.contains("taken back"), "{why}");
        assert!(link.is_file());
    }

    /// Without `--autostart` a registry that cannot be asked stops nothing: there is no entry to point here.
    #[cfg(windows)]
    #[test]
    fn an_install_without_a_login_entry_does_not_need_the_registry() {
        let said = entry_and_links(
            &APP,
            Path::new(r"C:\apps\rules-hub.exe"),
            InstallOptions::default(),
            &Env::default(),
            Err("no Windows folder found".to_owned()),
        )
        .expect("the install goes on");
        assert!(said.contains("no link in the Start menu"), "{said}");
    }

    #[test]
    fn the_start_script_of_an_older_version_goes_with_the_uninstall() {
        let dir = appkit_scratch::Scratch::new("old-start-script");
        std::fs::write(dir.join("start-hub.vbs"), "x").unwrap();
        std::fs::write(dir.join("config.json"), "{}").unwrap();
        remove_old_start_script(&dir);
        assert!(!dir.join("start-hub.vbs").exists());
        assert!(dir.join("config.json").is_file());
        remove_old_start_script(&dir);
    }

    /// A registry that holds one value in memory and says what it was asked.
    struct FakeRegistry {
        value: std::cell::RefCell<Option<String>>,
        calls: std::cell::RefCell<Vec<Vec<String>>>,
        /// A policy or an ACL: `add` and `delete` fail and change nothing.
        locked: bool,
        /// `reg.exe` is not there.
        missing: bool,
    }

    impl FakeRegistry {
        fn new(value: Option<&str>) -> Self {
            Self {
                value: std::cell::RefCell::new(value.map(str::to_owned)),
                calls: std::cell::RefCell::new(Vec::new()),
                locked: false,
                missing: false,
            }
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl Registry for FakeRegistry {
        fn run(&self, args: &[OsString]) -> Result<RegOutput, String> {
            let args: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.calls.borrow_mut().push(args.clone());
            if self.missing {
                return Err("reg.exe did not run: not found".to_owned());
            }
            let answer = |success: bool, stdout: String| {
                Ok(RegOutput {
                    success,
                    stdout,
                    // What reg.exe says when a policy or an ACL refuses.
                    stderr: if !success && self.locked {
                        "
ERROR: Access is denied.
"
                        .to_owned()
                    } else {
                        String::new()
                    },
                })
            };
            match args.first().map(String::as_str) {
                Some("add") if !self.locked => {
                    let at = args.iter().position(|a| a == "/d").unwrap_or(0) + 1;
                    *self.value.borrow_mut() = args.get(at).cloned();
                    answer(true, String::new())
                }
                Some("delete") if !self.locked => {
                    let had = self.value.borrow_mut().take().is_some();
                    answer(had, String::new())
                }
                Some("query") if args.get(1).map(String::as_str) != Some(RUN_KEY) => {
                    answer(false, String::new())
                }
                Some("query") => match self.value.borrow().as_deref() {
                    Some(v) => answer(
                        true,
                        format!("\r\nHKEY_CURRENT_USER\\Software\r\n    {RUN_VALUE}    REG_SZ    {v}\r\n\r\n"),
                    ),
                    None => answer(false, String::new()),
                },
                _ => answer(false, String::new()),
            }
        }
    }

    /// A registry that holds the `Uninstall` entries it is given: key -> data of `InstallLocation`
    /// (`None`: the key has no such value).
    struct UninstallRegistry(Vec<(String, Option<&'static str>)>);

    impl Registry for UninstallRegistry {
        fn run(&self, args: &[OsString]) -> Result<RegOutput, String> {
            let args: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let entry = self.0.iter().find(|(k, _)| Some(k) == args.get(1));
            let stdout = match (entry, args.get(2).map(String::as_str)) {
                (Some((k, Some(at))), Some("/v")) => {
                    format!("\r\n{k}\r\n    InstallLocation    REG_SZ    {at}\r\n\r\n")
                }
                (Some((k, _)), None) => format!("\r\n{k}\r\n    Publisher    REG_SZ    x\r\n"),
                _ => String::new(),
            };
            let success = !stdout.is_empty();
            Ok(RegOutput {
                success,
                stdout,
                stderr: String::new(),
            })
        }
    }

    fn setup_key() -> String {
        setup_key_in(0)
    }

    /// The entry of this app under one of the `Uninstall` keys (0: the user, 1: the machine, 2: its 32-bit view).
    fn setup_key_in(which: usize) -> String {
        format!(r"{}\rules-desktop", UNINSTALL_KEYS[which])
    }

    #[test]
    fn a_setup_installation_is_found_by_its_uninstall_entry_of_this_app_only() {
        let at = r"C:\Users\a b\AppData\Local\rules-desktop";
        let reg = UninstallRegistry(vec![(setup_key(), Some(at))]);
        let found = setup_install_in(&APP, &reg).unwrap().unwrap();
        assert_eq!(found.location, Some(PathBuf::from(at)));
        // The entry of another app is not ours.
        let other = UninstallRegistry(vec![(
            format!(r"{}\tasks-desktop", UNINSTALL_KEYS[0]),
            Some(at),
        )]);
        assert_eq!(setup_install_in(&APP, &other).unwrap(), None);
        // Nothing is installed by a setup.
        assert_eq!(
            setup_install_in(&APP, &UninstallRegistry(Vec::new())).unwrap(),
            None
        );
    }

    #[test]
    fn a_setup_for_all_users_is_found_too_in_either_view_of_the_machine() {
        let at = r"C:\Program Files\rules-desktop";
        for which in [1, 2] {
            let reg = UninstallRegistry(vec![(setup_key_in(which), Some(at))]);
            let found = setup_install_in(&APP, &reg).unwrap().unwrap();
            assert_eq!(found.location, Some(PathBuf::from(at)), "key {which}");
            // ... also an entry of the machine without a location.
            let bare = UninstallRegistry(vec![(setup_key_in(which), None)]);
            assert_eq!(
                setup_install_in(&APP, &bare).unwrap(),
                Some(SetupInstall { location: None }),
                "key {which}"
            );
        }
        // The entry of the user answers first when both are there.
        let both = UninstallRegistry(vec![
            (setup_key_in(1), Some(at)),
            (setup_key_in(0), Some(r"C:\Users\a\rules-desktop")),
        ]);
        let found = setup_install_in(&APP, &both).unwrap().unwrap();
        assert_eq!(
            found.location,
            Some(PathBuf::from(r"C:\Users\a\rules-desktop"))
        );
    }

    #[tokio::test]
    async fn install_refuses_before_it_touches_anything_when_a_setup_installed_the_app() {
        let found = SetupInstall {
            location: Some(PathBuf::from("/x/rules")),
        };
        let mut out = String::new();
        let why = install_steps_beside(&APP, InstallOptions::default(), &mut out, Some(found))
            .await
            .unwrap_err();
        assert!(
            why.contains("run the setup again") && why.contains("uninstaller"),
            "{why}"
        );
        assert_eq!(out, "", "something was done before the refusal");
    }

    /// The closing of `uninstall` in a config folder of its own: what it says, with and without a setup, and what
    /// `--purge-settings` removes (the files of the kit, nothing else).
    #[test]
    fn the_closing_of_uninstall_keeps_or_removes_the_settings_and_names_a_setup() {
        let dir = Scratch::new("uninstall-closing");
        std::fs::write(dir.join("config.json"), "{}").unwrap();
        std::fs::write(dir.join("mine.txt"), "not the kit's").unwrap();
        let env = Env::from_pairs([(
            APP.identity().var("CONFIG_DIR"),
            dir.to_string_lossy().into_owned(),
        )]);
        let exe = Path::new("/x/rules-desktop");
        let setup = SetupInstall {
            location: Some(PathBuf::from("/x/setup")),
        };

        // Kept, with a setup: the folder, the program file and the uninstaller of the setup are named.
        let (mut out, mut failed) = (String::new(), Vec::new());
        uninstall_closing(
            &APP,
            &env,
            UninstallOptions::default(),
            exe,
            Some(&setup),
            &mut out,
            &mut failed,
        );
        assert!(failed.is_empty(), "{failed:?}");
        assert!(out.contains("your settings stay in"), "{out}");
        assert!(out.contains("the program file is left at"), "{out}");
        assert!(out.contains("uninstaller of the setup"), "{out}");
        assert!(dir.join("config.json").exists());

        // Without a setup: no line about one.
        let (mut plain, mut failed) = (String::new(), Vec::new());
        uninstall_closing(
            &APP,
            &env,
            UninstallOptions::default(),
            exe,
            None,
            &mut plain,
            &mut failed,
        );
        assert!(!plain.contains("setup"), "{plain}");

        // Purged: the file of the kit goes, a file that is not the kit's stays.
        let (mut purged, mut failed) = (String::new(), Vec::new());
        let opts = UninstallOptions {
            purge_settings: true,
        };
        uninstall_closing(&APP, &env, opts, exe, None, &mut purged, &mut failed);
        assert!(failed.is_empty(), "{failed:?}");
        assert!(!dir.join("config.json").exists());
        assert!(dir.join("mine.txt").exists());
    }

    #[test]
    fn uninstall_names_the_files_of_a_setup_as_the_uninstallers_and_says_nothing_without_one() {
        let mut out = String::from("before\n");
        push_setup_note(&mut out, None);
        assert_eq!(out, "before\n");
        push_setup_note(
            &mut out,
            Some(&SetupInstall {
                location: Some(PathBuf::from("/x/rules")),
            }),
        );
        assert!(
            out.contains("uninstaller") && out.contains("rules"),
            "{out}"
        );
    }

    #[test]
    fn an_entry_without_a_location_is_an_installation_all_the_same() {
        let reg = UninstallRegistry(vec![(setup_key(), None)]);
        let found = setup_install_in(&APP, &reg).unwrap().unwrap();
        assert_eq!(found.location, None);
        // A registry that cannot be run is an error, not "nothing installed".
        let mut gone = FakeRegistry::new(None);
        gone.missing = true;
        assert!(setup_install_in(&APP, &gone).is_err());
    }

    #[test]
    fn the_refusal_names_the_way_to_repair_and_to_remove() {
        let said = setup_refusal(&SetupInstall {
            location: Some(PathBuf::from("/x/rules")),
        });
        assert!(
            said.contains("/x/rules") || said.contains(r"\x\rules"),
            "{said}"
        );
        assert!(
            said.contains("run the setup again") && said.contains("uninstaller"),
            "{said}"
        );
        assert!(!setup_refusal(&SetupInstall { location: None }).contains("()"));
        assert!(setup_left_text(&SetupInstall { location: None }).contains("uninstaller"));
    }

    const EXE: &str = r"C:\Program Files\tasks\tasks-hub.exe";

    #[test]
    fn turning_it_on_writes_the_quoted_command_under_the_run_key_of_the_user() {
        let reg = FakeRegistry::new(None);
        let text = run_key(Autostart::On, Path::new(EXE), &reg).unwrap();
        let wanted: Vec<String> = [
            "add",
            RUN_KEY,
            "/v",
            RUN_VALUE,
            "/t",
            "REG_SZ",
            "/d",
            &run_command(Path::new(EXE)),
            "/f",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(reg.calls(), vec![wanted]);
        assert!(
            RUN_KEY.starts_with(r"HKCU\"),
            "the key of the user needs no administrator"
        );
        assert!(text.contains("starts with your login"), "{text}");
        assert_eq!(
            reg.value.borrow().as_deref(),
            Some(run_command(Path::new(EXE)).as_str())
        );
    }

    #[test]
    fn the_reason_reg_gives_for_a_refusal_is_in_the_error() {
        let mut reg = FakeRegistry::new(Some("x"));
        reg.locked = true;
        let written = run_key(Autostart::On, Path::new(EXE), &reg).unwrap_err();
        assert_eq!(
            written,
            "the entry could not be written: ERROR: Access is denied."
        );
        let removed = run_key(Autostart::Off, Path::new(EXE), &reg).unwrap_err();
        assert_eq!(
            removed,
            "the entry could not be removed: ERROR: Access is denied."
        );
        // A control character in the complaint does not reach the terminal.
        let odd = RegOutput {
            success: false,
            stdout: String::new(),
            stderr: format!("bad{}thing", char::from(0x1b)),
        };
        assert_eq!(odd.reason(), ": bad thing");
    }

    #[test]
    fn a_refused_write_is_an_error_and_a_missing_program_too() {
        let mut reg = FakeRegistry::new(None);
        reg.locked = true;
        assert!(run_key(Autostart::On, Path::new(EXE), &reg)
            .unwrap_err()
            .starts_with("the entry could not be written"));
        let mut gone = FakeRegistry::new(None);
        gone.missing = true;
        for what in [Autostart::On, Autostart::Off, Autostart::Status] {
            let error = run_key(what, Path::new(EXE), &gone).unwrap_err();
            assert!(error.contains("did not run"), "{what:?}: {error}");
        }
    }

    #[test]
    fn turning_it_off_removes_the_value_and_a_missing_one_is_what_was_asked_for() {
        let reg = FakeRegistry::new(Some("x"));
        let first = run_key(Autostart::Off, Path::new(EXE), &reg).unwrap();
        assert!(first.contains("does not start"), "{first}");
        assert!(reg.value.borrow().is_none());
        // Again: the delete fails (nothing there), the key is asked, and that is fine.
        let again = run_key(Autostart::Off, Path::new(EXE), &reg).unwrap();
        assert!(again.contains("does not start"), "{again}");
        let last = reg.calls().last().cloned().unwrap();
        assert_eq!(
            last[0], "query",
            "a failed delete is followed by a look at the key"
        );
    }

    #[test]
    fn a_value_that_cannot_be_removed_is_an_error() {
        let mut reg = FakeRegistry::new(Some("x"));
        reg.locked = true;
        assert!(run_key(Autostart::Off, Path::new(EXE), &reg)
            .unwrap_err()
            .starts_with("the entry could not be removed"));
        assert!(reg.value.borrow().is_some());
    }

    #[test]
    fn the_status_names_the_value_or_says_off() {
        let reg = FakeRegistry::new(Some(r#""C:\x\tasks-hub.exe" start"#));
        let on = run_key(Autostart::Status, Path::new(EXE), &reg).unwrap();
        assert!(on.starts_with("on: "), "{on}");
        assert!(
            on.contains(RUN_VALUE) && on.contains("tasks-hub.exe"),
            "{on}"
        );
        assert!(
            !on.contains('\r'),
            "no line ending of reg.exe is left in it: {on:?}"
        );
        let off = run_key(Autostart::Status, Path::new(EXE), &FakeRegistry::new(None)).unwrap();
        assert_eq!(off, "off\n");
    }

    /// `reg query` names the desktop and the Start menu (`Programs`) as these folders.
    #[cfg(windows)]
    struct FolderRegistry {
        desktop: String,
        programs: String,
    }

    #[cfg(windows)]
    impl Registry for FolderRegistry {
        fn run(&self, args: &[OsString]) -> Result<RegOutput, String> {
            let name = args
                .last()
                .and_then(|a| a.to_str())
                .unwrap_or_default()
                .to_owned();
            let data = if name == "Desktop" {
                &self.desktop
            } else {
                &self.programs
            };
            Ok(RegOutput {
                success: true,
                stdout: format!(
                    "\r\n{SHELL_FOLDERS_KEY}\r\n    {name}    REG_EXPAND_SZ    {data}\r\n\r\n"
                ),
                stderr: String::new(),
            })
        }
    }

    /// `install` on Windows: the Start menu always, the desktop only with `--shortcut`, nothing automatic
    /// with `NO_LOGIN_SYNC`; a link of the Start menu that cannot be made does not fail the install, one
    /// asked for with `--shortcut` does.
    #[cfg(windows)]
    #[test]
    fn install_links_the_start_menu_always_and_the_desktop_only_when_asked() {
        let root = Scratch::new("hub-install-links");
        let desktop = root.join("Desktop");
        let programs = root.join("Programs");
        let reg = FolderRegistry {
            desktop: desktop.to_string_lossy().into_owned(),
            programs: programs.to_string_lossy().into_owned(),
        };
        let exe = Path::new(r"C:\Apps\rules-hub.exe");
        let link = |dir: &Path| dir.join(desktop_link_name(IDENTITY.title));
        let plain = InstallOptions::default();
        let asked = InstallOptions {
            shortcut: true,
            ..InstallOptions::default()
        };

        let said = install_links(&APP, exe, plain, &Env::default(), Ok(&reg)).unwrap();
        assert!(said.contains("in the Start menu"), "{said}");
        assert!(link(&programs).is_file());
        assert!(
            !link(&desktop).exists(),
            "no desktop link without --shortcut"
        );

        std::fs::remove_file(link(&programs)).unwrap();
        let said = install_links(&APP, exe, asked, &Env::default(), Ok(&reg)).unwrap();
        assert!(said.contains("on the desktop"), "{said}");
        assert!(link(&programs).is_file() && link(&desktop).is_file());

        std::fs::remove_file(link(&programs)).unwrap();
        std::fs::remove_file(link(&desktop)).unwrap();
        let quiet = Env::from_pairs([("RULES_DESKTOP_NO_LOGIN_SYNC", "1")]);
        let said = install_links(&APP, exe, plain, &quiet, Ok(&reg)).unwrap();
        assert_eq!(said, "");
        assert!(!link(&programs).exists() && !link(&desktop).exists());

        // A Start menu that cannot be found is said, the install goes on; with --shortcut it is an error.
        let broken = OnlyRegistry("Programs", r"%NOPE%\Programs");
        let said = install_links(&APP, exe, plain, &Env::default(), Ok(&broken)).unwrap();
        assert!(said.starts_with("no link in the Start menu: "), "{said}");
        let said = install_links(
            &APP,
            exe,
            plain,
            &Env::default(),
            Err("no reg.exe".to_owned()),
        );
        assert_eq!(said.unwrap(), "no link in the Start menu: no reg.exe\n");
        let err = install_links(
            &APP,
            exe,
            asked,
            &Env::default(),
            Err("no reg.exe".to_owned()),
        );
        assert_eq!(err.unwrap_err(), "no reg.exe");
    }

    /// `install` ends with the call that starts the program, and says whether its folder is on PATH (by the
    /// name of the program, and on Windows the entry of the Start menu).
    #[test]
    fn install_names_the_whole_call_and_the_path() {
        // Whole paths of no folder that exists: only compared, nothing is made.
        let root = PathBuf::from(if cfg!(windows) {
            "C:/rules-hub-test"
        } else {
            "/rules-hub-test"
        });
        let dir = root.join("bin");
        let target = dir.join(IDENTITY.program_file());
        let elsewhere = std::env::join_paths([root.join("elsewhere")]).unwrap();
        let off = how_to_start(&APP, &target, Some(&elsewhere));
        assert!(
            off.starts_with(&format!(
                "start it: \"{}\" start --open\n",
                target.display()
            )),
            "{off}"
        );
        assert!(off.contains("not on PATH"), "{off}");
        assert!(off.contains("to call `rules-hub` by name"), "{off}");
        assert!(off.contains(&dir.display().to_string()), "{off}");
        assert_eq!(
            off.contains("the Start menu entry Rules starts it too"),
            cfg!(windows),
            "{off}"
        );
        let none = how_to_start(&APP, &target, None);
        assert!(none.contains("not on PATH"), "{none}");
        let with = std::env::join_paths([root.join("elsewhere"), dir.clone()]).unwrap();
        let on = how_to_start(&APP, &target, Some(&with));
        assert!(on.contains("is on PATH: `rules-hub start --open`"), "{on}");
        assert!(!on.contains("not on PATH"), "{on}");
        if cfg!(windows) {
            // Windows does not tell the case of a folder apart, nor a trailing separator.
            let upper = OsString::from(format!("{}\\", dir.display().to_string().to_uppercase()));
            assert!(how_to_start(&APP, &target, Some(&upper)).contains("is on PATH"));
        }
    }
}
