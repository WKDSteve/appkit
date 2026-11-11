//! The PC side of a phone, for the command line of an app's hub (`<program> phone ...`): the pairing link of a phone and
//! its QR code, the list of devices, revoking one and setting its level. Everything goes to the running hub of the app
//! with its control secret, which only the person on this machine can read; nothing here reaches anything but
//! `127.0.0.1`.
//!
//! The pairing link is the form `appkit-mobile` reads (`way::parse_pair_link`): the address the phone uses (the tailnet
//! name, or the loopback port at the end of an SSH tunnel), the app, and a one-time code in the fragment (`MSEC-02`).

use std::path::Path;

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

/// A QR code of `text` for a terminal: two rows of modules per line of half blocks, dark on a light quiet zone, so it
/// scans on a dark and on a light terminal.
///
/// # Errors
///
/// The text is too long for a QR code.
pub fn qr_text(text: &str) -> Result<String, String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("no QR code: {e}"))?;
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .quiet_zone(true)
        .build())
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
}
