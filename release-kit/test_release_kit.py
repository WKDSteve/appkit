"""Tests of the release kit: stamping, the install scripts and publish.sh.

    python -m unittest discover -s release-kit -p "test_*.py" -v

The install scripts run against builds made here (a fake hub that writes its arguments to a file, a
fake setup that does the same), never against a download; publish.sh runs against a fake `gh` that
logs its calls. install.ps1 needs Windows (it asks for the Windows account); install.sh and
publish.sh need a POSIX shell (Git Bash on Windows).
"""

from __future__ import annotations

import hashlib
import http.server
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
import zipfile
from pathlib import Path

KIT = Path(__file__).resolve().parent
sys.path.insert(0, str(KIT))

import stamp  # noqa: E402
import version_json  # noqa: E402

# The install scripts download through the API when a token is set (a private repository); a
# token of the person who runs the tests must not turn the other tests into API calls.
for _name in ("GH_TOKEN", "GITHUB_TOKEN"):
    os.environ.pop(_name, None)

PS1 = KIT / "scripts" / "install.ps1"
SH = KIT / "scripts" / "install.sh"
PUBLISH = KIT / "publish.sh"
WINDOWS = os.name == "nt"


def git_bash(name: str) -> str | None:
    """A POSIX shell; on Windows the one of Git (the `bash` on PATH may be the WSL launcher).

    Git/usr/bin, not Git/bin: the launcher in Git/bin puts its own folders before the PATH it is
    given, and the fake tools of the tests would never be found.
    """
    if WINDOWS:
        for base in (os.environ.get("ProgramFiles"), os.environ.get("ProgramW6432"), r"C:\Program Files"):
            if base:
                candidate = Path(base) / "Git" / "usr" / "bin" / f"{name}.exe"
                if candidate.is_file():
                    return str(candidate)
        return None
    return shutil.which(name)


SH_EXE = git_bash("sh")
BASH_EXE = git_bash("bash")


def posix_path_env(*dirs: Path) -> str:
    """A PATH that puts `dirs` first, as the shell of the system reads it (Git Bash: /c/...)."""
    if WINDOWS:
        posix = ["/" + d.as_posix().replace(":", "", 1) for d in dirs]
        return ":".join(posix + ["/usr/bin", "/bin", os.environ["PATH"]])
    return os.pathsep.join([str(d) for d in dirs] + [os.environ["PATH"]])


def fake_tool(folder: Path, name: str, text: str) -> None:
    folder.mkdir(exist_ok=True)
    tool = folder / name
    tool.write_text(text, encoding="utf-8", newline="\n")
    tool.chmod(0o755)


# curl as install.sh calls it: `-o FILE URL` (and `-w '%{http_code}'` for the API). Files are served
# by their name from $FIXTURES and every URL is logged; $FAKE_NEWEST is the answer of the API
# (200 with $FAKE_TAG, 404, or another code).
FAKE_CURL = r"""#!/bin/sh
out=""; write=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) write="$2"; shift 2 ;;
    -H) printf '%s\n' "$2" >> "$CURL_LOG.headers"; shift 2 ;;
    --proto|--proto-redir) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
printf '%s\n' "$url" >> "$CURL_LOG"
case "$url" in
  */releases/tags/*)
    # What the API says of a release: the files of the fixtures, each with an id (100, 101, ...).
    n=100
    {
      printf '{\n  "assets": [\n'
      for f in $(ls "$FIXTURES"); do
        printf '    {\n      "url": "https://api.github.com/repos/o/r/releases/assets/%s",\n      "id": %s,\n      "node_id": "N%s",\n      "name": "%s",\n      "size": 1\n    },\n' "$n" "$n" "$n" "$f"
        n=$((n + 1))
      done
      printf '    {}\n  ]\n}\n'
    } > "$out"
    exit 0 ;;
  */releases/assets/*)
    n=100
    for f in $(ls "$FIXTURES"); do
      [ "$n" = "${url##*/}" ] && { cp "$FIXTURES/$f" "$out"; exit 0; }
      n=$((n + 1))
    done
    exit 22 ;;
  */releases/latest)
    code="${FAKE_NEWEST:-404}"
    [ "$code" = 200 ] && printf '{"tag_name": "%s"}' "$FAKE_TAG" > "$out"
    printf '%s' "$code"
    exit 0 ;;
esac
name="${url##*/}"
[ -f "$FIXTURES/$name" ] || exit 22
cp "$FIXTURES/$name" "$out"
"""

FAKE_UNAME = """#!/bin/sh
case "$1" in -m) echo "${FAKE_MACHINE:-x86_64}" ;; *) echo "${FAKE_SYSTEM:-Linux}" ;; esac
"""
POWERSHELLS = [p for p in (shutil.which("pwsh"), shutil.which("powershell")) if p] if WINDOWS else []


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_sums(folder: Path, *files: Path) -> Path:
    sums = folder / "SHA256SUMS"
    sums.write_text("".join(f"{sha(f)}  {f.name}\n" for f in files), encoding="utf-8", newline="\n")
    return sums


def values(**over: str) -> dict[str, str]:
    base = {
        "repo": "Owner/app-repo",
        "channel": "latest",
        "app": "demo",
        "kind": "hub",
        "program": "demo-hub",
        "release_asset": "demo-hub-{version}-{triple}.zip",
        "rolling_asset": "demo-hub-windows-x86_64.exe",
    }
    base.update(over)
    return base


class FakeGitHub:
    """A local server that answers like the GitHub API of a private repository, redirect included.

    The release by tag lists the assets; an asset address answers 302 to a "storage" address (what the
    real API does, to another host) and only to the right token; the storage address answers the bytes
    and must never see the token. `requests` logs (path, Authorization header) of every request.
    Real HTTP, because the faked cmdlets of the other tests cannot show how a PowerShell treats a 302.
    """

    def __init__(self, token: str, repo: str, tag: str, files: dict[str, bytes], redirect: bool = True) -> None:
        self.requests: list[tuple[str, str | None]] = []
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args: object) -> None:
                pass

            def reply(self, code: int, body: bytes = b"", **headers: str) -> None:
                self.send_response(code)
                for key, value in headers.items():
                    self.send_header(key.replace("_", "-"), value)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self) -> None:  # noqa: N802
                auth = self.headers.get("Authorization")
                outer.requests.append((self.path, auth))
                if self.path == f"/repos/{repo}/releases/tags/{tag}":
                    assets = [{"name": name, "url": f"{outer.base}/assets/{name}"} for name in files]
                    self.reply(200, json.dumps({"tag_name": tag, "assets": assets}).encode(),
                               Content_Type="application/json")
                elif self.path.startswith("/assets/"):
                    name = self.path.rsplit("/", 1)[1]
                    if auth != f"Bearer {token}":
                        self.reply(401)
                    elif not redirect:
                        self.reply(200, files[name], Content_Type="application/octet-stream")
                    else:
                        self.reply(302, Location=f"{outer.base}/storage/{self.path.rsplit('/', 1)[1]}")
                elif self.path.startswith("/storage/"):
                    name = self.path.rsplit("/", 1)[1]
                    if name in files:
                        self.reply(200, files[name], Content_Type="application/octet-stream")
                    else:
                        self.reply(404)
                else:
                    self.reply(404)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()


class ManifestsTest(unittest.TestCase):
    """The action manifests are YAML the runner can read. A ': ' in a plain description made `publish` of
    v0.19.0 unloadable on GitHub, and nothing here had parsed it. PyYAML is on the Linux runner of the CI
    (required there); elsewhere the test is skipped when it is missing."""

    def yaml(self):
        try:
            import yaml
        except ImportError:
            if os.environ.get("RUNNER_OS") == "Linux":
                self.fail("PyYAML is needed on the Linux runner to read the action manifests")
            self.skipTest("PyYAML is not installed")
        return yaml

    def test_every_action_manifest_parses(self) -> None:
        yaml = self.yaml()
        manifests = sorted(KIT.glob("*/action.yml"))
        self.assertGreaterEqual(len(manifests), 2)
        for path in manifests:
            with self.subTest(action=path.parent.name):
                doc = yaml.safe_load(path.read_text(encoding="utf-8"))
                self.assertIsInstance(doc, dict)
                self.assertEqual(doc["runs"]["using"], "composite")
                for name, spec in (doc.get("inputs") or {}).items():
                    self.assertIn("description", spec, f"input {name}")

    def test_every_input_a_manifest_reads_is_declared(self) -> None:
        import re
        yaml = self.yaml()
        for path in sorted(KIT.glob("*/action.yml")):
            with self.subTest(action=path.parent.name):
                text = path.read_text(encoding="utf-8")
                declared = set((yaml.safe_load(text).get("inputs") or {}).keys())
                used = set(re.findall(r"\$\{\{\s*inputs\.([A-Za-z0-9_-]+)\s*\}\}", text))
                self.assertEqual(used - declared, set(), "read but not declared")
                self.assertEqual(declared - used, set(), "declared but never read")


    def test_every_bash_step_has_a_git_bash_twin_for_windows(self) -> None:
        # On a self-hosted Windows runner `shell: bash` is the WSL launcher, which cannot read the runner's
        # Windows paths: the step runs there by the full path of Git Bash (8.3 form, the shell string is split
        # at spaces), and the bash one is skipped on Windows.
        yaml = self.yaml()
        for path in sorted(KIT.glob("*/action.yml")):
            with self.subTest(action=path.parent.name):
                steps = yaml.safe_load(path.read_text(encoding="utf-8"))["runs"]["steps"]
                bash = [s for s in steps if s.get("shell") == "bash"]
                git_bash = [s for s in steps if str(s.get("shell", "")).lower().startswith("c:\\progra~1\\git\\bin\\bash.exe")]
                self.assertTrue(bash, "a bash step for Linux and macOS")
                self.assertEqual(len(bash), len(git_bash), "one Git Bash twin per bash step")
                for s in bash:
                    self.assertEqual(s.get("if"), "runner.os != 'Windows'")
                for s in git_bash:
                    self.assertEqual(s.get("if"), "runner.os == 'Windows'")


class StampTest(unittest.TestCase):
    def test_ps1_gets_every_value_and_nothing_else_changes(self) -> None:
        source = PS1.read_text(encoding="utf-8")
        out = stamp.stamp(source, ".ps1", values(channel="v1.2.3"))
        self.assertIn("$StampRepo = 'Owner/app-repo'", out)
        self.assertIn("$StampChannel = 'v1.2.3'", out)
        self.assertIn("$StampKind = 'hub'", out)
        self.assertIn("$StampReleaseAsset = 'demo-hub-{version}-{triple}.zip'", out)
        changed = [(a, b) for a, b in zip(source.splitlines(), out.splitlines()) if a != b]
        self.assertEqual(len(changed), 7, changed)
        self.assertEqual(len(source.splitlines()), len(out.splitlines()))

    def test_sh_gets_every_value(self) -> None:
        out = stamp.stamp(SH.read_text(encoding="utf-8"), ".sh", values(kind="appimage", program="demo"))
        self.assertIn('STAMP_REPO="Owner/app-repo"', out)
        self.assertIn('STAMP_CHANNEL="latest"', out)
        self.assertIn('STAMP_KIND="appimage"', out)

    def test_refuses_what_a_field_may_not_hold(self) -> None:
        text = PS1.read_text(encoding="utf-8")
        for bad in (
            values(repo="no-slash"),
            values(repo="a/b'; rm"),
            values(channel="main"),
            values(app="two words"),
            values(kind="msi"),
            values(program=""),
            values(rolling_asset=""),
            values(channel="v1.0.0", release_asset=""),
            values(rolling_asset="dir/file.exe"),
            values(release_asset="$(evil)"),
            values(rolling_asset="demo.exe\n"),
            values(repo="Owner/x\n"),
            values(channel="latest\n"),
        ):
            with self.subTest(bad=bad), self.assertRaises(stamp.StampError):
                stamp.stamp(text, ".ps1", bad)

    def test_a_setup_needs_no_program(self) -> None:
        out = stamp.stamp(PS1.read_text(encoding="utf-8"), ".ps1", values(kind="nsis", program=""))
        self.assertIn("$StampKind = 'nsis'", out)

    def test_kinds_per_script(self) -> None:
        with self.assertRaises(stamp.StampError):
            stamp.stamp(SH.read_text(encoding="utf-8"), ".sh", values(kind="nsis"))
        with self.assertRaises(stamp.StampError):
            stamp.stamp(PS1.read_text(encoding="utf-8"), ".ps1", values(kind="appimage"))

    def test_a_stamped_script_is_not_stamped_again(self) -> None:
        once = stamp.stamp(PS1.read_text(encoding="utf-8"), ".ps1", values())
        with self.assertRaises(stamp.StampError):
            stamp.stamp(once, ".ps1", values())

    def test_no_block_no_stamp(self) -> None:
        with self.assertRaises(stamp.StampError):
            stamp.stamp("echo hi\n", ".sh", values())

    def test_cli_writes_the_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "install.sh"
            code = stamp.main([
                str(SH), str(out), "--repo", "O/N", "--channel", "latest", "--app", "a",
                "--kind", "hub", "--program", "p", "--rolling-asset", "p.tar.gz",
            ])
            self.assertEqual(code, 0)
            self.assertIn('STAMP_ROLLING_ASSET="p.tar.gz"', out.read_text(encoding="utf-8"))
            self.assertEqual(stamp.main([str(SH), str(out), "--repo", "bad", "--channel", "latest",
                                         "--app", "a", "--kind", "hub", "--program", "p"]), 1)


FAKE_HUB_SH = """#!/bin/sh
if [ "$1" = "--version" ]; then echo "fake-hub 1.0"; exit 0; fi
printf '%s\\n' "$*" > "$OUT"
"""


@unittest.skipUnless(SH_EXE, "no POSIX sh")
class InstallShTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.out = self.tmp / "out.txt"
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def run_sh(self, script: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        full = dict(os.environ, OUT=self.out.as_posix(), **env)
        return subprocess.run([SH_EXE, script.as_posix(), *args], capture_output=True, text=True, env=full)

    def hub_archive(self, entries: dict[str, str] | None = None) -> Path:
        archive = self.tmp / "demo-hub.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            for name, text in (entries or {"demo-hub": FAKE_HUB_SH}).items():
                data = text.encode()
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o755
                tar.addfile(info, io.BytesIO(data))
        return archive

    def test_hub_archive_runs_install_with_the_arguments(self) -> None:
        archive = self.hub_archive()
        write_sums(self.tmp, archive)
        res = self.run_sh(SH, "--file", archive.as_posix(), "--kind", "hub", "--program", "demo-hub",
                          "--allow-root", "--", "--autostart")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("checksum ok", res.stdout)
        self.assertEqual(self.out.read_text().strip(), "install --autostart")

    def test_a_wrong_checksum_installs_nothing(self) -> None:
        archive = self.hub_archive()
        res = self.run_sh(SH, "--file", archive.as_posix(), "--sha256", "0" * 64, "--kind", "hub",
                          "--program", "demo-hub", "--allow-root")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("does not match", res.stderr)
        self.assertFalse(self.out.exists())

    def test_a_file_not_in_the_sums_installs_nothing(self) -> None:
        archive = self.hub_archive()
        other = self.tmp / "other.txt"
        other.write_text("x")
        write_sums(self.tmp, other)
        res = self.run_sh(SH, "--file", archive.as_posix(), "--kind", "hub", "--program", "demo-hub",
                          "--allow-root")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("is not listed", res.stderr)
        self.assertFalse(self.out.exists())

    def test_a_path_out_of_the_archive_is_refused(self) -> None:
        archive = self.hub_archive({"demo-hub": FAKE_HUB_SH, "../evil": "x"})
        write_sums(self.tmp, archive)
        res = self.run_sh(SH, "--file", archive.as_posix(), "--kind", "hub", "--program", "demo-hub",
                          "--allow-root")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("leaves its folder", res.stderr)
        self.assertFalse(self.out.exists())

    def test_a_bare_file_name_runs_that_file_not_one_on_the_path(self) -> None:
        # An old program of the same name on the PATH must not be what runs after the new one was checked.
        program = self.tmp / "demo-hub"
        program.write_text(FAKE_HUB_SH, encoding="utf-8", newline="\n")
        write_sums(self.tmp, program)
        old = self.tmp / "old"
        fake_tool(old, "demo-hub", '#!/bin/sh\nprintf "OLD %s\\n" "$*" > "$OUT"\n')
        res = subprocess.run(
            [SH_EXE, SH.as_posix(), "--file", "demo-hub", "--kind", "hub", "--program", "demo-hub", "--allow-root"],
            capture_output=True, text=True, cwd=self.tmp,
            env=dict(os.environ, OUT=self.out.as_posix(), PATH=posix_path_env(old)),
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(self.out.read_text().strip(), "install")

    def download(self, script: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        tools = self.tmp / "tools"
        fake_tool(tools, "curl", FAKE_CURL)
        fake_tool(tools, "uname", FAKE_UNAME)
        # MSYS=noglob: started from Windows, the MSYS runtime globs its command line ({version} -> version).
        full = dict(os.environ, OUT=self.out.as_posix(), PATH=posix_path_env(tools), MSYS="noglob",
                    FIXTURES=(self.tmp / "fixtures").as_posix(), CURL_LOG=(self.tmp / "curl.log").as_posix())
        full.update(env)
        return subprocess.run([SH_EXE, script.as_posix(), "--allow-root", *args], capture_output=True, text=True, env=full)

    def fixtures(self, name: str) -> None:
        folder = self.tmp / "fixtures"
        folder.mkdir()
        archive = self.hub_archive()
        archive.rename(folder / name)
        write_sums(folder, folder / name)

    def urls(self) -> list[str]:
        log = self.tmp / "curl.log"
        return log.read_text().splitlines() if log.exists() else []

    def stamped(self, **over: str) -> Path:
        script = self.tmp / "install.sh"
        script.write_text(stamp.stamp(SH.read_text(encoding="utf-8"), ".sh", values(**over)), encoding="utf-8", newline="\n")
        return script

    def test_the_stamped_release_downloads_its_own_version(self) -> None:
        self.fixtures("demo-hub-1.2.3-x86_64-unknown-linux-gnu.tar.gz")
        script = self.stamped(channel="v1.2.3", release_asset="demo-hub-{version}-{triple}.tar.gz")
        res = self.download(script)
        self.assertEqual(res.returncode, 0, res.stderr)
        base = "https://github.com/Owner/app-repo/releases/download/v1.2.3/"
        self.assertEqual(self.urls(), [base + "demo-hub-1.2.3-x86_64-unknown-linux-gnu.tar.gz", base + "SHA256SUMS"])
        self.assertEqual(self.out.read_text().strip(), "install")

    def test_a_token_downloads_through_the_api(self) -> None:
        # A private repository answers the public address with 404: with a token the files come
        # through the API (the release by tag, then each asset by id), the token in the header.
        self.fixtures("demo-hub-1.2.3-x86_64-unknown-linux-gnu.tar.gz")
        script = self.stamped(channel="v1.2.3", release_asset="demo-hub-{version}-{triple}.tar.gz")
        res = self.download(script, GH_TOKEN="secret-token")
        self.assertEqual(res.returncode, 0, res.stderr)
        api = "https://api.github.com/repos/Owner/app-repo/releases/"
        urls = self.urls()
        self.assertEqual(len(urls), 4, urls)
        self.assertTrue(all(u.startswith(api) for u in urls), urls)
        self.assertEqual([urls[0], urls[2]], [api + "tags/v1.2.3"] * 2)
        self.assertIn("Authorization: Bearer secret-token", (self.tmp / "curl.log.headers").read_text())
        self.assertEqual(self.out.read_text().strip(), "install")

    def test_without_a_token_the_header_is_never_sent(self) -> None:
        self.fixtures("demo-hub-1.2.3-x86_64-unknown-linux-gnu.tar.gz")
        script = self.stamped(channel="v1.2.3", release_asset="demo-hub-{version}-{triple}.tar.gz")
        res = self.download(script)
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertFalse((self.tmp / "curl.log.headers").exists())

    def test_the_stamped_rolling_copy_downloads_latest(self) -> None:
        self.fixtures("demo-hub-linux-x86_64.tar.gz")
        res = self.download(self.stamped(rolling_asset="demo-hub-linux-x86_64.tar.gz"))
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(self.urls()[0], "https://github.com/Owner/app-repo/releases/download/latest/demo-hub-linux-x86_64.tar.gz")

    def test_the_rolling_build_of_linux_is_not_installed_on_a_mac(self) -> None:
        self.fixtures("demo-hub-linux-x86_64.tar.gz")
        res = self.download(self.stamped(rolling_asset="demo-hub-linux-x86_64.tar.gz"),
                            FAKE_SYSTEM="Darwin", FAKE_MACHINE="arm64")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("for Linux x86-64 only", res.stderr)
        self.assertEqual(self.urls(), [], "nothing is downloaded")

    def test_the_release_of_linux_is_not_installed_on_a_mac_either(self) -> None:
        # An AppImage has no system in its name: on a Mac it would download, pass its checksum and not run.
        self.fixtures("demo_1.2.3_amd64.AppImage")
        script = self.stamped(channel="v1.2.3", kind="appimage", program="demo",
                              release_asset="demo_{version}_amd64.AppImage")
        res = self.download(script, FAKE_SYSTEM="Darwin", FAKE_MACHINE="arm64")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("for Linux x86-64 only", res.stderr)
        self.assertEqual(self.urls(), [], "nothing is downloaded")

    def test_an_appimage_release_installs_on_linux(self) -> None:
        self.fixtures("demo_1.2.3_amd64.AppImage")
        bin_dir = self.tmp / "bin"
        script = self.stamped(channel="v1.2.3", kind="appimage", program="demo",
                              release_asset="demo_{version}_amd64.AppImage")
        res = self.download(script, XDG_BIN_HOME=bin_dir.as_posix())
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertTrue((bin_dir / "demo").is_file())

    def test_a_rolling_build_named_by_its_system(self) -> None:
        self.fixtures("demo-hub-aarch64-apple-darwin.tar.gz")
        res = self.download(self.stamped(rolling_asset="demo-hub-{triple}.tar.gz"),
                            FAKE_SYSTEM="Darwin", FAKE_MACHINE="arm64")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(self.urls()[0],
                         "https://github.com/Owner/app-repo/releases/download/latest/demo-hub-aarch64-apple-darwin.tar.gz")

    def test_no_channel_takes_the_newest_release(self) -> None:
        self.fixtures("demo-hub-9.9.9-x86_64-unknown-linux-gnu.tar.gz")
        res = self.download(SH, "--repo", "Owner/app-repo", "--kind", "hub", "--program", "demo-hub",
                            "--release-asset", "demo-hub-{version}-{triple}.tar.gz", FAKE_NEWEST="200", FAKE_TAG="v9.9.9")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("/download/v9.9.9/demo-hub-9.9.9-x86_64-unknown-linux-gnu.tar.gz", self.urls()[1])

    def test_no_release_at_all_takes_the_rolling_build(self) -> None:
        self.fixtures("demo-hub-linux-x86_64.tar.gz")
        res = self.download(SH, "--repo", "Owner/app-repo", "--kind", "hub", "--program", "demo-hub",
                            "--rolling-asset", "demo-hub-linux-x86_64.tar.gz", FAKE_NEWEST="404")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("/download/latest/demo-hub-linux-x86_64.tar.gz", self.urls()[1])

    def test_an_api_failure_is_not_the_rolling_build(self) -> None:
        self.fixtures("demo-hub-linux-x86_64.tar.gz")
        res = self.download(SH, "--repo", "Owner/app-repo", "--kind", "hub", "--program", "demo-hub",
                            "--rolling-asset", "demo-hub-linux-x86_64.tar.gz", FAKE_NEWEST="403")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("answered 403", res.stderr)
        self.assertEqual(len(self.urls()), 1, "nothing is downloaded")
        self.assertFalse(self.out.exists())

    def test_missing_sums_of_the_rolling_build_say_so(self) -> None:
        self.fixtures("demo-hub-linux-x86_64.tar.gz")
        (self.tmp / "fixtures" / "SHA256SUMS").unlink()
        res = self.download(self.stamped(rolling_asset="demo-hub-linux-x86_64.tar.gz"))
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("a build is running", res.stderr)
        self.assertFalse(self.out.exists())

    def test_appimage_is_copied_and_made_executable(self) -> None:
        image = self.tmp / "demo.AppImage"
        image.write_bytes(b"\x7fELF fake image")
        write_sums(self.tmp, image)
        bin_dir = self.tmp / "bin"
        res = self.run_sh(SH, "--file", image.as_posix(), "--kind", "appimage", "--program", "demo",
                          "--allow-root", XDG_BIN_HOME=bin_dir.as_posix())
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual((bin_dir / "demo").read_bytes(), image.read_bytes())
        self.assertFalse((bin_dir / ".demo.new").exists())

    def test_an_unstamped_copy_wants_its_kind(self) -> None:
        archive = self.hub_archive()
        write_sums(self.tmp, archive)
        res = self.run_sh(SH, "--file", archive.as_posix(), "--allow-root")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("not stamped", res.stderr)

    def test_a_stamped_copy_needs_no_options(self) -> None:
        stamped = self.tmp / "install.sh"
        stamped.write_text(stamp.stamp(SH.read_text(encoding="utf-8"), ".sh", values(
            program="demo-hub", rolling_asset="demo-hub.tar.gz")), encoding="utf-8", newline="\n")
        archive = self.hub_archive()
        write_sums(self.tmp, archive)
        res = self.run_sh(stamped, "--file", archive.as_posix(), "--allow-root")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertEqual(self.out.read_text().strip(), "install")


FAKE_CMD = "@echo off\r\nif \"%~1\"==\"--version\" (echo fake-hub 1.0& exit /b 0)\r\necho %*> \"%OUT%\"\r\nexit /b %FAKE_EXIT%\r\n"


@unittest.skipUnless(POWERSHELLS, "install.ps1 runs on Windows")
class InstallPs1Test(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.out = self.tmp / "out.txt"
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def run_ps(self, shell: str, script: Path, args: str, fake_exit: str = "0", prelude: str = "",
               cwd: Path | None = None, **env: str) -> subprocess.CompletedProcess[str]:
        full = dict(os.environ, OUT=str(self.out), FAKE_EXIT=fake_exit)
        full.update(env)
        command = f"{prelude}& '{script}' {args} -AllowElevated"
        # errors="replace": a PowerShell on a German system writes its messages in the OEM code page, and a
        # test that only wants the exit code must not die on an umlaut before it can say what failed.
        return subprocess.run([shell, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                               "-Command", command], capture_output=True, text=True, encoding="utf-8",
                              errors="replace", env=full, cwd=cwd)

    def fake(self, name: str) -> Path:
        path = self.tmp / name
        path.write_bytes(FAKE_CMD.encode())
        return path

    def out_text(self) -> str:
        return self.out.read_text().strip() if self.out.exists() else ""

    def each_shell(self):
        for shell in POWERSHELLS:
            if self.out.exists():
                self.out.unlink()
            with self.subTest(shell=Path(shell).name):
                yield shell

    def test_hub_zip_runs_install_with_the_arguments(self) -> None:
        program = self.fake("demo-hub.cmd")
        archive = self.tmp / "demo-hub.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.write(program, "demo-hub.cmd")
        program.unlink()
        write_sums(self.tmp, archive)
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, f"-File '{archive}' -Kind hub -Program demo-hub.cmd -InstallArgs '--autostart'")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertIn("checksum ok", res.stdout)
            self.assertEqual(self.out_text(), "install --autostart")

    def test_bare_hub_program(self) -> None:
        program = self.fake("demo-hub.cmd")
        write_sums(self.tmp, program)
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, f"-File '{program}' -Kind hub -Program demo-hub.cmd")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertEqual(self.out_text(), "install")

    def test_nsis_and_inno_run_silently(self) -> None:
        setup = self.fake("demo-setup.cmd")
        write_sums(self.tmp, setup)
        for kind, flags in (("nsis", "/S"), ("inno", "/VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER /NORESTART")):
            for shell in self.each_shell():
                res = self.run_ps(shell, PS1, f"-File '{setup}' -Kind {kind}")
                self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
                self.assertEqual(self.out_text(), flags)

    def test_a_failing_setup_fails(self) -> None:
        setup = self.fake("demo-setup.cmd")
        write_sums(self.tmp, setup)
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, f"-File '{setup}' -Kind nsis", fake_exit="3")
            self.assertNotEqual(res.returncode, 0)
            self.assertIn("exit code 3", res.stdout + res.stderr)

    def test_a_wrong_checksum_installs_nothing(self) -> None:
        setup = self.fake("demo-setup.cmd")
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, f"-File '{setup}' -Sha256 {'0' * 64} -Kind nsis")
            self.assertNotEqual(res.returncode, 0)
            self.assertIn("does not match", res.stdout + res.stderr)
            self.assertFalse(self.out.exists())

    def test_a_path_out_of_the_archive_is_refused(self) -> None:
        archive = self.tmp / "demo-hub.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("demo-hub.cmd", FAKE_CMD)
            zf.writestr("../evil.txt", "x")
        write_sums(self.tmp, archive)
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, f"-File '{archive}' -Kind hub -Program demo-hub.cmd")
            self.assertNotEqual(res.returncode, 0)
            self.assertIn("leaves its folder", res.stdout + res.stderr)
            self.assertFalse(self.out.exists())

    def test_a_path_with_a_space_out_of_the_archive_is_refused(self) -> None:
        archive = self.tmp / "demo-hub.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("demo-hub.cmd", FAKE_CMD)
            zf.writestr(".. /escaped.txt", "x")
        write_sums(self.tmp, archive)
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, f"-File '{archive}' -Kind hub -Program demo-hub.cmd")
            self.assertNotEqual(res.returncode, 0)
            self.assertFalse(self.out.exists())

    def test_a_bare_file_name_runs_that_file_not_one_on_the_path(self) -> None:
        self.fake("demo-hub.cmd")
        write_sums(self.tmp, self.tmp / "demo-hub.cmd")
        old = self.tmp / "old"
        old.mkdir()
        (old / "demo-hub.cmd").write_bytes(b"@echo off\r\necho OLD %*> \"%OUT%\"\r\n")
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, "-File demo-hub.cmd -Kind hub -Program demo-hub.cmd", cwd=self.tmp,
                              PATH=f"{old}{os.pathsep}{os.environ['PATH']}")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertEqual(self.out_text(), "install")

    # Invoke-WebRequest and Invoke-RestMethod as functions: they win over the cmdlets inside the
    # script. Files come from $env:FIXTURES by their name; every URL is logged; $env:FAKE_NEWEST is
    # the answer of the API (200 with $env:FAKE_TAG, or a status code the exception carries).
    FAKE_WEB = (
        "function Invoke-WebRequest { param($Uri, $OutFile, [switch] $UseBasicParsing) "
        "Add-Content -LiteralPath $env:WEB_LOG -Value $Uri; "
        "$src = Join-Path $env:FIXTURES ($Uri -split '/')[-1]; "
        "if (-not (Test-Path -LiteralPath $src)) { throw \"404 $Uri\" }; Copy-Item -LiteralPath $src -Destination $OutFile }; "
        "function Invoke-RestMethod { param($Uri, [switch] $UseBasicParsing) "
        "Add-Content -LiteralPath $env:WEB_LOG -Value $Uri; "
        "if ($env:FAKE_NEWEST -eq '200') { return [pscustomobject]@{ tag_name = $env:FAKE_TAG } }; "
        "$e = New-Object System.Exception ('status ' + $env:FAKE_NEWEST); "
        "$e | Add-Member -NotePropertyName Response -NotePropertyValue ([pscustomobject]@{ StatusCode = [int] $env:FAKE_NEWEST }); "
        "throw $e }; "
    )

    def download(self, shell: str, script: Path, args: str = "", **env: str) -> subprocess.CompletedProcess[str]:
        log = self.tmp / "web.log"
        if log.exists():
            log.unlink()
        return self.run_ps(shell, script, args, prelude=self.FAKE_WEB, FIXTURES=str(self.tmp / "fixtures"),
                           WEB_LOG=str(log), **env)

    def urls(self) -> list[str]:
        log = self.tmp / "web.log"
        return log.read_text().splitlines() if log.exists() else []

    def fixtures(self, name: str) -> None:
        folder = self.tmp / "fixtures"
        folder.mkdir(exist_ok=True)
        (folder / name).write_bytes(FAKE_CMD.encode())
        write_sums(folder, folder / name)

    def stamped(self, **over: str) -> Path:
        script = self.tmp / "install.ps1"
        script.write_text(stamp.stamp(PS1.read_text(encoding="utf-8"), ".ps1", values(**over)), encoding="utf-8")
        return script

    def test_the_stamped_release_downloads_its_own_version(self) -> None:
        self.fixtures("demo-1.2.3-setup.cmd")
        script = self.stamped(channel="v1.2.3", kind="nsis", program="", release_asset="demo-{version}-setup.cmd")
        for shell in self.each_shell():
            res = self.download(shell, script)
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            base = "https://github.com/Owner/app-repo/releases/download/v1.2.3/"
            self.assertEqual(self.urls(), [base + "demo-1.2.3-setup.cmd", base + "SHA256SUMS"])
            self.assertEqual(self.out_text(), "/S")

    def test_a_token_follows_the_redirect_of_the_api_in_every_powershell(self) -> None:
        # A private repository: each asset comes through the API, which answers 302 to the storage
        # host. Windows PowerShell 5.1 throws for `-MaximumRedirection 0` without handing over the
        # response (found 2026-10-27, installing a private release with GH_TOKEN); PowerShell 7 does
        # not. The token goes to the API and never to the storage address.
        self.fixtures("demo-1.2.3-setup.cmd")
        folder = self.tmp / "fixtures"
        files = {p.name: p.read_bytes() for p in folder.iterdir()}
        server = FakeGitHub("secret-token", "Owner/app-repo", "v1.2.3", files)
        self.addCleanup(server.stop)
        script = self.stamped(channel="v1.2.3", kind="nsis", program="", release_asset="demo-{version}-setup.cmd")
        for shell in self.each_shell():
            server.requests.clear()
            res = self.run_ps(shell, script, f"-ApiUrl {server.base}", GH_TOKEN="secret-token")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertEqual(self.out_text(), "/S")
            storage = [auth for path, auth in server.requests if path.startswith("/storage/")]
            self.assertEqual(len(storage), 2, server.requests)  # the setup and SHA256SUMS
            self.assertEqual(storage, [None, None], "the token went to the storage address")
            api = [auth for path, auth in server.requests if not path.startswith("/storage/")]
            self.assertTrue(api and all(a == "Bearer secret-token" for a in api), server.requests)

    def test_a_token_reads_a_file_the_api_sends_without_a_redirect(self) -> None:
        self.fixtures("demo-1.2.3-setup.cmd")
        files = {p.name: p.read_bytes() for p in (self.tmp / "fixtures").iterdir()}
        server = FakeGitHub("secret-token", "Owner/app-repo", "v1.2.3", files, redirect=False)
        self.addCleanup(server.stop)
        script = self.stamped(channel="v1.2.3", kind="nsis", program="", release_asset="demo-{version}-setup.cmd")
        for shell in self.each_shell():
            server.requests.clear()
            res = self.run_ps(shell, script, f"-ApiUrl {server.base}", GH_TOKEN="secret-token")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertEqual(self.out_text(), "/S")
            self.assertFalse([p for p, _ in server.requests if p.startswith("/storage/")], server.requests)

    def test_a_wrong_token_installs_nothing(self) -> None:
        self.fixtures("demo-1.2.3-setup.cmd")
        files = {p.name: p.read_bytes() for p in (self.tmp / "fixtures").iterdir()}
        server = FakeGitHub("right-token", "Owner/app-repo", "v1.2.3", files)
        self.addCleanup(server.stop)
        script = self.stamped(channel="v1.2.3", kind="nsis", program="", release_asset="demo-{version}-setup.cmd")
        for shell in self.each_shell():
            res = self.run_ps(shell, script, f"-ApiUrl {server.base}", GH_TOKEN="wrong-token")
            self.assertNotEqual(res.returncode, 0)
            self.assertEqual(self.out_text(), "")

    def test_the_api_address_is_https_or_local(self) -> None:
        # The token goes there: never plain http to another host.
        for shell in self.each_shell():
            for bad in ("http://example.com", "ftp://127.0.0.1", "http://localhost.evil.test"):
                res = self.run_ps(shell, PS1, f"-Repo Owner/app-repo -Kind nsis -ApiUrl {bad}", GH_TOKEN="t")
                self.assertNotEqual(res.returncode, 0, bad)
                self.assertIn("-ApiUrl must be an https address", res.stdout + res.stderr, bad)

    def test_a_rolling_build_named_by_its_system(self) -> None:
        self.fixtures("demo-x86_64-pc-windows-msvc-setup.cmd")
        script = self.stamped(kind="nsis", program="", rolling_asset="demo-{triple}-setup.cmd")
        for shell in self.each_shell():
            res = self.download(shell, script)
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertIn("/download/latest/demo-x86_64-pc-windows-msvc-setup.cmd", self.urls()[0])

    def test_no_release_at_all_takes_the_rolling_build(self) -> None:
        self.fixtures("demo-setup.cmd")
        args = "-Repo Owner/app-repo -Kind nsis -RollingAsset demo-setup.cmd"
        for shell in self.each_shell():
            res = self.download(shell, PS1, args, FAKE_NEWEST="404")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertIn("/download/latest/demo-setup.cmd", self.urls()[1])

    def test_an_api_failure_is_not_the_rolling_build(self) -> None:
        self.fixtures("demo-setup.cmd")
        args = "-Repo Owner/app-repo -Kind nsis -RollingAsset demo-setup.cmd"
        for shell in self.each_shell():
            res = self.download(shell, PS1, args, FAKE_NEWEST="403")
            self.assertNotEqual(res.returncode, 0)
            self.assertEqual(len(self.urls()), 1, "nothing is downloaded")
            self.assertFalse(self.out.exists())

    def test_no_channel_takes_the_newest_release(self) -> None:
        self.fixtures("demo-9.9.9-setup.cmd")
        args = "-Repo Owner/app-repo -Kind nsis -ReleaseAsset 'demo-{version}-setup.cmd'"
        for shell in self.each_shell():
            res = self.download(shell, PS1, args, FAKE_NEWEST="200", FAKE_TAG="v9.9.9")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertIn("/download/v9.9.9/demo-9.9.9-setup.cmd", self.urls()[1])

    def test_an_unstamped_copy_wants_its_kind(self) -> None:
        setup = self.fake("demo-setup.cmd")
        write_sums(self.tmp, setup)
        for shell in self.each_shell():
            res = self.run_ps(shell, PS1, f"-File '{setup}'")
            self.assertNotEqual(res.returncode, 0)
            self.assertIn("not stamped", res.stdout + res.stderr)

    def test_a_stamped_copy_needs_no_options(self) -> None:
        stamped = self.tmp / "install.ps1"
        stamped.write_text(stamp.stamp(PS1.read_text(encoding="utf-8"), ".ps1", values(
            kind="inno", program="", rolling_asset="demo-setup.exe")), encoding="utf-8")
        setup = self.fake("demo-setup.cmd")
        write_sums(self.tmp, setup)
        for shell in self.each_shell():
            res = self.run_ps(shell, stamped, f"-File '{setup}'")
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
            self.assertIn("/VERYSILENT", self.out_text())

    def test_invoke_expression_leaves_the_session_alone(self) -> None:
        # Through `irm ... | iex` the script runs in the caller's session: its strict mode and error
        # preference must stay inside, and a failure must not end the session.
        for shell in self.each_shell():
            command = (
                "$File = 'mine'; $Version = 'mine'; $Kind = 'mine'; "
                f"try {{ Get-Content -Raw '{PS1}' | Invoke-Expression }} catch {{ 'caught: ' + $_ }}; "
                "'after: ' + $ErrorActionPreference + ' ' + $File + ' ' + $Version + ' ' + $Kind"
            )
            res = subprocess.run([shell, "-NoProfile", "-NonInteractive", "-Command", command],
                                 capture_output=True, text=True)
            self.assertIn("caught: install.ps1:", res.stdout, res.stderr)
            self.assertIn("after: Continue mine mine mine", res.stdout)


FAKE_GH = r"""#!/usr/bin/env bash
printf '%s\n' "gh $*" >> "$GH_LOG"
if [ "$1" = "api" ] && [ -n "${GH_FAIL_PATCH:-}" ]; then
  echo "refusing to allow a GitHub App to create or update workflow" >&2; exit 1
fi
case "$1 $2" in
  "release create") : > "$GH_LOG.created" ;;
  "release view")
    # No release until one is made.
    if [ -n "${GH_NO_RELEASE:-}" ] && [ ! -e "$GH_LOG.created" ]; then exit 1; fi
    case " $* " in
      *" isDraft "*) printf '%s\n' "${GH_DRAFT:-true}" ;;
      *" assets "*) printf '%s\n' ${GH_ASSETS:-} ;;
    esac
    ;;
esac
exit 0
"""


class VersionJsonTest(unittest.TestCase):
    SHA = "b" * 40
    DATE = "2026-11-01T12:00:00Z"

    def test_the_rolling_build_says_what_it_is_told(self) -> None:
        doc = version_json.make("demo", "latest", "1.2.3", self.SHA, self.DATE)
        self.assertEqual(
            doc,
            {"schema": "release-kit-version/1", "app": "demo", "channel": "latest", "version": "1.2.3",
             "commit": self.SHA, "date": self.DATE},
        )

    def test_a_rolling_build_not_told_its_version_says_null(self) -> None:
        doc = version_json.make("demo", "latest", "", self.SHA, self.DATE)
        self.assertIsNone(doc["version"])

    def test_a_release_has_the_version_of_its_tag(self) -> None:
        doc = version_json.make("demo", "v1.2.3", "", "", self.DATE)
        self.assertEqual((doc["version"], doc["commit"]), ("1.2.3", None))
        self.assertEqual(version_json.make("demo", "v1.2.3", "1.2.3", "", self.DATE)["version"], "1.2.3")

    def test_a_release_refuses_another_version_than_its_tag(self) -> None:
        with self.assertRaises(version_json.VersionError):
            version_json.make("demo", "v1.2.3", "1.2.4")

    def test_what_a_field_may_not_hold_is_refused(self) -> None:
        for args in (
            ("de mo", "latest"),
            ("demo", "main"),
            ("demo", "latest", "one"),
            ("demo", "latest", "1.2.3", "abc"),
            ("demo", "latest", "1.2.3", self.SHA, "yesterday"),
        ):
            with self.subTest(args=args), self.assertRaises(version_json.VersionError):
                version_json.make(*args)

    def test_the_date_defaults_to_now_in_utc(self) -> None:
        self.assertRegex(version_json.make("demo", "latest")["date"], version_json.DATE.pattern)

    def test_cli_writes_json_and_writes_nothing_on_refusal(self) -> None:
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, tmp, ignore_errors=True)
        out = tmp / "version.json"
        self.assertEqual(version_json.main([str(out), "--app", "demo", "--channel", "latest", "--version", "1.0.0"]), 0)
        self.assertEqual(json.loads(out.read_text(encoding="utf-8"))["version"], "1.0.0")
        out.unlink()
        self.assertEqual(version_json.main([str(out), "--app", "demo", "--channel", "nope"]), 1)
        self.assertFalse(out.exists())


@unittest.skipUnless(BASH_EXE and shutil.which("python3" if not WINDOWS else "python"), "no bash")
class PublishTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        gh = self.bin / "gh"
        gh.write_text(FAKE_GH, encoding="utf-8", newline="\n")
        gh.chmod(0o755)
        self.log = self.tmp / "gh.log"
        self.dist = self.tmp / "dist"
        self.dist.mkdir()
        (self.dist / "demo-hub-windows-x86_64.exe").write_bytes(b"windows")
        (self.dist / "demo-hub-linux-x86_64.tar.gz").write_bytes(b"linux")

    def publish(self, **env: str) -> subprocess.CompletedProcess[str]:
        full = dict(
            os.environ,
            PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            GH_LOG=self.log.as_posix(),
            GITHUB_REPOSITORY="Owner/demo",
            KIT=KIT.as_posix(),
            FILES=self.dist.as_posix(),
            EXPECT="2",
            APP="demo",
            SHA="a" * 40,
            TAG="latest",
            WINDOWS_KIND="hub",
            WINDOWS_PROGRAM="demo-hub.exe",
            WINDOWS_RELEASE_ASSET="demo-hub-{version}-{triple}.zip",
            WINDOWS_ROLLING_ASSET="demo-hub-windows-x86_64.exe",
            UNIX_KIND="hub",
            UNIX_PROGRAM="demo-hub",
            UNIX_RELEASE_ASSET="demo-hub-{version}-{triple}.tar.gz",
            UNIX_ROLLING_ASSET="demo-hub-linux-x86_64.tar.gz",
            PYTHON=sys.executable.replace("\\", "/"),
        )
        full.update(env)
        if WINDOWS:
            # Git Bash finds the fake gh by its POSIX path.
            full["PATH"] = "/" + self.bin.as_posix().replace(":", "", 1) + ":" + "/usr/bin:/bin:" + os.environ["PATH"]
        return subprocess.run([BASH_EXE, PUBLISH.as_posix()], capture_output=True, text=True, env=full)

    def calls(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_rolling_order_sums_first_out_and_words_last(self) -> None:
        res = self.publish(GH_ASSETS="SHA256SUMS demo-hub-windows-x86_64.exe old-name.exe")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        calls = self.calls()
        kinds = [c.split(" --repo")[0] for c in calls]
        first_delete = kinds.index("gh release delete-asset latest SHA256SUMS")
        upload = next(i for i, c in enumerate(kinds) if c.startswith("gh release upload latest demo-hub"))
        stale = kinds.index("gh release delete-asset latest old-name.exe")
        sums = kinds.index("gh release upload latest SHA256SUMS")
        tag = next(i for i, c in enumerate(calls) if "git/refs/tags/latest" in c)
        edit = next(i for i, c in enumerate(kinds) if c.startswith("gh release edit latest"))
        self.assertLess(first_delete, upload)
        self.assertLess(upload, stale)
        self.assertLess(stale, sums)
        self.assertLess(sums, tag)
        self.assertLess(tag, edit)
        self.assertIn("Rolling build (aaaaaaa)", calls[edit])
        self.assertIn("install.ps1", calls[upload])
        self.assertIn("install.sh", calls[upload])
        self.assertIn("SHA256SUMS", res.stdout + "".join(calls))
        self.assertIn("install.ps1", res.stdout)

    def test_the_first_rolling_build_makes_the_pre_release(self) -> None:
        res = self.publish(GH_NO_RELEASE="1")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        create = next(c for c in self.calls() if c.startswith("gh release create latest"))
        for flag in ("--prerelease", "--latest=false", "--target " + "a" * 40):
            self.assertIn(flag, create)

    def test_a_tag_that_cannot_move_still_names_the_commit(self) -> None:
        res = self.publish(GH_FAIL_PATCH="1")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("::warning::", res.stderr)
        self.assertTrue(any(c.startswith("gh release edit latest") and "aaaaaaa" in c for c in self.calls()))

    def test_a_missing_system_publishes_nothing(self) -> None:
        (self.dist / "demo-hub-linux-x86_64.tar.gz").unlink()
        res = self.publish()
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("instead of 2", res.stderr)
        self.assertEqual(self.calls(), [])

    def test_release_makes_a_draft_and_attaches(self) -> None:
        res = self.publish(TAG="v1.2.3", SHA="", GH_NO_RELEASE="1")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        calls = self.calls()
        self.assertTrue(any(c.startswith("gh release create v1.2.3") and "--draft" in c for c in calls), calls)
        upload = next(c for c in calls if c.startswith("gh release upload v1.2.3"))
        for name in ("install.ps1", "install.sh", "SHA256SUMS", "demo-hub-windows-x86_64.exe"):
            self.assertIn(name, upload)
        self.assertFalse(any("git/refs/tags" in c for c in calls), "a release tag is never moved")

    def test_a_windows_only_release_has_no_install_sh(self) -> None:
        # An app built for Windows only (the desktop apps on the author's runner): no unix inputs, one file.
        (self.dist / "demo-hub-linux-x86_64.tar.gz").unlink()
        res = self.publish(TAG="v1.2.3", SHA="", GH_NO_RELEASE="1", EXPECT="1", UNIX_KIND="", UNIX_PROGRAM="",
                           UNIX_RELEASE_ASSET="", UNIX_ROLLING_ASSET="")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        upload = next(c for c in self.calls() if c.startswith("gh release upload v1.2.3"))
        self.assertIn("install.ps1", upload)
        self.assertNotIn("install.sh", upload)

    def test_a_draft_of_an_earlier_cut_gets_the_words_of_this_one(self) -> None:
        notes = self.tmp / "notes.md"
        notes.write_text("the words of this cut\n")
        res = self.publish(TAG="v1.2.3", SHA="", GH_DRAFT="true", NOTES_FILE=notes.as_posix())
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        calls = self.calls()
        self.assertFalse(any(c.startswith("gh release create") for c in calls), "the draft is there")
        self.assertTrue(any(c.startswith("gh release edit v1.2.3") and "--notes-file" in c for c in calls), calls)

    def test_the_title_and_the_tag_in_the_notes(self) -> None:
        notes = self.tmp / "notes.md"
        notes.write_text("install: .../download/{tag}/install.sh\n")
        sent = self.tmp / "sent-notes.md"
        gh = self.bin / "gh"
        copy = f'if [ "$1 $2" = "release create" ]; then for a in "$@"; do [ -f "$a" ] && cp "$a" "{sent.as_posix()}"; done; fi\n'
        self.assertTrue(FAKE_GH.endswith("exit 0\n"))
        gh.write_text(FAKE_GH[: -len("exit 0\n")] + copy + "exit 0\n", encoding="utf-8", newline="\n")
        res = self.publish(TAG="v1.2.3", SHA="", GH_NO_RELEASE="1", NOTES_FILE=notes.as_posix(),
                           TITLE="demo v1.2.3")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        create = next(c for c in self.calls() if c.startswith("gh release create v1.2.3"))
        self.assertIn('--title demo v1.2.3', create)
        self.assertEqual(sent.read_text().strip(), "install: .../download/v1.2.3/install.sh")

    def test_the_title_defaults_to_the_tag(self) -> None:
        res = self.publish(TAG="v1.2.3", SHA="", GH_NO_RELEASE="1")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        create = next(c for c in self.calls() if c.startswith("gh release create v1.2.3"))
        self.assertIn("--title v1.2.3", create)

    def test_a_title_of_two_lines_is_refused(self) -> None:
        res = self.publish(TAG="v1.2.3", SHA="", GH_NO_RELEASE="1", TITLE="one\ntwo")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("title must be one line", res.stderr)
        self.assertEqual(self.calls(), [])

    def test_the_title_of_a_draft_that_is_there_follows(self) -> None:
        res = self.publish(TAG="v1.2.3", SHA="", GH_DRAFT="true", TITLE="demo v1.2.3")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        edit = next(c for c in self.calls() if c.startswith("gh release edit v1.2.3"))
        self.assertIn("--title demo v1.2.3", edit)

    def test_a_published_release_is_not_touched(self) -> None:
        res = self.publish(TAG="v1.2.3", SHA="", GH_DRAFT="false")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("NREL-10", res.stderr)
        self.assertFalse(any("upload" in c for c in self.calls()))

    def test_an_extra_file_may_not_shadow_a_script(self) -> None:
        extra = self.tmp / "install.ps1"
        extra.write_text("x")
        res = self.publish(EXTRA=extra.as_posix())
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("two files are called install.ps1", res.stderr)
        self.assertEqual(self.calls(), [])

    def test_the_stamped_scripts_carry_the_channel(self) -> None:
        # A fake gh that keeps a copy of what is uploaded.
        keep = self.tmp / "kept"
        keep.mkdir()
        gh = self.bin / "gh"
        copy = f'if [ "$1 $2" = "release upload" ]; then for f in "$@"; do [ -f "$f" ] && cp "$f" "{keep.as_posix()}/"; done; fi\n'
        self.assertTrue(FAKE_GH.endswith("exit 0\n"))
        gh.write_text(FAKE_GH[: -len("exit 0\n")] + copy + "exit 0\n", encoding="utf-8", newline="\n")
        res = self.publish(TAG="v2.0.0", SHA="", GH_NO_RELEASE="1")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("$StampChannel = 'v2.0.0'", (keep / "install.ps1").read_text(encoding="utf-8"))
        self.assertIn('STAMP_CHANNEL="v2.0.0"', (keep / "install.sh").read_text(encoding="utf-8"))
        sums = (keep / "SHA256SUMS").read_text()
        for name in ("install.ps1", "install.sh", "demo-hub-windows-x86_64.exe", "demo-hub-linux-x86_64.tar.gz"):
            self.assertIn(f"  {name}", sums)
        self.assertEqual(
            sums.split()[sums.split().index("install.ps1") - 1], sha(keep / "install.ps1"),
            "the checksum is of the stamped script",
        )

    def keep_uploads(self) -> Path:
        """A fake gh that keeps a copy of every file it is asked to upload."""
        keep = self.tmp / "kept"
        keep.mkdir()
        gh = self.bin / "gh"
        copy = f'if [ "$1 $2" = "release upload" ]; then for f in "$@"; do [ -f "$f" ] && cp "$f" "{keep.as_posix()}/"; done; fi\n'
        gh.write_text(FAKE_GH[: -len("exit 0\n")] + copy + "exit 0\n", encoding="utf-8", newline="\n")
        return keep

    def test_the_rolling_build_carries_version_json_listed_in_the_sums(self) -> None:
        keep = self.keep_uploads()
        res = self.publish(VERSION="1.4.0", GH_ASSETS="SHA256SUMS version.json")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        doc = json.loads((keep / "version.json").read_text(encoding="utf-8"))
        self.assertEqual((doc["app"], doc["channel"], doc["version"], doc["commit"]), ("demo", "latest", "1.4.0", "a" * 40))
        sums = (keep / "SHA256SUMS").read_text()
        self.assertEqual(sums.split()[sums.split().index("version.json") - 1], sha(keep / "version.json"))
        first = next(c for c in self.calls() if c.startswith("gh release upload latest demo-hub"))
        self.assertIn("version.json", first)
        self.assertFalse(any("delete-asset latest version.json" in c for c in self.calls()), "this build has it")

    def test_a_release_carries_the_version_of_its_tag(self) -> None:
        keep = self.keep_uploads()
        res = self.publish(TAG="v2.0.1", SHA="", GH_NO_RELEASE="1")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        doc = json.loads((keep / "version.json").read_text(encoding="utf-8"))
        self.assertEqual((doc["channel"], doc["version"], doc["commit"]), ("v2.0.1", "2.0.1", None))

    def test_a_release_with_another_version_publishes_nothing(self) -> None:
        res = self.publish(TAG="v2.0.1", SHA="", VERSION="2.0.2", GH_NO_RELEASE="1")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("not the version of the tag", res.stderr)
        self.assertEqual(self.calls(), [])

    def test_a_file_called_version_json_may_not_be_given(self) -> None:
        (self.dist / "version.json").write_text("{}")
        res = self.publish(EXPECT="3")
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("two files are called version.json", res.stderr)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
