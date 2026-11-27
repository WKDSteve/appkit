# release-kit

How a native app is published, the same way for every app: a release `vX.Y.Z` that a person opens
before it goes out, a rolling build `latest` of the newest green commit of `main`, and on both an
install script that installs exactly that build.

| Part | What it does |
|---|---|
| `prepare/` | A GitHub Action: which commit is built: the newest one of `main` with a green CI run (or one named by hand). Makes the pre-release `latest` once. |
| `publish/` | A GitHub Action: puts the built files, the stamped install scripts and `SHA256SUMS` on `latest` (and moves it) or on a release draft, and attests them (see "Attestation"). Its steps are `publish.sh`. |
| `scripts/install.ps1` | Windows: downloads the build, checks it against `SHA256SUMS`, installs it for the current user. |
| `scripts/install.sh` | Linux and macOS: the same. |
| `stamp.py` | Writes the repository, the app and the channel into a copy of an install script. |
| `version_json.py` | Writes `version.json`, which `publish.sh` puts on every build (see below). |

The kit builds nothing: the build stays a job of each app (pnpm and cargo, Tauri with its sidecar,
MSYS2 and Inno Setup are too different). It takes over what is the same everywhere: deciding,
uploading, checking, naming.

## The channels

- **`latest`** is a pre-release that is never marked "Latest". After every green CI run on `main`,
  the build of the newest green commit of `main` (not of the tip: a red tip, or one still in CI, does not keep it
  from being built) replaces its files, which keep **fixed names without a version**
  (`releases/download/latest/<file>` always works). A channel only moves forward: a target that is not a descendant of the commit
  in `version.json` of the build that is up builds nothing, so a late run never moves it backwards. While a
  descendant of the newest green commit is still in CI (its run queued or in progress), nothing is built either: that
  CI ends in a rolling run of its own, which builds the descendant when it is green and the older commit when it is
  red, so nothing stays unbuilt and no build is made only to be replaced minutes later (the job `if` of `prepare`
  therefore lets red CI runs through too). `wait-for-descendants: false` builds every green state instead; a commit
  named by hand and the other channels never wait. `SHA256SUMS` goes first, files of an earlier build that
  this one no longer has go, and only at the end does the tag move to the commit and the page name it:
  a run that stops in between leaves the page naming the previous commit.
- **`vX.Y.Z`** is a release from a tag, left as a **draft**: a person installs it, opens it,
  and publishes it. A published release is never touched again. A draft that is there
  already (the tag was set again on a newer commit) gets the files and the notes of this run.

### More rolling channels: `nightly` and `stable`

Only where they are needed; the default of an app is `latest` alone. Both are pre-releases of their
own name with the same fixed file names as `latest`, move only forward (each against the `version.json` of its own
release), and carry their name in `version.json` (`"channel": "nightly"`) and in the stamped install scripts.

- **`nightly`**: the tip of `main`, built at once, green or not. For an app whose newest state somebody wants to
  see before the CI is through. Triggered by a `push` to `main`.
- **`stable`**: the commit `latest` has had at the top for at least 14 days (`STABLE_DAYS` of `pick_target.py`, read
  from the date in `version.json` of `latest`), if no CI run of `main` went red since and no issue labelled `bug`
  was filed since. A weekly `schedule` runs the decision; a manual start with `sha` promotes a green commit by hand,
  whatever its age. The bug check sees the GitHub issues of the repository only; bug tasks that live elsewhere do
  not reach it.

An app names the channels it has switched on in one line, the `channels` input of `prepare`
(`channels: latest nightly stable`, default `latest`); a run for another channel builds nothing and says so.
The decision of every channel is `pick_target.py` (tests: `test_pick_target.py`).

The tag `latest` is moved with the token of the workflow. Should GitHub refuse that (a token may not
point a ref past a change of `.github/workflows` without the `workflows` right), the run only warns:
the downloads go by the name of the release, not by the commit of its tag, and the page still names
the commit of its files. A repository with **immutable releases** turned on cannot have a rolling
build: its files could never be replaced.

An upload (and reading the asset list, and editing the page) that GitHub answers with a passing error -- an HTTP
5xx, a timeout, a dropped connection -- is made again, up to four times with a growing wait (`GH_TRIES`, `GH_WAIT` in
seconds; every such call is safe to repeat). Any other failure ends the run at once. If the checksums still cannot be
put up, the rolling build is left without `SHA256SUMS` until the next green build, and the install scripts say that
it is either being published or its last publish failed, never only that a build is running.

## `version.json`

Both channels carry a small `version.json` next to the files (listed in `SHA256SUMS` like the rest, address
`releases/download/<channel>/version.json`). A hub that lists apps compares the version of the installed app
with it, instead of reading the release API of each app:

```json
{"schema": "release-kit-version/1", "app": "demo", "channel": "latest", "version": "1.2.3",
 "commit": "<40 hex>", "date": "2026-11-01T12:00:00Z"}
```

- `version` is what the program reports about itself. A release has the version of its tag and `publish`
  refuses a different `version` input; the rolling build has `null` unless the app passes `version:`
  (read it from `Cargo.toml` or `package.json` in the workflow).
- `commit` is the commit of the files (`null` on a release), `date` is when the build was published (UTC).
- A later minor version only adds keys; a reader ignores what it does not know. A file of the name
  `version.json` among the built or extra files is refused: the name is the kit's.

## The install scripts

Each release carries `install.ps1` (and `install.sh` where there is a Linux or macOS build). The copy
on a release is **stamped**: it knows its repository, its app and its channel. The script of
`v1.2.0` installs v1.2.0, the script of `latest` installs the rolling build, both without options.
The first line is typed into a PowerShell session (Windows PowerShell 5.1 or PowerShell 7), the second
into a POSIX shell:

```
irm https://github.com/OWNER/NAME/releases/download/latest/install.ps1 | iex
curl -fsSL https://github.com/OWNER/NAME/releases/download/latest/install.sh | sh
```

**A private repository** answers these addresses with 404. Set `GH_TOKEN` (or `GITHUB_TOKEN`; a token
with read access to the repository is enough) and the script downloads through the API instead, from
a copy you saved first (the one-liner cannot send the token for its own download):

```
gh release download latest -R OWNER/NAME -p install.ps1   # or install.sh
$env:GH_TOKEN = '<token>'; .\install.ps1
```

The token goes only to `api.github.com`, never on to the storage host the API redirects to.

Why a script and not only the setup: a file the browser downloads is marked as coming from the
internet (Mark-of-the-Web), and SmartScreen stops an unsigned setup with that mark; on a managed
workstation it may not be run at all. A file the script downloads carries no mark. What the script
checks is that the file is the one `SHA256SUMS` of the same release lists, and it runs nothing when
it does not match. That proves the file is whole and belongs to that release, not who made the
release: it is no signature, and it replaces none. `| iex` is used because a script downloaded by the
browser is marked as well; text handed to `iex` is no script file, so the execution policy does not
apply to it (tried under `Restricted`). It does not help against AppLocker or WDAC: what those forbid
stays forbidden.

**Do not wrap the line in a new PowerShell.** Until 2026-11-14 this page gave it as
`powershell -ExecutionPolicy Bypass -c "<the line>"`. Microsoft Defender reports that command line as
`Trojan:Win32/Commando.B!ml`: a verdict of its cloud model on the command line alone (a new
`powershell.exe` with the bypass switch and a download piped into `Invoke-Expression`), not on the
script and not on the setup, which both scan clean. Nothing is installed then, and what the person sees
is a misleading parser error ("the multi-line comment is missing its terminator `#>`"): after the
report the script reaches `Invoke-Expression` line by line, and its first line is `<#`. Typed into a
session there is no such command line. From `cmd.exe` or the Run dialog, the same line without the
bypass switch (`powershell -c "<the line>"`) was not reported on that day; a model's verdict can change,
so the session is the way this page names. `test_release_kit.py` fails when the wrapped form comes back
into this page or the script, and `publish` refuses to send a page whose notes have it, or whose app has it
in its README (see "Using it in an app").

The scripts install **for the current user, without administrator rights**, and refuse an elevated
session (`-AllowElevated` / `--allow-root` for a build machine): elevated, the app would be installed
into the profile of the administrator account. The one-liner takes no parameters; a CI job that runs it as
it is on an elevated machine (a GitHub-hosted Windows runner is one) sets `INSTALL_ALLOW_ELEVATED=1` in the
environment instead, which `install.ps1` reads as `-AllowElevated` (only the value `1` counts).

| Kind | Script | Install step |
|---|---|---|
| `hub` | both | The program (bare, or the only thing in a `.zip`/`.tar.gz`) runs `<program> install ARGS`, which copies itself to its fixed place. |
| `nsis` | `install.ps1` | The NSIS setup runs with `/S`. It installs for the current user only when the setup is built so (Tauri's default `installMode: "currentUser"`); a per-machine setup asks for administrator rights. |
| `inno` | `install.ps1` | The Inno setup runs with `/VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER /NORESTART`. It installs for the current user when the setup has `PrivilegesRequired=lowest` (or allows the override on the command line, `PrivilegesRequiredOverridesAllowed=commandline`). |
| `appimage` | `install.sh` | The AppImage is copied to `${XDG_BIN_HOME:-~/.local/bin}/<program>`. |

A setup that fails while the app still runs (its files locked; a silent Inno setup answers its own
Abort/Retry/Ignore box with Abort, exit code 5) is named as such: `install.ps1` looks for a running process of the
program (`-Program`, else `<app>.exe`) and says its pid and path, and to stop it, and whatever starts it again, before
running the line once more. It does not refuse up front: a build of the same name running from a checkout locks
nothing of the installed copy.

The install step also tells the other apps of the family that the app is installed: it writes
`<family>/installed/<app>.json` (`%LOCALAPPDATA%\appkit-family` on Windows) and its uninstaller removes it
([docs/FAMILY.md](../docs/FAMILY.md)). `<program> install` of a hub does it by itself; an NSIS or Inno setup does
it in its own script (ai-desktop's `installer/ai-desktop.iss` is the example). The scripts of the kit do not write
it: an uninstall from Settings > Apps never runs them, and would leave the entry behind.

Options of both scripts: another release (`-Version vX.Y.Z` / `--version`), the rolling build
(`-Rolling` / `--rolling`), a file you already have (`-File` / `--file`, with `-Sums` / `-Sha256`),
more arguments for the install step (`-InstallArgs '--autostart'` / `-- --autostart`), a stop without a
verified attestation (`-RequireAttestation` / `--require-attestation`, see "Attestation"). An unstamped
copy (the one in this folder) takes the stamped values as options (`-Repo`, `-Kind`, ...).

The `irm ... | iex` line takes no arguments. To pass some without saving the file, make a script block
of the text, which takes them (tried in PowerShell 7 and Windows PowerShell 5.1):

```
& ([scriptblock]::Create((irm https://github.com/OWNER/NAME/releases/download/latest/install.ps1))) -InstallArgs '--autostart'
```

A saved `install.ps1` runs as `powershell -ExecutionPolicy Bypass -File .\install.ps1 ...`. Through
`powershell -File` a command line cannot carry an array: `-InstallArgs 'a','b'` arrives as one string,
so pass one value there, or call the script from a PowerShell session (`& .\install.ps1 -InstallArgs 'a','b'`),
where arrays are arrays. A silent setup does not run the optional steps of its wizard (an Inno `unchecked`
task such as "add to PATH") unless they are named in `-InstallArgs` (`/TASKS=addtopath`).

## Using it in an app

Pin the actions by commit, with the tag in a comment. The rolling build, in
`.github/workflows/rolling.yml` of the app:

```yaml
name: rolling
on:
  workflow_run:
    workflows: [ci]
    types: [completed]
    branches: [main]
  workflow_dispatch:
    inputs:
      sha:
        description: A green commit of main to build again (full id); empty - the newest green commit
        required: false
# One group, nothing cancelled: a run decides at its own start (prepare), so an older run neither replaces nor
# cancels a newer build, and a waiting run is replaced by a newer one of the same group. The group does not stop a
# run that is building already; prepare does not start one while a newer commit is still in CI.
concurrency:
  group: rolling
  cancel-in-progress: false
permissions:
  contents: read

jobs:
  prepare:
    # Every completed CI run of a push, red ones too: after a red descendant, prepare builds the older green commit
    # it waited for (it builds nothing when there is nothing new).
    if: ${{ github.event_name == 'workflow_dispatch' || github.event.workflow_run.event == 'push' }}
    runs-on: ubuntu-latest
    permissions:
      contents: write
      actions: read        # the CI runs of main: which are green, which still run
    outputs:
      current: ${{ steps.prepare.outputs.current }}
      sha: ${{ steps.prepare.outputs.sha }}
      short: ${{ steps.prepare.outputs.short }}
    steps:
      - id: prepare
        uses: WKDSteve/appkit/release-kit/prepare@<sha> # vX.Y.Z
        with:
          sha: ${{ github.event.inputs.sha }}
          token: ${{ github.token }}

  build:
    needs: prepare
    if: ${{ needs.prepare.outputs.current == 'true' }}
    # ... the app's own matrix: check out needs.prepare.outputs.sha, build, give the file its
    # fixed name, then:
    #   - uses: actions/upload-artifact@<sha>
    #     with: { name: rolling-${{ matrix.os }}, path: <the file> }

  publish:
    needs: [prepare, build]
    runs-on: ubuntu-latest
    permissions:
      contents: write
      id-token: write       # the attestation of the files (see "Attestation"); attest: false needs neither
      attestations: write
    steps:
      - uses: actions/checkout@<sha>
        with:
          ref: ${{ needs.prepare.outputs.sha }}
          persist-credentials: false
      - uses: actions/download-artifact@<sha>
        with: { pattern: rolling-*, path: dist, merge-multiple: true }
      - uses: WKDSteve/appkit/release-kit/publish@<sha> # vX.Y.Z
        with:
          token: ${{ github.token }}
          tag: latest
          sha: ${{ needs.prepare.outputs.sha }}
          files: dist
          expect: 2
          app: my-app
          windows-kind: nsis
          windows-release-asset: my-app-{version}-x64-setup.exe
          windows-rolling-asset: my-app-windows-x86_64-setup.exe
          unix-kind: appimage
          unix-program: my-app
          unix-release-asset: my-app-{version}-x86_64.AppImage
          unix-rolling-asset: my-app-linux-x86_64.AppImage
          notes-file: .github/rolling-notes.md
```

A release (`release.yml`, on a tag `v*`) has the same `build` and `publish` jobs with
`tag: ${{ github.ref_name }}` and no `sha`; its own checks (tag, version and CI agree) come
before the build. Give `windows-release-asset` and `windows-rolling-asset` both in either workflow:
the stamped script of a release still offers `-Rolling`, and the other way round.

`{version}` (the tag without its `v`, release assets only) and `{triple}` (`x86_64-pc-windows-msvc`,
`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`) are filled in by the script on the user's
machine. A `unix-release-asset` or `unix-rolling-asset` without `{triple}` (an AppImage has none) is a
build for Linux x86-64: `install.sh` stops on any other system, in either channel, instead of
downloading a file that would not run there.
`extra` attaches more files of the repository (an `uninstall.ps1`), checked like the rest.

The release page of a tag: `title` is its title (default: the tag; one line), and `{tag}` in the file of
`notes-file` is replaced by the tag, so the notes can show the install line of exactly that release
(`…/releases/download/{tag}/install.ps1`). The rolling build names its commit in its title itself. A draft
that is there already gets the title and the notes of this run.

Before anything is staged or sent, `publish` reads the notes file and the README of the app (`README.md` and
`README.<lang>.md` in the folder the action runs in, the checkout when there is one) and stops when one of
them wraps the install line in a new PowerShell that switches the execution policy off (the bypass switch,
also as `-ep` or `-ex`, and a pipe into `iex` or `Invoke-Expression` on the same line): Microsoft Defender
reports that command line as a trojan (see "The install scripts"). The error names the file and the line.
A sentence that only names the wrapper, without the pipe on the same line, passes.

### A Windows-only app, a self-hosted Windows runner

Both actions run on a Windows runner too. On a self-hosted one `shell: bash` is the WSL launcher, which cannot read
the runner's paths, so each action has a second step that runs the same script (`prepare.sh`, `publish.sh`) with Git
Bash by its full path (`C:\PROGRA~1\Git\bin\bash.exe`, the 8.3 form: the shell string is split at spaces); the plain
`bash` step is skipped there. Git for Windows must be in `C:\Program Files`. For an app that is built for Windows only,
leave the `unix-*` inputs empty and set `expect` to the number of Windows files (one): there is no `install.sh`.
`publish` then needs `python3`, `gh` and `sha256sum` on the runner's PATH (Git Bash brings the last one).

### Windows on the self-hosted runner, Linux and macOS on hosted runners

A public repository gets GitHub-hosted runners for free, so an app whose Windows build stays on the self-hosted runner
can add Linux and macOS as throw-away VMs in the same `build` matrix (reference: `hub-desktop`,
`.github/workflows/rolling.yml`). The parts that matter:

```yaml
  build:
    name: build (${{ matrix.name }})
    runs-on: ${{ matrix.runs-on }}
    continue-on-error: ${{ matrix.experimental }}   # a red new leg leaves the job green
    strategy:
      fail-fast: false
      matrix:
        include:
          - { name: windows, runs-on: [self-hosted, windows], shell: 'C:\PROGRA~1\Git\bin\bash.exe --noprofile --norc -e -o pipefail {0}', program: my-hub.exe, asset: my-hub-windows-x86_64.exe, experimental: false }
          - { name: linux, runs-on: ubuntu-22.04, shell: 'bash --noprofile --norc -e -o pipefail {0}', program: my-hub, asset: my-hub-x86_64-unknown-linux-gnu.tar.gz, experimental: false }
          - { name: macos, runs-on: macos-latest, shell: 'bash --noprofile --norc -e -o pipefail {0}', program: my-hub, asset: my-hub-aarch64-apple-darwin.tar.gz, experimental: true }
    defaults:
      run:
        shell: ${{ matrix.shell }}
    # steps: build; for a .tar.gz put the program at the top of the archive
    #   (cp target/release/$PROGRAM LICENSE stage/ && COPYFILE_DISABLE=1 tar -czf dist/$ASSET -C stage .);
    #   run the app's install test on the program; upload-artifact name rolling-${{ matrix.name }}

  publish:
    steps:
      # ... download-artifact, then count what is there: the required files must be, the experimental one may be
      - id: files
        run: |
          test -f dist/my-hub-windows-x86_64.exe
          test -f dist/my-hub-x86_64-unknown-linux-gnu.tar.gz
          count=2; [ -f dist/my-hub-aarch64-apple-darwin.tar.gz ] && count=3
          echo "count=$count" >> "$GITHUB_OUTPUT"
      - uses: WKDSteve/appkit/release-kit/publish@<sha> # vX.Y.Z
        with:
          expect: ${{ steps.files.outputs.count }}
          unix-kind: hub
          unix-program: my-hub
          unix-release-asset: my-hub-{version}-{triple}.tar.gz
          unix-rolling-asset: my-hub-{triple}.tar.gz   # one name for Linux and macOS: install.sh fills in its own triple
          extra: |
            scripts/uninstall.ps1
            scripts/uninstall.sh
          # ... the windows-* inputs as before
```

- **Names carry the triple.** `install.sh` only knows `Linux-x86_64` and `Darwin-arm64`; with `{triple}` in both asset
  names it downloads the archive of its own system. A name without `{triple}` is a Linux x86-64 build only.
- **`kind: hub` needs the program's own `install` command on Linux and macOS** (the program copies itself to its place).
  An app made of several files (a `bin/` folder, an extension, a player with libraries) is no `hub`: it needs an install
  script of its own beside its archive, as `takt` has (`scripts/install.sh`, `package-release.py`).
- **A new leg starts experimental.** `publish` requires only the legs that have proven themselves; a missing archive of
  the experimental leg is not published, and on the rolling build `publish` takes an earlier one of that name off the
  release (it removes what a build no longer has; a release draft cut again keeps it). Make the leg required after
  some green runs. Name the artifacts by the commit (`rolling-${{ matrix.name }}-${{ needs.prepare.outputs.short }}`,
  the same in the download pattern), so a re-run whose `prepare` picks a newer commit never takes up an older file,
  and let `publish` run on `!cancelled() && needs.build.result != 'failure'`: a cancelled experimental leg (a timeout)
  can make the matrix result `cancelled`, which would skip it.
- **Linux on `ubuntu-22.04`**, not `ubuntu-latest`: a program linked against an older glibc runs on more systems.
  `macos-latest` is arm64 (`aarch64-apple-darwin`). An Intel Mac needs more than a leg: `install.sh` refuses
  `Darwin-x86_64` (it knows `Linux-x86_64` and `Darwin-arm64` only).
- **Native libraries:** a hub with `axum`/`tokio` and no tray needs none on Linux. `cpal` needs `libasound2-dev`
  (and `libdbus-1-dev` for rtkit), a Tauri app the WebKitGTK packages (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`): install them with `apt-get` in the Linux leg.
- **Shell scripts that run on macOS** meet Bash 3.2 (an empty array under `set -u` is unset: `${A[@]+"${A[@]}"}`; no
  `mapfile`, no associative arrays), no `sha256sum` (`shasum -a 256`) and a locale-dependent `sort`/`ls` order
  (`LC_ALL=C`). The hosted macOS runner met the same Bash: `docmap.nvim`'s build script failed there on an empty
  array (fixed in `docmap.nvim@d2add0d`).
- **Never pipe a listing into `grep -q` under `pipefail`:** `tar -tzf a.tar.gz | grep -q x` fails on GNU tar
  (Linux) when it still writes after grep has exited (`tar: stdout: write error`; a line after the match was enough in
  50 of 50 tries), and passes on macOS; write the
  listing to a file first (the first Linux run of `hub-desktop` failed so).
- **The self-hosted Windows CI is unchanged:** the hosted legs only build what CI already proved on Windows; the app's
  CI may add `clippy`/tests on `ubuntu-latest` and `macos-latest` for the code behind `cfg(unix)` (as appkit's own
  `rust-other-systems` job does).

## Attestation

`publish` attests every file it puts on a release, the install scripts and `version.json` included, with
[`actions/attest`](https://github.com/actions/attest): a Sigstore build provenance, signed with the OIDC token of the
workflow, that names the repository, the workflow file, the commit and the kind of runner. It runs between two halves of
`publish.sh`: `STEP=stage` puts the files into a folder and writes their checksums beside it, the attestation takes
exactly those checksums, and `STEP=upload` sends the folder only after checking it still matches them (`version.json`
carries the time of the build, so staging again would make other bytes). The publish job needs `id-token: write` and
`attestations: write`; `attest: false` turns it off. Attestations are free for public repositories; a private one needs
GitHub Enterprise Cloud, so set `attest: false` there.

Check a downloaded file by hand:

```
gh attestation verify <file> -R OWNER/NAME --signer-workflow OWNER/NAME/.github/workflows/rolling.yml
```

(`release.yml` for a release.) What it proves: the file is the one this workflow of this repository built and
published, and a file swapped on the release later, with a token that may write releases but cannot sign as the
workflow, no longer verifies, while `SHA256SUMS` can be swapped along with it. What it does not prove: that the
runner was sound. A compromised runner signs with the same token; only a build on a runner it cannot reach (a
GitHub-hosted one, checked with `--deny-self-hosted-runners`) takes it out of the chain.

The install scripts make the same check after the checksum when `gh` is on the PATH (`--signer-workflow` of the
channel: `rolling.yml` for `latest`, `release.yml` for a release). A file whose attestation does not verify is never
installed; that takes gh's verdict that it is wrong (verification failed, a mismatch). Without `gh`, or when gh cannot
check (a build from before attestations, gh not signed in, no network), they say so with gh's first line and go on;
`-RequireAttestation` / `--require-attestation` makes either a stop. The publish job says at once when attestation is on
but the job has no OIDC token (`id-token: write` missing). Mind where the script came from: the
one-liner takes `install.ps1` from the same release as the file, so a swapped release brings a swapped script that
checks nothing. The check protects a copy of the script you keep, or got elsewhere, and a file you verify by hand.

## The entry in Settings > Apps of an app with an install script of its own

pult, takt and streamtap install from their own `install.ps1` (a payload of several files, not one
program). The entry they write in Settings > Apps, and the check of it in their `test-release.ps1`,
is one text here: `scripts/uninstall-entry.ps1`, dot-sourced from beside the script that uses it.

| Function | What it does |
|---|---|
| `Set-UninstallEntry` | writes the entry (`DisplayName`, `DisplayVersion`, `Publisher`, `InstallLocation`, `DisplayIcon`, `UninstallString`, `NoModify`, `NoRepair`, `EstimatedSize` of the whole folder); says when it took the entry from an install in another folder; refuses a registry root outside HKCU |
| `Get-UninstallEntryCommand` | the command: the installed program with `--uninstall` (appkit-uninstall), and only the places that are not the user's own (`--start-menu-dir`, `--desktop-dir`, `--registry-root`); no PowerShell |
| `Test-UninstallEntry` | what is wrong with an entry, as sentences (none: as written) |
| `Invoke-UninstallEntry` | runs the command of an entry as Settings > Apps would, plus `--yes`, with a temp folder of the test and a time limit |

Each app keeps a copy in its `scripts\` (it ships beside `install.ps1`), and `tools/check-release-kit-copy.py`
(a copy of appkit's) holds the copy to this file at the commit its workflows pin the release-kit actions to.
A fix is made here, then the apps raise that pin and take the file. The tests (below) write only below a
throw-away key `HKCU:\Software\appkit-test-uninstall-entry-*` and remove it.

## Checks

```
python3 -m unittest discover -s release-kit -p "test_*.py" -v
```

`scripts/uninstall-entry.ps1` runs in `pwsh` and Windows PowerShell 5.1 against a throw-away registry key.
The tests run the install scripts against builds made on the spot (a fake hub and a fake setup that
write down their arguments), `install.sh` with the `sh` of the system (dash on Linux, the MSYS `sh`
of Git on Windows), `install.ps1` with `pwsh` and Windows PowerShell 5.1, and `publish.sh` against a
fake `gh` that logs its calls. The download path runs too: `install.sh` against a fake `curl` and
`uname` on the PATH, `install.ps1` against functions that stand in for `Invoke-WebRequest` and
`Invoke-RestMethod` (which URLs, in which channel, what happens without a release or when the API
fails). The CI runs them on Linux and Windows. What they cannot reach is GitHub itself: the first
run of an app's `rolling.yml` is the test of that.
