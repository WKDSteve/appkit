//! The commands every hub app shares, with the same words and the same exit codes: an app reads its arguments
//! with [`parse`], runs the common commands (for `status`, `logs` and `paths` with the functions here) and keeps its
//! own (`serve`, `setup`, `phone`, ...) for what [`parse`] leaves to it ([`Parsed::Own`]).
//!
//! | Command | What it does |
//! |---|---|
//! | (no argument) | start the hub when it is not running, and open the page (a double click) |
//! | `start [--open]` | run the hub in the background; `--open` also opens the page |
//! | `stop`, `quit` | end the hub that runs in the background |
//! | `restart [--open]` | `stop`, then `start` |
//! | `status [--json]` | whether the hub runs, with its process, port and version; exit code 0 runs, 1 does not |
//! | `open` | open the page, starting the hub when it is not running (`open --launch` is still taken) |
//! | `link` | print the one-time pairing link of the running hub (for a phone, a tunnel, a script) |
//! | `logs [-f] [-n N]` | the last lines of the log of the hub in the background; `-f` follows it |
//! | `paths` | the program, the installed program, the settings, the log and the command |
//! | `version`, `--version`, `-V` | the version |
//! | `help [<command>]`, `--help`, `-h`, `<command> --help` | what the commands do |
//! | `completions powershell`, `completions bash` | print the script for tab completion ([`completions_script`]) |
//!
//! Exit codes: 0 done, 1 failed (or, for `status`, not running), 2 the arguments were wrong. `close` is not a
//! command: it would read as "close the window, the hub runs on"; it is answered with a pointer to `stop`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use appkit_core::registry::HubRecord;
use serde_json::json;

use crate::background::{self, Background};
use crate::cli_entry;
use crate::env::Env;
use crate::instance;

/// The exit code of wrong arguments.
pub const USAGE_EXIT: i32 = 2;

/// How many lines `logs` shows when not told.
pub const DEFAULT_LOG_LINES: usize = 40;

/// The most lines `logs -n` shows.
const MAX_LOG_LINES: usize = 10_000;

/// A common command, as [`parse`] read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    /// No argument: as a double click.
    Click,
    Start {
        open: bool,
    },
    /// `stop` or `quit`.
    Stop,
    Restart {
        open: bool,
    },
    Status {
        json: bool,
    },
    /// `open`, also `open --launch` (what earlier versions called it).
    Open,
    Link,
    Logs {
        lines: usize,
        follow: bool,
    },
    Paths,
    Version,
    /// `completions powershell` or `completions bash`.
    Completions(Shell),
    /// `help`, `--help`, `-h` (`None`), or `help <command>` and `<command> --help`.
    Help(Option<String>),
}

/// The shells that `completions` writes a script for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    PowerShell,
    Bash,
}

/// What the arguments are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// A common command.
    Common(Verb),
    /// Not a common command (or a common one with words of the app, such as `open --phone`): the app's to read.
    Own,
    /// Wrong: the text says why (exit code [`USAGE_EXIT`]).
    Usage(String),
}

/// The words of a request for help.
fn is_help(word: &str) -> bool {
    matches!(word, "--help" | "-h")
}

/// Read the arguments (without the program name).
#[must_use]
pub fn parse(args: &[String]) -> Parsed {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let verb = match words.as_slice() {
        [] => Verb::Click,
        ["help" | "--help" | "-h"] => Verb::Help(None),
        ["help", command] => Verb::Help(Some((*command).to_owned())),
        [command, help] if is_help(help) => Verb::Help(Some((*command).to_owned())),
        ["version" | "--version" | "-V"] => Verb::Version,
        ["start"] => Verb::Start { open: false },
        ["start", "--open"] => Verb::Start { open: true },
        ["stop" | "quit"] => Verb::Stop,
        ["restart"] => Verb::Restart { open: false },
        ["restart", "--open"] => Verb::Restart { open: true },
        ["status"] => Verb::Status { json: false },
        ["status", "--json"] => Verb::Status { json: true },
        ["open"] | ["open", "--launch"] => Verb::Open,
        ["link"] => Verb::Link,
        ["paths"] => Verb::Paths,
        ["completions", "powershell"] => Verb::Completions(Shell::PowerShell),
        ["completions", "bash"] => Verb::Completions(Shell::Bash),
        ["completions", ..] => {
            return Parsed::Usage("completions: say the shell, `powershell` or `bash`".to_owned());
        }
        ["logs", rest @ ..] => match parse_logs(rest) {
            Ok(verb) => verb,
            Err(e) => return Parsed::Usage(e),
        },
        ["close", ..] => {
            return Parsed::Usage(
                "`close` is not a command: `stop` (or `quit`) ends the hub".to_owned(),
            );
        }
        _ => return Parsed::Own,
    };
    Parsed::Common(verb)
}

/// The words after `logs`: `-f`/`--follow`, `-n N`, `-n=N`, `--lines N`, `--lines=N`.
fn parse_logs(words: &[&str]) -> Result<Verb, String> {
    let mut lines = DEFAULT_LOG_LINES;
    let mut follow = false;
    let mut rest = words.iter();
    let count = |text: &str| -> Result<usize, String> {
        match text.parse::<usize>() {
            Ok(n) if (1..=MAX_LOG_LINES).contains(&n) => Ok(n),
            _ => Err(format!(
                "logs: the number of lines is 1 to {MAX_LOG_LINES}, not {text:?}"
            )),
        }
    };
    while let Some(word) = rest.next() {
        match *word {
            "-f" | "--follow" => follow = true,
            "-n" | "--lines" => {
                lines = count(rest.next().ok_or("logs: -n needs a number")?)?;
            }
            other => {
                if let Some(n) = other
                    .strip_prefix("-n=")
                    .or_else(|| other.strip_prefix("--lines="))
                {
                    lines = count(n)?;
                } else {
                    return Err(format!("logs: unknown word {other:?} (logs [-f] [-n N])"));
                }
            }
        }
    }
    Ok(Verb::Logs { lines, follow })
}

/// The lines of the common commands for the usage of an app called `command`, in the layout of the usage texts of
/// the apps (the command at the left, what it does from column 42).
#[must_use]
pub fn usage_lines(command: &str) -> String {
    let rows: [(&str, &str); 12] = [
        (
            "",
            "(no argument, as a double click) start the hub when it is not running, open the page",
        ),
        (
            "start [--open]",
            "run the hub in the background; --open also opens the page",
        ),
        ("stop | quit", "end the hub that runs in the background"),
        ("restart [--open]", "stop, then start"),
        (
            "status [--json]",
            "does the hub run (process, port, version)? exit code 0 yes, 1 no",
        ),
        (
            "open",
            "open the page, starting the hub when it is not running",
        ),
        ("link", "print a one-time pairing link for the running hub"),
        (
            "logs [-f] [-n N]",
            "the last lines of the hub's log (-f follows it)",
        ),
        (
            "paths",
            "the program, the settings, the log and the command",
        ),
        ("version | --version", "the version"),
        (
            "completions <shell>",
            "tab completion script for powershell or bash (help completions)",
        ),
        (
            "help [<command>]",
            "this text, or what one command does (also <command> --help)",
        ),
    ];
    let mut out = String::new();
    for (words, what) in rows {
        let left = if words.is_empty() {
            format!("  {command}")
        } else {
            format!("  {command} {words}")
        };
        out.push_str(&format!("{left:<42}{what}\n"));
    }
    out
}

/// The common commands with the words that can follow each, for the completion scripts. Every [`Verb`] has its word
/// here (`every_verb_is_completed` fails when one does not).
const COMMON_WORDS: [(&str, &[&str]); 12] = [
    ("start", &["--open"]),
    ("stop", &[]),
    ("quit", &[]),
    ("restart", &["--open"]),
    ("status", &["--json"]),
    ("open", &["--launch"]),
    ("link", &[]),
    ("logs", &["-f", "--follow", "-n", "--lines"]),
    ("paths", &[]),
    ("version", &[]),
    ("help", &[]),
    ("completions", &["powershell", "bash"]),
];

/// Whether `word` may stand in a script as it is (a command name, never a quote or a space).
fn is_plain_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The script that completes the commands of an app with the Tab key, for `completions powershell|bash`.
/// `programs` are the names the shell finds the app by (`tasks-desktop`, `tasks-hub`), `own` the app's own commands
/// (`serve`, `setup`, ...). A word that is not plain (letters, digits, `-`, `_`, `.`) is left out, so nothing of an
/// argument ends up as code. The script names no path: only the command names.
#[must_use]
pub fn completions_script(shell: Shell, programs: &[&str], own: &[&str]) -> String {
    let programs: Vec<&str> = programs
        .iter()
        .copied()
        .filter(|p| is_plain_word(p))
        .collect();
    let mut top: Vec<&str> = COMMON_WORDS.iter().map(|(w, _)| *w).collect();
    for word in own.iter().copied().filter(|w| is_plain_word(w)) {
        if !top.contains(&word) {
            top.push(word);
        }
    }
    let top = top.join(" ");
    match shell {
        Shell::PowerShell => powershell_script(&programs, &top),
        Shell::Bash => bash_script(&programs, &top),
    }
}

fn powershell_script(programs: &[&str], top: &str) -> String {
    let names = programs
        .iter()
        .map(|p| format!("'{p}'"))
        .collect::<Vec<_>>()
        .join(",");
    let mut out =
        format!("Register-ArgumentCompleter -Native -CommandName {names} -ScriptBlock {{\n");
    out.push_str("    param($wordToComplete, $commandAst, $cursorPosition)\n");
    out.push_str("    $words = @($commandAst.CommandElements | Select-Object -Skip 1 | ForEach-Object { $_.ToString() })\n");
    out.push_str(
        "    if ($wordToComplete -ne '') { $words = @($words | Select-Object -SkipLast 1) }\n",
    );
    out.push_str("    $candidates = @()\n");
    out.push_str(&format!(
        "    if ($words.Count -eq 0) {{ $candidates = '{top}' -split ' ' }}\n"
    ));
    out.push_str("    else {\n        switch ($words[0]) {\n");
    for (word, next) in COMMON_WORDS {
        let list = if word == "help" {
            // `help <command>` completes one command, not a second word.
            if next.is_empty() {
                "if ($words.Count -eq 1) { $candidates = '{top}' -split ' ' }".replace("{top}", top)
            } else {
                continue;
            }
        } else if next.is_empty() {
            continue;
        } else {
            format!("$candidates = '{}' -split ' '", next.join(" "))
        };
        out.push_str(&format!("            '{word}' {{ {list} }}\n"));
    }
    out.push_str("        }\n    }\n");
    out.push_str("    $candidates | Where-Object { $_.StartsWith($wordToComplete, [System.StringComparison]::OrdinalIgnoreCase) } |\n");
    out.push_str("        ForEach-Object { [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }\n");
    out.push_str("}\n");
    out
}

fn bash_script(programs: &[&str], top: &str) -> String {
    let function = format!(
        "_{}_complete",
        programs
            .first()
            .copied()
            .unwrap_or("app")
            .replace(['-', '.'], "_")
    );
    let mut out = format!("{function}() {{\n");
    out.push_str("    local cur=\"${COMP_WORDS[COMP_CWORD]}\"\n");
    out.push_str("    if [ \"$COMP_CWORD\" -eq 1 ]; then\n");
    out.push_str(&format!(
        "        COMPREPLY=($(compgen -W \"{top}\" -- \"$cur\"))\n        return 0\n    fi\n"
    ));
    out.push_str("    case \"${COMP_WORDS[1]}\" in\n");
    for (word, next) in COMMON_WORDS {
        let list = if word == "help" {
            top.to_owned()
        } else if next.is_empty() {
            continue;
        } else {
            next.join(" ")
        };
        let guard = if word == "help" {
            "[ \"$COMP_CWORD\" -eq 2 ] && "
        } else {
            ""
        };
        out.push_str(&format!(
            "        {word}) {guard}COMPREPLY=($(compgen -W \"{list}\" -- \"$cur\")) ;;\n"
        ));
    }
    out.push_str("    esac\n    return 0\n}\n");
    out.push_str(&format!("complete -F {function} {}\n", programs.join(" ")));
    out
}

/// What one common command does, for `help <command>`; `None` for a command that is not a common one (the app's
/// own, which it explains itself).
#[must_use]
pub fn help(command: &str, program: &str) -> Option<String> {
    let text = match command {
        "start" => format!(
            "{program} start [--open]\n  Run the hub in the background (no window; its log is `{program} logs`).\n  --open also opens the page in the browser. A hub that runs already is not started twice."
        ),
        "stop" | "quit" => format!(
            "{program} stop   (also: {program} quit)\n  End the hub that runs in the background. Nothing runs: done all the same."
        ),
        "restart" => format!(
            "{program} restart [--open]\n  Stop the hub and start it again (after an update, say); --open also opens the page."
        ),
        "status" => format!(
            "{program} status [--json]\n  Whether the hub runs: its process, port and version, the start with the login,\n  the program, the settings and the log. Exit code 0 when it runs, 1 when not.\n  --json prints the same as one JSON object, for a script."
        ),
        "open" => format!(
            "{program} open\n  Open the page in the browser; a hub that does not run is started first.\n  (`open --launch` of earlier versions does the same.)"
        ),
        "link" => format!(
            "{program} link\n  Print a one-time pairing link for the running hub, without opening it:\n  for a phone, a tunnel or a script. It works once."
        ),
        "logs" => format!(
            "{program} logs [-f] [-n N]\n  The last N lines (default {DEFAULT_LOG_LINES}) of the log of the hub in the background.\n  -f keeps printing what is added, until Ctrl-C."
        ),
        "paths" => format!(
            "{program} paths\n  The running program, the installed one, the settings folder, the log and the\n  command in the terminal."
        ),
        "version" => format!("{program} version   (also --version, -V)\n  The version."),
        "completions" => format!(
            "{program} completions powershell|bash\n  Print the script that completes the commands of {program} with the Tab key.\n  PowerShell: add `{program} completions powershell | Out-String | Invoke-Expression` to your profile ($PROFILE).\n  bash: add `source <({program} completions bash)` to ~/.bashrc."
        ),
        "help" => format!(
            "{program} help [<command>]\n  All commands, or what one does (also `{program} <command> --help`)."
        ),
        _ => return None,
    };
    Some(text)
}

/// What `status` found.
#[derive(Debug, Clone)]
pub struct Status {
    /// The name of the app (`tasks-desktop`).
    pub app: String,
    /// The hub that runs, as `hub.json` names it and its `hello` confirms.
    pub running: Option<HubRecord>,
    /// Whether it starts with the login (`None`: cannot be told here).
    pub autostart: Option<bool>,
    /// The program that answers (`current_exe`) and its version.
    pub program: Option<PathBuf>,
    pub program_version: String,
    /// The settings folder and the log in it.
    pub config: Option<PathBuf>,
    pub log: Option<PathBuf>,
    /// The starter of the command in the terminal, when there is one.
    pub command: Option<PathBuf>,
}

impl Status {
    /// 0 when the hub runs, 1 when not.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(self.running.is_none())
    }

    /// The lines for the person.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = match &self.running {
            Some(hub) => format!(
                "{}: running (process {}, port {}, version {})\n",
                self.app, hub.pid, hub.port, hub.version
            ),
            None => format!("{}: not running\n", self.app),
        };
        let shown = |p: &Option<PathBuf>| {
            p.as_ref()
                .map_or_else(|| "(unknown)".to_owned(), |p| p.display().to_string())
        };
        let autostart = match self.autostart {
            Some(true) => "on",
            Some(false) => "off",
            None => "unknown",
        };
        out.push_str(&format!("  start with the login: {autostart}\n"));
        out.push_str(&format!(
            "  program:  {} ({})\n",
            shown(&self.program),
            self.program_version
        ));
        out.push_str(&format!("  settings: {}\n", shown(&self.config)));
        out.push_str(&format!("  log:      {}\n", shown(&self.log)));
        if let Some(command) = &self.command {
            out.push_str(&format!("  command:  {}\n", command.display()));
        }
        out
    }

    /// The same as one JSON object.
    #[must_use]
    pub fn json(&self) -> String {
        let path = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
        json!({
            "app": self.app,
            "running": self.running.is_some(),
            "pid": self.running.as_ref().map(|h| h.pid),
            "port": self.running.as_ref().map(|h| h.port),
            "version": self.running.as_ref().map(|h| h.version.clone()),
            "autostart": self.autostart,
            "program": path(&self.program),
            "program_version": self.program_version,
            "config": path(&self.config),
            "log": path(&self.log),
            "command": path(&self.command),
        })
        .to_string()
    }
}

/// The log of the hub in the background in the settings folder.
fn log_path(config: &Path) -> PathBuf {
    config.join("hub.log")
}

/// The starter of `command` that is there (`cli_entry`), the one the shell of this system finds.
fn command_file(command: &str, env: &Env) -> Option<PathBuf> {
    let dir = cli_entry::starter_dir(env)?;
    cli_entry::starter_files(&dir, command)
        .into_iter()
        .find(|f| f.is_file())
}

/// Whether the hub of the app runs, and where its parts are. `version` is the version of this program.
pub async fn status(app: &Background<'_>, version: &str) -> Status {
    let env = Env::from_process();
    let identity = app.login.identity;
    let config = instance::config_folder(&env, identity).ok();
    let running = match &config {
        Some(dir) => instance::running_hub(dir, identity).await,
        None => None,
    };
    Status {
        app: identity.app.to_owned(),
        running,
        autostart: background::autostart_of(identity),
        program: std::env::current_exe().ok(),
        program_version: version.to_owned(),
        log: config.as_deref().map(log_path),
        config,
        command: command_file(identity.app, &env),
    }
}

/// The lines of `paths`.
#[must_use]
pub fn paths(app: &Background<'_>) -> String {
    let env = Env::from_process();
    let identity = app.login.identity;
    let shown = |p: Result<PathBuf, String>| match p {
        Ok(p) => p.display().to_string(),
        Err(e) => format!("(none: {e})"),
    };
    let config = instance::config_folder(&env, identity);
    let mut out = String::new();
    out.push_str(&format!(
        "program:   {}\n",
        shown(std::env::current_exe().map_err(|e| e.to_string()))
    ));
    out.push_str(&format!(
        "installed: {}\n",
        shown(background::installed_program(app, &env))
    ));
    out.push_str(&format!("settings:  {}\n", shown(config.clone())));
    out.push_str(&format!(
        "log:       {}\n",
        shown(config.map(|c| log_path(&c)))
    ));
    out.push_str(&format!(
        "command:   {}\n",
        command_file(identity.app, &env).map_or_else(
            || format!(
                "(none: `{} install` makes `{}`)",
                identity.binary, identity.app
            ),
            |p| p.display().to_string()
        )
    ));
    out
}

/// The last `lines` lines of `text` (all of it when it has fewer).
#[must_use]
pub fn last_lines(text: &str, lines: usize) -> &str {
    if lines == 0 {
        return "";
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut seen = 0;
    for (at, byte) in body.bytes().enumerate().rev() {
        if byte == b'\n' {
            seen += 1;
            if seen == lines {
                return &text[at + 1..];
            }
        }
    }
    text
}

/// How often `logs -f` looks for new lines.
const FOLLOW_PAUSE: Duration = Duration::from_millis(500);

/// Print the last `lines` lines of the log of the hub in the background; with `follow`, keep printing what is added
/// until the process is ended (Ctrl-C). A log that was started anew (it is shorter than before) is followed from its
/// start.
///
/// # Errors
///
/// There is no settings folder, or the log cannot be read (not there: the hub never ran in the background).
pub async fn logs(app: &Background<'_>, lines: usize, follow: bool) -> Result<(), String> {
    let env = Env::from_process();
    let config = instance::config_folder(&env, app.login.identity)?;
    let path = log_path(&config);
    let raw = std::fs::read(&path).map_err(|e| {
        format!(
            "no log at {} ({e}): the hub has not run in the background yet",
            path.display()
        )
    })?;
    print!("{}", last_lines(&String::from_utf8_lossy(&raw), lines));
    if !follow {
        return Ok(());
    }
    let mut tail = Tail::after(&raw);
    loop {
        tokio::time::sleep(FOLLOW_PAUSE).await;
        if let Some(added) = tail.added(&path) {
            print!("{}", String::from_utf8_lossy(&added));
        }
    }
}

/// Where `logs -f` is in the log: a count of bytes of the file, never of the text it was decoded to (a byte that is
/// not UTF-8 becomes three).
struct Tail {
    at: u64,
}

impl Tail {
    /// After all of `read`, the bytes read so far.
    fn after(read: &[u8]) -> Self {
        Self {
            at: read.len() as u64,
        }
    }

    /// What was added to the file at `path` since the last look; from its start when it is shorter than before (a
    /// log started anew). `None`: nothing new, or it cannot be read now.
    fn added(&mut self, path: &Path) -> Option<Vec<u8>> {
        let mut file = std::fs::File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        if len < self.at {
            self.at = 0;
        }
        if len == self.at {
            return None;
        }
        file.seek(SeekFrom::Start(self.at)).ok()?;
        let mut added = Vec::new();
        file.read_to_end(&mut added).ok()?;
        self.at += added.len() as u64;
        Some(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn common(words: &[&str]) -> Verb {
        match parse(&args(words)) {
            Parsed::Common(verb) => verb,
            other => panic!("{words:?}: {other:?}"),
        }
    }

    #[test]
    fn the_common_words_and_their_aliases_are_read() {
        assert_eq!(common(&[]), Verb::Click);
        assert_eq!(common(&["start"]), Verb::Start { open: false });
        assert_eq!(common(&["start", "--open"]), Verb::Start { open: true });
        assert_eq!(common(&["stop"]), Verb::Stop);
        assert_eq!(common(&["quit"]), Verb::Stop);
        assert_eq!(common(&["restart", "--open"]), Verb::Restart { open: true });
        assert_eq!(common(&["status"]), Verb::Status { json: false });
        assert_eq!(common(&["status", "--json"]), Verb::Status { json: true });
        assert_eq!(common(&["open"]), Verb::Open);
        assert_eq!(common(&["open", "--launch"]), Verb::Open);
        assert_eq!(common(&["link"]), Verb::Link);
        assert_eq!(common(&["paths"]), Verb::Paths);
        assert_eq!(
            common(&["completions", "bash"]),
            Verb::Completions(Shell::Bash)
        );
        assert_eq!(
            common(&["completions", "powershell"]),
            Verb::Completions(Shell::PowerShell)
        );
        for v in ["version", "--version", "-V"] {
            assert_eq!(common(&[v]), Verb::Version);
        }
        for h in ["help", "--help", "-h"] {
            assert_eq!(common(&[h]), Verb::Help(None));
        }
        assert_eq!(
            common(&["help", "status"]),
            Verb::Help(Some("status".into()))
        );
        assert_eq!(
            common(&["status", "--help"]),
            Verb::Help(Some("status".into()))
        );
        // The help of a command of the app is asked the same way; the app answers it.
        assert_eq!(common(&["setup", "-h"]), Verb::Help(Some("setup".into())));
    }

    #[test]
    fn the_words_of_an_app_are_left_to_it() {
        for own in [
            &["serve"][..],
            &["serve", "--port", "9000"],
            &["tray"],
            &["install", "--shortcut"],
            &["uninstall"],
            &["autostart", "on"],
            &["setup"],
            // A common verb with words of the app: tasks-hub has `open --phone`.
            &["open", "--phone", "--no-window"],
            &["start", "--port", "1"],
            &["devices", "revoke", "x"],
        ] {
            assert_eq!(parse(&args(own)), Parsed::Own, "{own:?}");
        }
    }

    #[test]
    fn close_points_to_stop_and_bad_logs_words_are_said() {
        match parse(&args(&["close"])) {
            Parsed::Usage(text) => assert!(text.contains("`stop`"), "{text}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            common(&["logs"]),
            Verb::Logs {
                lines: DEFAULT_LOG_LINES,
                follow: false
            }
        );
        assert_eq!(
            common(&["logs", "-f", "-n", "5"]),
            Verb::Logs {
                lines: 5,
                follow: true
            }
        );
        assert_eq!(
            common(&["logs", "--lines=7", "--follow"]),
            Verb::Logs {
                lines: 7,
                follow: true
            }
        );
        assert_eq!(
            common(&["logs", "-n=3"]),
            Verb::Logs {
                lines: 3,
                follow: false
            }
        );
        for bad in [
            &["logs", "-n"][..],
            &["logs", "-n", "0"],
            &["logs", "-n", "x"],
            &["logs", "-n", "99999"],
            &["logs", "--tail"],
        ] {
            assert!(matches!(parse(&args(bad)), Parsed::Usage(_)), "{bad:?}");
        }
    }

    #[test]
    fn every_common_command_has_its_help_and_the_usage_lists_them() {
        let usage = usage_lines("tasks-desktop");
        for verb in [
            "start",
            "stop",
            "restart",
            "status",
            "open",
            "link",
            "logs",
            "paths",
            "version",
            "completions",
            "help",
        ] {
            let text = help(verb, "tasks-desktop").unwrap_or_else(|| panic!("{verb}"));
            assert!(text.starts_with("tasks-desktop "), "{text}");
            assert!(
                usage.contains(&format!("tasks-desktop {verb}")),
                "{verb}: {usage}"
            );
        }
        assert!(usage.contains("tasks-desktop stop | quit"), "{usage}");
        assert!(help("quit", "tasks-desktop").is_some());
        assert_eq!(help("setup", "tasks-desktop"), None);
        assert!(usage.starts_with("  tasks-desktop "), "{usage}");
        // What the command does starts in the same column on every line.
        for line in usage.lines() {
            assert!(line.len() > 42 && line.as_bytes()[41] == b' ', "{line:?}");
        }
    }

    /// The word of a verb in the completions; the match has no wildcard, so a new [`Verb`] does not compile before it
    /// has its word.
    fn word_of(verb: &Verb) -> &'static str {
        match verb {
            Verb::Click => "",
            Verb::Start { .. } => "start",
            Verb::Stop => "stop",
            Verb::Restart { .. } => "restart",
            Verb::Status { .. } => "status",
            Verb::Open => "open",
            Verb::Link => "link",
            Verb::Logs { .. } => "logs",
            Verb::Paths => "paths",
            Verb::Version => "version",
            Verb::Completions(_) => "completions",
            Verb::Help(_) => "help",
        }
    }

    #[test]
    fn every_verb_is_completed() {
        let verbs = [
            Verb::Click,
            Verb::Start { open: false },
            Verb::Stop,
            Verb::Restart { open: false },
            Verb::Status { json: false },
            Verb::Open,
            Verb::Link,
            Verb::Logs {
                lines: 1,
                follow: false,
            },
            Verb::Paths,
            Verb::Version,
            Verb::Completions(Shell::Bash),
            Verb::Help(None),
        ];
        let scripts = [Shell::PowerShell, Shell::Bash].map(|shell| {
            completions_script(shell, &["tasks-desktop", "tasks-hub"], &["serve", "setup"])
        });
        for script in &scripts {
            for verb in &verbs {
                let word = word_of(verb);
                assert!(word.is_empty() || script.contains(word), "{word}: {script}");
            }
            for own in ["serve", "setup", "quit"] {
                assert!(script.contains(own), "{own}");
            }
        }
        // Every word of the list is one the parser knows, `quit` and `completions` included.
        for (word, _) in COMMON_WORDS {
            assert_ne!(parse(&args(&[word])), Parsed::Own, "{word}");
        }
        assert!(scripts[0].contains("-CommandName 'tasks-desktop','tasks-hub'"));
        assert!(scripts[1].contains("complete -F _tasks_desktop_complete tasks-desktop tasks-hub"));
    }

    #[test]
    fn completions_need_a_known_shell_and_leave_out_odd_words() {
        for bad in [
            &["completions"][..],
            &["completions", "zsh"],
            &["completions", "bash", "x"],
        ] {
            assert!(matches!(parse(&args(bad)), Parsed::Usage(_)), "{bad:?}");
        }
        let script = completions_script(Shell::Bash, &["a b", "app"], &["ok", "x'; rm -rf ~; '"]);
        assert!(script.contains("ok") && !script.contains("rm -rf") && !script.contains("a b"));
    }

    #[test]
    fn status_says_whether_it_runs_in_text_json_and_exit_code() {
        let mut status = Status {
            app: "tasks-desktop".into(),
            running: None,
            autostart: Some(false),
            program: Some(PathBuf::from("/apps/tasks-hub")),
            program_version: "1.2.3".into(),
            config: Some(PathBuf::from("/cfg")),
            log: Some(PathBuf::from("/cfg/hub.log")),
            command: None,
        };
        assert_eq!(status.exit_code(), 1);
        let off = status.text();
        assert!(off.starts_with("tasks-desktop: not running\n"), "{off}");
        assert!(off.contains("start with the login: off"), "{off}");
        assert!(off.contains("(1.2.3)"), "{off}");
        let doc: serde_json::Value = serde_json::from_str(&status.json()).unwrap();
        assert_eq!(doc["running"], false);
        assert_eq!(doc["pid"], serde_json::Value::Null);

        status.running = Some(HubRecord {
            app: "tasks-desktop".into(),
            port: 8123,
            pid: 42,
            version: "1.2.0".into(),
            instance: "i".into(),
        });
        status.autostart = None;
        assert_eq!(status.exit_code(), 0);
        let on = status.text();
        assert!(
            on.starts_with("tasks-desktop: running (process 42, port 8123, version 1.2.0)\n"),
            "{on}"
        );
        assert!(on.contains("start with the login: unknown"), "{on}");
        let doc: serde_json::Value = serde_json::from_str(&status.json()).unwrap();
        assert_eq!(doc["running"], true);
        assert_eq!(doc["port"], 8123);
        assert_eq!(doc["program_version"], "1.2.3");
        assert_eq!(doc["log"], "/cfg/hub.log");
    }

    /// `logs -f` goes on after the bytes it read, also when the log holds a byte that is not UTF-8 (decoded, it would
    /// be longer, and the log would look started anew and be printed again from the top).
    #[test]
    fn following_a_log_prints_only_what_was_added() {
        let root = appkit_scratch::Scratch::new("cli-logs-follow");
        std::fs::create_dir_all(&*root).unwrap();
        let log = root.join("hub.log");
        let first = b"one \xff\xfe two\n".to_vec();
        std::fs::write(&log, &first).unwrap();
        let mut tail = Tail::after(&std::fs::read(&log).unwrap());
        assert_eq!(tail.added(&log), None);
        let mut more = first.clone();
        more.extend_from_slice(b"three\n");
        std::fs::write(&log, &more).unwrap();
        assert_eq!(tail.added(&log), Some(b"three\n".to_vec()));
        assert_eq!(tail.added(&log), None);
        // Started anew (shorter): from its start.
        std::fs::write(&log, b"new\n").unwrap();
        assert_eq!(tail.added(&log), Some(b"new\n".to_vec()));
    }

    #[test]
    fn the_last_lines_of_a_log_are_its_end() {
        let log = "a\nb\nc\n";
        assert_eq!(last_lines(log, 2), "b\nc\n");
        assert_eq!(last_lines(log, 3), log);
        assert_eq!(last_lines(log, 9), log);
        assert_eq!(last_lines(log, 0), "");
        assert_eq!(last_lines("a\nb", 1), "b");
        assert_eq!(last_lines("", 3), "");
    }
}
