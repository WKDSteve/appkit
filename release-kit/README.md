# release-kit

How a native app is published, the same way for every app: a release `vX.Y.Z` that a person opens
before it goes out, a rolling build `latest` of the newest green commit of `main`, and on both an
install script that installs exactly that build.

| Part | What it does |
|---|---|
| `prepare/` | A GitHub Action: is the commit of a green CI run the tip of `main`? Makes the pre-release `latest` once. |
| `publish/` | A GitHub Action: puts the built files, the stamped install scripts and `SHA256SUMS` on `latest` (and moves it) or on a release draft. Its steps are `publish.sh`. |
| `scripts/install.ps1` | Windows: downloads the build, checks it against `SHA256SUMS`, installs it for the current user. |
| `scripts/install.sh` | Linux and macOS: the same. |
| `stamp.py` | Writes the repository, the app and the channel into a copy of an install script. |
| `version_json.py` | Writes `version.json`, which `publish.sh` puts on every build (see below). |

The kit builds nothing: the build stays a job of each app (pnpm and cargo, Tauri with its sidecar,
MSYS2 and Inno Setup are too different). It takes over what is the same everywhere: deciding,
uploading, checking, naming.

## The two channels

- **`latest`** is a pre-release that is never marked "Latest". After every green CI run on `main`,
  the build of that commit replaces its files, which keep **fixed names without a version**
  (`releases/download/latest/<file>` always works). Only the tip of `main` is built, so a late run
  of an older commit never moves it backwards. `SHA256SUMS` goes first, files of an earlier build that
  this one no longer has go, and only at the end does the tag move to the commit and the page name it:
  a run that stops in between leaves the page naming the previous commit.
- **`vX.Y.Z`** is a release from a tag, left as a **draft** (NREL-07): a person installs it, opens it,
  and publishes it. A published release is never touched again (NREL-10). A draft that is there
  already (the tag was set again on a newer commit) gets the files and the notes of this run.

The tag `latest` is moved with the token of the workflow. Should GitHub refuse that (a token may not
point a ref past a change of `.github/workflows` without the `workflows` right), the run only warns:
the downloads go by the name of the release, not by the commit of its tag, and the page still names
the commit of its files. A repository with **immutable releases** turned on cannot have a rolling
build: its files could never be replaced.

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
`v1.2.0` installs v1.2.0, the script of `latest` installs the rolling build, both without options:

```
powershell -ExecutionPolicy Bypass -c "irm https://github.com/OWNER/NAME/releases/download/latest/install.ps1 | iex"
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
release: it is no signature, and it replaces none. `-ExecutionPolicy Bypass` and `| iex` are needed
because a script downloaded by the browser is marked as well. Neither helps against AppLocker or
WDAC: what those forbid stays forbidden.

The scripts install **for the current user, without administrator rights**, and refuse an elevated
session (`-AllowElevated` / `--allow-root` for a build machine): elevated, the app would be installed
into the profile of the administrator account.

| Kind | Script | Install step |
|---|---|---|
| `hub` | both | The program (bare, or the only thing in a `.zip`/`.tar.gz`) runs `<program> install ARGS`, which copies itself to its fixed place. |
| `nsis` | `install.ps1` | The NSIS setup runs with `/S`. It installs for the current user only when the setup is built so (Tauri's default `installMode: "currentUser"`); a per-machine setup asks for administrator rights. |
| `inno` | `install.ps1` | The Inno setup runs with `/VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER /NORESTART`. It installs for the current user when the setup has `PrivilegesRequired=lowest` (or allows the override on the command line, `PrivilegesRequiredOverridesAllowed=commandline`). |
| `appimage` | `install.sh` | The AppImage is copied to `${XDG_BIN_HOME:-~/.local/bin}/<program>`. |

Options of both scripts: another release (`-Version vX.Y.Z` / `--version`), the rolling build
(`-Rolling` / `--rolling`), a file you already have (`-File` / `--file`, with `-Sums` / `-Sha256`),
more arguments for the install step (`-InstallArgs '--autostart'` / `-- --autostart`). An unstamped
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

Pin the actions by commit, with the tag in a comment (NREL-03). The rolling build, in
`.github/workflows/rolling.yml` of the app:

```yaml
name: rolling
on:
  workflow_run:
    workflows: [ci]
    types: [completed]
    branches: [main]
# Only the run of the tip may build; a newer tip replaces a build that is still running.
concurrency:
  group: rolling-${{ github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'push' && github.event.workflow_run.head_sha == github.sha }}
  cancel-in-progress: true
permissions:
  contents: read

jobs:
  prepare:
    if: ${{ github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'push' }}
    runs-on: ubuntu-latest
    permissions:
      contents: write
    outputs:
      current: ${{ steps.prepare.outputs.current }}
      sha: ${{ steps.prepare.outputs.sha }}
      short: ${{ steps.prepare.outputs.short }}
    steps:
      - id: prepare
        uses: WKDSteve/appkit/release-kit/prepare@<sha> # vX.Y.Z
        with:
          sha: ${{ github.event.workflow_run.head_sha }}
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
`tag: ${{ github.ref_name }}` and no `sha`; its own checks (tag, version and CI agree, NREL-08) come
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

### A Windows-only app, a self-hosted Windows runner

Both actions run on a Windows runner too. On a self-hosted one `shell: bash` is the WSL launcher, which cannot read
the runner's paths, so each action has a second step that runs the same script (`prepare.sh`, `publish.sh`) with Git
Bash by its full path (`C:\PROGRA~1\Git\bin\bash.exe`, the 8.3 form: the shell string is split at spaces); the plain
`bash` step is skipped there. Git for Windows must be in `C:\Program Files`. For an app that is built for Windows only,
leave the `unix-*` inputs empty and set `expect` to the number of Windows files (one): there is no `install.sh`.
`publish` then needs `python3`, `gh` and `sha256sum` on the runner's PATH (Git Bash brings the last one).

## Checks

```
python3 -m unittest discover -s release-kit -p "test_*.py" -v
```

The tests run the install scripts against builds made on the spot (a fake hub and a fake setup that
write down their arguments), `install.sh` with the `sh` of the system (dash on Linux, the MSYS `sh`
of Git on Windows), `install.ps1` with `pwsh` and Windows PowerShell 5.1, and `publish.sh` against a
fake `gh` that logs its calls. The download path runs too: `install.sh` against a fake `curl` and
`uname` on the PATH, `install.ps1` against functions that stand in for `Invoke-WebRequest` and
`Invoke-RestMethod` (which URLs, in which channel, what happens without a release or when the API
fails). The CI runs them on Linux and Windows. What they cannot reach is GitHub itself: the first
run of an app's `rolling.yml` is the test of that.
