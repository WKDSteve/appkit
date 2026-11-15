//! `install.ps1` of the release kit with a hub that is a window program (`window-probe`), in PowerShell 7 and in
//! Windows PowerShell 5.1: a shell does not wait for a window program unless its output is piped, so the script must
//! pipe every call of the hub whose end or exit code it needs (`--version`, `install`). Before that, `$x = (& $exe
//! --version)` closed the pipe while the program still ran: it died writing, and `$LASTEXITCODE` was not set.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use appkit_scratch::Scratch;

const WINDOW_PROBE: &str = env!("CARGO_BIN_EXE_window-probe");

fn install_ps1() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release-kit/scripts/install.ps1")
}

/// PowerShell 7 when it is on `PATH`, and Windows PowerShell 5.1 at its fixed place.
fn shells() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let path = std::env::var_os("PATH").unwrap_or_default();
    if let Some(pwsh) = std::env::split_paths(&path)
        .map(|d| d.join("pwsh.exe"))
        .find(|f| f.is_file())
    {
        found.push(pwsh);
    }
    let root = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    found.push(root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    found
}

/// Run `install.ps1 -Kind hub` on a copy of the window program, with its output captured as a build machine does.
fn install(shell: &Path, dir: &Path, exit: &str) -> (Output, String) {
    let program = dir.join("demo-hub.exe");
    std::fs::copy(WINDOW_PROBE, &program).unwrap();
    let installed = dir.join("installed.txt");
    let _ = std::fs::remove_file(&installed);
    let script = install_ps1();
    let command = format!(
        "$h = (Get-FileHash -LiteralPath '{p}' -Algorithm SHA256).Hash; & '{s}' -File '{p}' -Sha256 $h -Kind hub -Program demo-hub.exe -App demo -AllowElevated",
        p = program.display(),
        s = script.display()
    );
    let out = Command::new(shell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ])
        .env("INSTALL_GH", "no-such-gh-for-the-tests")
        .env("WINDOW_PROBE_INSTALLED", &installed)
        .env("WINDOW_PROBE_EXIT", exit)
        .env("TEMP", dir)
        .env("TMP", dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out, said)
}

#[test]
fn a_hub_that_is_a_window_program_is_waited_for_and_its_exit_code_counts() {
    for shell in shells() {
        let dir = Scratch::new("install-ps1-window-program");
        let (out, said) = install(&shell, dir.path(), "0");
        assert!(out.status.success(), "{}: {said}", shell.display());
        assert!(
            said.contains("installing window-probe 1.0"),
            "{}: {said}",
            shell.display()
        );
        assert!(
            said.contains("installed window-probe 1.0"),
            "{}: {said}",
            shell.display()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("installed.txt"))
                .ok()
                .as_deref(),
            Some("install "),
            "{}: {said}",
            shell.display()
        );

        // A failed install is the script's failure, after the program has ended.
        let (out, said) = install(&shell, dir.path(), "4");
        assert!(!out.status.success(), "{}: {said}", shell.display());
        assert!(
            said.contains("install failed"),
            "{}: {said}",
            shell.display()
        );
        assert!(
            dir.join("installed.txt").is_file(),
            "{}: {said}",
            shell.display()
        );
    }
}
