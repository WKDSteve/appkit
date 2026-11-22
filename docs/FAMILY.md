# The family: which apps are installed, which run

An app of the family asks whether another one is there before it shows a button that hands something
over to it ("hand this task to an agent" shows only while ai-desktop is installed). The answer comes
from two kinds of small files, **without a network and without a secret**: a few file reads, cheap
enough for every call. `appkit_core::family` reads them; a hub passes the answer to its page at
`GET /api/family`.

## Installed: the manifest in the family folder

An installer writes `<family>/installed/<app>.json` and the uninstaller removes it. `<family>` is

| System | Folder |
|---|---|
| any | `APPKIT_FAMILY_DIR`, when it is set (tests, a portable setup) |
| Windows | `%LOCALAPPDATA%\appkit-family` |
| macOS | `~/Library/Application Support/appkit-family` |
| Linux | `$XDG_DATA_HOME/appkit-family`, else `~/.local/share/appkit-family` |

It is the local folder, not the roaming one: what is installed on one machine is not installed on the
next. The manifest:

```json
{
  "schema": "appkit-installed/1",
  "app": "ai-desktop",
  "version": "0.1.0",
  "program": "C:\\Users\\me\\AppData\\Local\\Programs\\ai-desktop\\app\\ai-desktop.exe",
  "env_prefix": "AI_DESKTOP"
}
```

- `schema` is `appkit-installed/1`; another value is not read. A later version only adds keys, which a
  reader ignores (unlike `hub.json`, see below).
- `app` is the name of the file without `.json` and the `app` of the app's `hub.json`: letters, digits,
  `-`, `_`, `.`, not starting with a dot.
- `program` is the installed program. A manifest whose program is gone (deleted by hand, an uninstaller
  that did not run) does not count as installed.
- `env_prefix` names the variable that may move the app's config folder (`<PREFIX>_CONFIG_DIR`), so the
  reader finds its `hub.json` there.
- `version` may be missing (`appkit-hub install` does not know the app's version; the running app says it).

Who writes it: `appkit-hub`'s `install` command (every hub app: tasks-, rules-, docmap-, testing-,
hub-desktop) and its `uninstall`; the Inno setup of ai-desktop. The tests of a hub program
(`<PREFIX>_NO_LOGIN_SYNC`) leave the folder of the machine alone unless they set `APPKIT_FAMILY_DIR`.

## The command in the terminal: the starters in `<family>/bin`

Every install makes the app a command named after the app, in any new terminal: `tasks-desktop status`,
`hub-desktop`, not the whole path of `tasks-hub.exe`. `appkit_hub::cli_entry` does it for the hub apps
(`install` and `uninstall`); an installer of its own (NSIS, Inno) writes the same files.

| System | Folder | Files per app | On `PATH` |
|---|---|---|---|
| Windows | `<family>\bin` (`%LOCALAPPDATA%\appkit-family\bin`) | `<app>.cmd` (PowerShell, cmd) and `<app>` (a shell script for Git Bash) | the `PATH` of the user (`HKCU\Environment`), added once, as `%LOCALAPPDATA%\appkit-family\bin` in an expandable value |
| Linux, macOS | `~/.local/bin` (`<family>/bin` when `APPKIT_FAMILY_DIR` is set) | `<app>`, executable | not changed; `install` says when the folder is missing on it |

- A starter starts the program with all arguments and hands its exit code back; a program that is gone
  ends it with exit code 3 and a line on stderr. It carries the words `appkit starter`: a file of that
  name without them is never overwritten or removed.
- The `.cmd` names a program below `LOCALAPPDATA` as `%LOCALAPPDATA%\...`; another path must be plain
  ASCII, since cmd reads the file in the code page of the console.
- A starter, not a copy of the program: the hubs are window programs, and PowerShell does not wait for a
  window program (its output would come after the prompt, `$LASTEXITCODE` would be wrong); cmd waits for
  the program a `.cmd` starts.
- `uninstall` removes the starters of its app; the last one takes the folder and its `PATH` entry with it.
- The `PATH` of the user keeps the type of its value (`REG_EXPAND_SZ` stays expandable, so its `%VAR%`
  keep working); after a change the session is told (`WM_SETTINGCHANGE`), so new terminals see it. A
  terminal that is open already does not: `install` names the line that adds the folder there.
- The tests of a hub program (`<PREFIX>_NO_LOGIN_SYNC`) make no starter unless they set
  `APPKIT_FAMILY_DIR`, and then never touch the `PATH` of the user.

## Running: `hub.json` in the config folder

The config folder of an app is `<PREFIX>_CONFIG_DIR`, else `%APPDATA%\<app>` (Windows),
`~/Library/Application Support/<app>` (macOS), `$XDG_CONFIG_HOME/<app>` or `~/.config/<app>`. A running
app keeps `hub.json` there (`appkit_core::registry`):

```json
{
  "app": "ai-desktop",
  "port": 8080,
  "pid": 4242,
  "version": "0.1.0",
  "instance": "0123456789abcdef0123456789abcdef"
}
```

The format is exact: the reader refuses unknown keys (`deny_unknown_fields`), so an app that is not
built on appkit (ai-desktop, C++) writes exactly these five keys, in this order, as
`serde_json::to_string_pretty` does (two spaces, a newline at the end), through a temporary file and a
rename, and removes the file when it ends. `instance` changes with every start: it is the key of what a
client keeps about the app.

A `hub.json` counts as running only while the process `pid` is alive: a file left by a process that was
killed is not believed. On Windows the process must also have started before the file was written, so
that a process id the system gave to another program later is not taken for the app. No port is asked:
before a real hand-over the caller talks to the app anyway (and checks `instance` there).

## The API

```rust
use appkit_core::family::{family_apps, family_app, Member};

let vars = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(Into::into);
// Every installed app of the family:
let apps = family_apps(&vars, &[]);
// One app, also when it runs from a checkout without a manifest:
let ai = family_app(&vars, Member { app: "ai-desktop", env_prefix: "AI_DESKTOP" });
if ai.installed { /* show the button */ }
if let (true, Some(port)) = (ai.running, ai.port) { /* hand over on 127.0.0.1:port */ }
```

`FamilyApp` has `app`, `installed`, `running`, `port`, `version` (of the running app, else of the
manifest) and `instance`, and for the program (never for a page) `program`.

## The route of a hub: `GET /api/family`

A route of the core, behind the session like `/api/vaults`; a paired device does not reach it. It takes
no query and answers

```json
{"schema": 1, "kind": "hub.family", "apps": [
  {"app": "ai-desktop", "installed": true, "running": false, "port": null, "version": "0.1.0", "instance": null}
]}
```

with every app that has a manifest, sorted by name, and no path. Each request reads the files anew. The
page asks once at its start and keeps the answer, and asks again when its window gets the focus and
before a hand-over: port and instance change with every start of the other app. A page that offers AI
functions also honours its own setting to hide them; the family only says what is possible.
