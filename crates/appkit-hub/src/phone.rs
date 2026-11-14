//! The PC side of a phone, for the command line of an app's hub (`<program> phone ...`): the pairing link of a phone and
//! its QR code, the list of devices, revoking one and setting its level. Everything goes to the running hub of the app
//! with its control secret, which only the person on this machine can read; nothing here reaches anything but
//! `127.0.0.1`.
//!
//! The pairing link is the form `appkit-mobile` reads (`way::parse_pair_link`): the address the phone uses (the tailnet
//! name, or the loopback port at the end of an SSH tunnel), the app, and a one-time code in the fragment (`MSEC-02`).

use std::path::{Path, PathBuf};

use appkit_core::identity::AppIdentity;
use appkit_core::registry;
use serde_json::Value;

use crate::client;

/// The way the phone will use to reach the hub, for the address in the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneWay {
    /// Tailscale Serve on this name (`pc.example.ts.net`).
    Tailnet(String),
    /// An SSH tunnel that ends on the phone's `127.0.0.1` at the hub's own port.
    Tunnel,
}

/// The pairing link: `<base>/#appkit-pair=1&app=<namespace>&code=<code>`.
#[must_use]
pub fn pair_link(way: &PhoneWay, port: u16, namespace: &str, code: &str) -> String {
    let base = match way {
        PhoneWay::Tailnet(name) => format!("https://{name}"),
        PhoneWay::Tunnel => format!("http://127.0.0.1:{port}"),
    };
    format!("{base}/#appkit-pair=1&app={namespace}&code={code}")
}

/// How long a pairing code is good, in seconds (the hub's rule; the page of a code hides it after that).
pub const PAIR_SECONDS: u64 = 60;

/// The name of the page of a pairing code in the settings folder of the app ([`show_qr`]).
pub const QR_FILE: &str = "pair-qr.html";

fn qr_code(text: &str) -> Result<qrcode::QrCode, String> {
    qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("no QR code: {e}"))
}

/// A QR code of `text` as plain text: two rows of modules per line of half blocks, in the colours of whatever shows
/// it. On a dark terminal that is light on dark (inverted), which not every scanner reads: for a terminal take
/// [`qr_terminal`].
///
/// # Errors
///
/// The text is too long for a QR code.
pub fn qr_text(text: &str) -> Result<String, String> {
    use qrcode::render::unicode::Dense1x2;
    Ok(qr_code(text)?
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .quiet_zone(true)
        .build())
}

/// Black on white, whatever the theme of the terminal: the plain pair first, then the exact colours for a terminal
/// that knows them (one that does not keeps the plain pair).
const QR_COLOURS: &str = "\x1b[30;47m\x1b[38;2;0;0;0m\x1b[48;2;255;255;255m";
const QR_RESET: &str = "\x1b[0m";

/// [`qr_text`] with fixed colours: every line black on white by escape sequences, so the code is dark on light on
/// any theme and every scanner reads it (the camera app of a phone does not read an inverted code).
///
/// # Errors
///
/// The text is too long for a QR code.
pub fn qr_coloured(text: &str) -> Result<String, String> {
    let plain = qr_text(text)?;
    let mut out = String::with_capacity(plain.len() + plain.lines().count() * 48);
    for line in plain.lines() {
        out.push_str(QR_COLOURS);
        out.push_str(line);
        out.push_str(QR_RESET);
        out.push('\n');
    }
    Ok(out)
}

/// The QR code of `text` for the standard output of a command: [`qr_coloured`] when that is a terminal that takes
/// escape sequences, else [`qr_text`] (a file or a pipe gets no escape sequences).
///
/// # Errors
///
/// The text is too long for a QR code.
pub fn qr_terminal(text: &str) -> Result<String, String> {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() && terminal_takes_colours() {
        qr_coloured(text)
    } else {
        qr_text(text).map(|mut qr| {
            qr.push('\n');
            qr
        })
    }
}

/// Does the terminal of the standard output take escape sequences? On Windows a console has to be told so
/// (`ENABLE_VIRTUAL_TERMINAL_PROCESSING`); elsewhere every terminal but `TERM=dumb` does.
#[cfg(windows)]
#[allow(unsafe_code)]
fn terminal_takes_colours() -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        STD_OUTPUT_HANDLE,
    };
    // SAFETY: the calls take a handle of this process and a pointer to a local that lives for the call; a handle that
    // is no console makes `GetConsoleMode` fail, which is answered with `false`.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &raw mut mode) == 0 {
            return false;
        }
        mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
            || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn terminal_takes_colours() -> bool {
    std::env::var_os("TERM").is_some_and(|term| term != "dumb")
}

fn html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A page that shows the QR code of `text` as a picture: black on white with its quiet zone, as large as the window
/// allows, the text below it and `caption` above. No script and nothing from the network; the code is hidden after
/// [`PAIR_SECONDS`], as long as a pairing code is good.
///
/// # Errors
///
/// The text is too long for a QR code.
pub fn qr_page(text: &str, caption: &str) -> Result<String, String> {
    const QUIET: usize = 4;
    let code = qr_code(text)?;
    let width = code.width();
    let side = width + 2 * QUIET;
    let mut path = String::new();
    for (i, colour) in code.to_colors().into_iter().enumerate() {
        if colour == qrcode::Color::Dark {
            let (x, y) = (i % width + QUIET, i / width + QUIET);
            path.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    let caption = html_text(caption);
    let text = html_text(text);
    Ok(format!(
        r##"<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{caption}</title>
<style>
  html {{ background: #fff; color: #000; font: 16px/1.4 system-ui, sans-serif; }}
  body {{ margin: 0; min-height: 100vh; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 16px; box-sizing: border-box; }}
  svg {{ width: min(80vh, 92vw); height: auto; }}
  code {{ font: 13px/1.4 ui-monospace, monospace; overflow-wrap: anywhere; text-align: center; }}
  .code {{ display: flex; flex-direction: column; align-items: center; gap: 12px; animation: gone 0s {PAIR_SECONDS}s forwards; }}
  .over {{ visibility: hidden; position: absolute; animation: come 0s {PAIR_SECONDS}s forwards; }}
  @keyframes gone {{ to {{ visibility: hidden; }} }}
  @keyframes come {{ to {{ visibility: visible; }} }}
</style>
<h1>{caption}</h1>
<div class="code">
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {side}" shape-rendering="crispEdges" role="img" aria-label="QR code">
<rect width="{side}" height="{side}" fill="#fff"/>
<path fill="#000" d="{path}"/>
</svg>
<code>{text}</code>
</div>
<p class="over">This code is over. Ask for a new one.</p>
</html>
"##
    ))
}

/// Write the page of the QR code of `text` ([`qr_page`]) to the settings folder `dir` of the app, as [`QR_FILE`], for
/// the person only, and open it with the program the system shows a page with. The file holds a one-time code:
/// [`forget_qr`] removes it; the next code overwrites it.
///
/// # Errors
///
/// The text is too long for a QR code, the file cannot be written, or the system could not open it (the file is
/// removed again then).
pub fn show_qr(dir: &Path, text: &str, caption: &str) -> Result<PathBuf, String> {
    let page = qr_page(text, caption)?;
    let path = dir.join(QR_FILE);
    write_private(&path, page.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    if let Err(e) = crate::browser::open_own_page(&path) {
        forget_qr(dir);
        return Err(e);
    }
    Ok(path)
}

/// What a `phone pair` command shows on its standard output: the QR code of `link` ([`qr_terminal`]), the link and
/// `hint` (what to do with them). With `window`, and only when the output is a terminal (a test or a script opens
/// nothing), the code is also opened as a picture ([`show_qr`]); the command then waits until the code is over
/// ([`PAIR_SECONDS`]) and removes the page. A window that cannot be opened is said and is no error: the terminal has
/// the code.
///
/// # Errors
///
/// The link is too long for a QR code, or the standard output cannot be written.
pub async fn present_pairing(
    dir: &Path,
    link: &str,
    caption: &str,
    hint: &str,
    window: bool,
) -> Result<(), String> {
    use std::io::{IsTerminal, Write};
    let qr = qr_terminal(link)?;
    let shown = (window && std::io::stdout().is_terminal()).then(|| show_qr(dir, link, caption));
    let note = match &shown {
        Some(Ok(_)) => format!(
            "The code is also open as a picture in a window. This command ends when the code is over ({PAIR_SECONDS} s).
"
        ),
        Some(Err(e)) => format!("No window with the code: {e}
"),
        None => String::new(),
    };
    let text = format!(
        "{qr}
{link}

{hint}
{note}"
    );
    // In a block of its own: the lock of the standard output must not live across the wait below, or the future could
    // not be handed to another thread (`tokio::spawn`).
    {
        let mut out = std::io::stdout().lock();
        out.write_all(text.as_bytes())
            .and_then(|()| out.flush())
            .map_err(|e| format!("cannot write the pairing code: {e}"))?;
    }
    if matches!(shown, Some(Ok(_))) {
        tokio::time::sleep(std::time::Duration::from_secs(PAIR_SECONDS)).await;
        forget_qr(dir);
    }
    Ok(())
}

/// Remove the page of a pairing code from the settings folder `dir` ([`show_qr`]); nothing happens when there is none.
pub fn forget_qr(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(QR_FILE));
}

/// Write `bytes` to `path` anew (never through a link that lies there), readable by the person only.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The running hub of the app and its control secret, once it said it is the hub `hub.json` names.
async fn control(dir: &Path, identity: &AppIdentity) -> Result<(u16, String), String> {
    let record = crate::instance::running_hub(dir, identity)
        .await
        .ok_or_else(|| format!("no hub of {} is running: start it first", identity.app))?;
    let secret = registry::read_secret(dir)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no control secret: the hub is not running".to_owned())?;
    Ok((record.port, secret))
}

async fn post(port: u16, secret: &str, path: &str, body: &str) -> Result<Value, String> {
    let auth = format!("Bearer {secret}");
    let reply = client::request(
        port,
        "POST",
        path,
        &[
            ("Authorization", auth.as_str()),
            ("Content-Type", "application/json"),
        ],
        body,
    )
    .await
    .map_err(|e| format!("the hub does not answer: {e}"))?;
    let doc: Value = serde_json::from_str(&reply.body).unwrap_or(Value::Null);
    if reply.status != 200 {
        let code = doc["error"]["code"].as_str().unwrap_or("http_error");
        return Err(format!("the hub refused ({}): {code}", reply.status));
    }
    Ok(doc)
}

/// A one-time pairing link for a phone, from the running hub.
///
/// # Errors
///
/// No hub is running, it is not the one of `hub.json`, it keeps no devices, or it refused.
pub async fn pairing_link(
    dir: &Path,
    identity: &AppIdentity,
    way: &PhoneWay,
) -> Result<String, String> {
    let (port, secret) = control(dir, identity).await?;
    let doc = post(port, &secret, "/api/control/pair", "{}").await?;
    let code = doc["code"]
        .as_str()
        .filter(|c| c.len() == 32 && c.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("the hub gave no code")?;
    Ok(pair_link(way, port, identity.route_ns, code))
}

/// The paired devices of the running hub, one line each: id, name, level, whether it can confirm a trigger, when it
/// was last used.
///
/// # Errors
///
/// As [`pairing_link`].
pub async fn device_lines(dir: &Path, identity: &AppIdentity) -> Result<String, String> {
    let (port, secret) = control(dir, identity).await?;
    let doc = post(port, &secret, crate::guard::CONTROL_DEVICES_PATH, "{}").await?;
    let mut out = String::new();
    for d in doc["devices"].as_array().into_iter().flatten() {
        let text = |k: &str| d[k].as_str().unwrap_or("?").to_owned();
        let used = d["last_used"]
            .as_u64()
            .map_or_else(|| "never".to_owned(), |t| t.to_string());
        let confirm = if d["confirm"].as_bool() == Some(true) {
            "confirms"
        } else {
            "no confirmation key"
        };
        out.push_str(&format!(
            "{}  {}  level {}  {confirm}  last used {used}\n",
            text("id"),
            text("name"),
            text("level")
        ));
    }
    if out.is_empty() {
        out.push_str("no device is paired\n");
    }
    Ok(out)
}

/// Revoke a device of the running hub; its sessions end at once.
///
/// # Errors
///
/// As [`pairing_link`], or no such device.
pub async fn revoke(dir: &Path, identity: &AppIdentity, device: &str) -> Result<(), String> {
    let (port, secret) = control(dir, identity).await?;
    let body = serde_json::json!({ "device": device }).to_string();
    let doc = post(port, &secret, crate::guard::CONTROL_REVOKE_PATH, &body).await?;
    if doc["revoked"] == true {
        Ok(())
    } else {
        Err(format!("no device {device}"))
    }
}

/// Set the level of a device of the running hub (`read`, `create`, `change`, `trigger`).
///
/// # Errors
///
/// As [`pairing_link`], an unknown level, or no such device.
pub async fn set_level(
    dir: &Path,
    identity: &AppIdentity,
    device: &str,
    level: &str,
) -> Result<(), String> {
    if crate::devices::Level::parse(level).is_none() {
        return Err(format!(
            "no level {level:?}: read, create, change or trigger"
        ));
    }
    let (port, secret) = control(dir, identity).await?;
    let body = serde_json::json!({ "device": device, "level": level }).to_string();
    let doc = post(port, &secret, crate::guard::CONTROL_LEVEL_PATH, &body).await?;
    if doc["set"] == true {
        Ok(())
    } else {
        Err(format!("no device {device}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn the_link_names_the_way_the_app_and_the_code_in_the_fragment() {
        assert_eq!(
            pair_link(
                &PhoneWay::Tailnet("pc.example.ts.net".to_owned()),
                17618,
                "testing",
                CODE
            ),
            format!("https://pc.example.ts.net/#appkit-pair=1&app=testing&code={CODE}")
        );
        assert_eq!(
            pair_link(&PhoneWay::Tunnel, 17618, "testing", CODE),
            format!("http://127.0.0.1:17618/#appkit-pair=1&app=testing&code={CODE}")
        );
    }

    #[test]
    fn a_link_makes_a_qr_code_of_half_blocks() {
        let qr = qr_text(&pair_link(&PhoneWay::Tunnel, 17618, "testing", CODE)).unwrap();
        let lines: Vec<&str> = qr.lines().collect();
        assert!(lines.len() > 10, "{qr}");
        assert!(lines
            .iter()
            .all(|l| l.chars().count() == lines[0].chars().count()));
        assert!(qr
            .chars()
            .all(|c| matches!(c, ' ' | '\u{2580}' | '\u{2584}' | '\u{2588}' | '\n')));
        assert!(qr_text(&"x".repeat(5000)).is_err());
    }

    #[test]
    fn the_coloured_code_is_the_plain_one_with_black_on_white_on_every_line() {
        let link = pair_link(&PhoneWay::Tunnel, 17618, "testing", CODE);
        let plain = qr_text(&link).unwrap();
        let coloured = qr_coloured(&link).unwrap();
        assert!(coloured.ends_with('\n'));
        let lines: Vec<&str> = coloured.lines().collect();
        assert_eq!(lines.len(), plain.lines().count());
        for (line, plain) in lines.iter().zip(plain.lines()) {
            // Each line sets its colours and gives them back, so nothing of them reaches the text after the code.
            let inner = line
                .strip_prefix("\x1b[30;47m\x1b[38;2;0;0;0m\x1b[48;2;255;255;255m")
                .and_then(|rest| rest.strip_suffix("\x1b[0m"))
                .unwrap_or_else(|| panic!("{line:?}"));
            assert_eq!(inner, plain);
        }
        assert!(qr_coloured(&"x".repeat(5000)).is_err());
    }

    /// The modules of the page's picture, read back from its path: `true` is dark.
    fn modules_of(page: &str) -> (usize, Vec<bool>) {
        let side: usize = page
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        let path = page
            .split("<path fill=\"#000\" d=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        let mut dark = vec![false; side * side];
        for part in path.split('M').skip(1) {
            let (xy, shape) = part.split_once('h').unwrap();
            assert_eq!(shape, "1v1h-1z");
            let (x, y) = xy.split_once(' ').unwrap();
            let (x, y): (usize, usize) = (x.parse().unwrap(), y.parse().unwrap());
            assert!(!dark[y * side + x], "a module drawn twice");
            dark[y * side + x] = true;
        }
        (side, dark)
    }

    #[test]
    fn the_page_draws_exactly_the_modules_of_the_code_inside_a_quiet_zone() {
        let link = pair_link(
            &PhoneWay::Tailnet("pc.example.ts.net:8443".to_owned()),
            17618,
            "testing",
            CODE,
        );
        let page = qr_page(&link, "Pair a phone with testing").unwrap();
        let code = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M)
            .unwrap();
        let width = code.width();
        let (side, dark) = modules_of(&page);
        assert_eq!(side, width + 8);
        let colours = code.to_colors();
        for y in 0..side {
            for x in 0..side {
                let inside = (4..4 + width).contains(&x) && (4..4 + width).contains(&y);
                let want = inside && colours[(y - 4) * width + (x - 4)] == qrcode::Color::Dark;
                assert_eq!(dark[y * side + x], want, "module {x},{y}");
            }
        }
        // White behind the code whatever the browser's theme, and the code is hidden when it is over.
        assert!(page.contains(&format!(
            "<rect width=\"{side}\" height=\"{side}\" fill=\"#fff\"/>"
        )));
        assert!(page.contains("animation: gone 0s 60s forwards"));
        assert!(qr_page(&"x".repeat(5000), "x").is_err());
    }

    #[test]
    fn the_page_runs_nothing_and_shows_the_link_and_the_caption_as_text() {
        let page = qr_page(
            "https://pc.example.ts.net/#a=1&b=<script>alert(1)</script>",
            "Pair \"a\" <b>phone</b> & 'more'",
        )
        .unwrap();
        assert!(page.contains(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">"
        ));
        assert!(!page.contains("<script"));
        assert!(!page.contains("<b>"));
        assert!(page.contains("<code>https://pc.example.ts.net/#a=1&amp;b=&lt;script&gt;alert(1)&lt;/script&gt;</code>"));
        assert!(page
            .contains("<h1>Pair &quot;a&quot; &lt;b&gt;phone&lt;/b&gt; &amp; &#39;more&#39;</h1>"));
    }

    #[test]
    fn the_page_of_a_code_is_written_anew_and_forgotten() {
        let dir = appkit_scratch::Scratch::new("phone-qr");
        let path = dir.path().join(QR_FILE);
        // What lies there from the code before is replaced, not appended to.
        std::fs::write(&path, "x".repeat(100_000)).unwrap();
        write_private(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(crate::browser::is_own_page(&path));
        forget_qr(dir.path());
        assert!(!path.exists());
        assert!(!crate::browser::is_own_page(&path));
        forget_qr(dir.path());
    }

    #[test]
    fn only_the_page_of_a_code_is_opened_as_a_file() {
        let dir = appkit_scratch::Scratch::new("phone-qr-open");
        let other = dir.path().join("other.html");
        std::fs::write(&other, "x").unwrap();
        assert!(!crate::browser::is_own_page(&other));
        assert!(crate::browser::open_own_page(&other).is_err());
        // The right name, but not there, relative, or a folder: not opened.
        assert!(crate::browser::open_own_page(&dir.path().join(QR_FILE)).is_err());
        assert!(crate::browser::open_own_page(Path::new(QR_FILE)).is_err());
        let folder = dir.path().join("sub");
        std::fs::create_dir_all(folder.join(QR_FILE)).unwrap();
        assert!(crate::browser::open_own_page(&folder.join(QR_FILE)).is_err());
    }

    #[tokio::test]
    async fn a_pairing_can_be_shown_from_a_task_of_the_runtime() {
        // An app runs it with `tokio::spawn` (the main thread of Windows has a small stack): the future has to be
        // `Send`, which it was not while it held the lock of the standard output across its wait.
        let dir = appkit_scratch::Scratch::new("phone-present-task");
        let folder = dir.path().to_path_buf();
        let link = pair_link(&PhoneWay::Tunnel, 17618, "testing", CODE);
        tokio::spawn(async move {
            present_pairing(&folder, &link, "Pair a phone", "Scan it.", false).await
        })
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test]
    async fn without_a_terminal_a_pairing_opens_no_window_and_does_not_wait() {
        // The output of a test is no terminal: the code is written, no page is made and nothing waits for the code
        // to end, though a window was asked for.
        let dir = appkit_scratch::Scratch::new("phone-present");
        let link = pair_link(&PhoneWay::Tunnel, 17618, "testing", CODE);
        let started = std::time::Instant::now();
        present_pairing(dir.path(), &link, "Pair a phone", "Scan it.", true)
            .await
            .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(PAIR_SECONDS / 2));
        assert!(!dir.path().join(QR_FILE).exists());
        assert!(
            present_pairing(dir.path(), &"x".repeat(5000), "x", "x", true)
                .await
                .is_err()
        );
    }
}
