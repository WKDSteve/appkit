//! The Android side of appkit's mobile basis for a Tauri app (see `docs/MOBILE.md` of the kit). The shell of an app
//! calls it from Rust; the page reaches none of it (the plugin has no command for the page).
//!
//! - [`Platform::public_key`] and [`Platform::sign`]: device keys in the Android Keystore. A key made with `user_auth`
//!   signs only after the biometrics or the device lock, bound by a `CryptoObject` (`AND-05`): the trigger key.
//! - [`Platform::scan_qr`]: the text of a QR code (ZXing).
//! - [`Platform::pick_folder`], [`Platform::list`], [`Platform::read`]: the folder of a vault cloned on the phone,
//!   read only and bounded.
//! - [`Platform::push_register`], [`Platform::push_state`], [`Platform::push_unregister`]: UnifiedPush.
//!
//! Every call blocks until the platform answers (a prompt may wait for the person): call it off the async runtime
//! (`tauri::async_runtime::spawn_blocking`). Refusals are short words ([`Refused`]).

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
#[cfg(target_os = "android")]
use tauri::Manager;
use tauri::Runtime;

/// The plugin; on Android it puts a [`Platform`] into the app's state.
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("appkit")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin("io.github.wkdsteve.appkit", "AppkitPlugin")?;
                app.manage(Platform(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                let _ = (app, api);
            }
            Ok(())
        })
        .build()
}

/// The handle of the Android side, from the app's state (`app.try_state::<Platform<R>>()`); there is none elsewhere.
pub struct Platform<R: Runtime>(
    #[cfg(target_os = "android")] tauri::plugin::PluginHandle<R>,
    #[cfg(not(target_os = "android"))] std::marker::PhantomData<fn() -> R>,
);

/// Why the platform did not do it, as one word: `cancelled` (the person did not confirm), `gone` (the key is not there
/// any more), `no_lock` (a key bound to the person needs a screen lock), `no_folder`, `missing`, `too_large`,
/// `no_distributor` (no UnifiedPush distributor app), or `failed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

const WORDS: [&str; 9] = [
    "cancelled",
    "gone",
    "no_lock",
    "no_folder",
    "missing",
    "too_large",
    "no_distributor",
    "unsupported",
    "failed",
];

impl Refused {
    fn of(word: &str) -> Self {
        Self(
            if WORDS.contains(&word) {
                word
            } else {
                "failed"
            }
            .to_owned(),
        )
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyArgs<'a> {
    alias: &'a str,
    user_auth: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignArgs<'a> {
    alias: &'a str,
    message: String,
    user_auth: bool,
    title: &'a str,
    subtitle: &'a str,
}

#[derive(Serialize)]
struct PathArgs<'a> {
    path: &'a str,
    max: u32,
}

#[derive(Serialize)]
struct PushArgs<'a> {
    vapid: &'a str,
    title: &'a str,
    text: &'a str,
}

#[derive(Deserialize)]
struct KeyAnswer {
    key: String,
}

#[derive(Deserialize)]
struct SignAnswer {
    signature: String,
}

#[derive(Deserialize)]
struct ScanAnswer {
    text: Option<String>,
}

#[derive(Deserialize)]
struct TextAnswer {
    text: String,
}

/// An entry of a folder of the cloned vault.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

#[derive(Deserialize)]
struct ListAnswer {
    entries: Vec<Entry>,
}

/// What push looks like on this phone.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PushState {
    /// How many UnifiedPush distributor apps are installed.
    pub distributors: u32,
    /// Whether the person lets the app show notices.
    pub notices: bool,
    /// The endpoint the distributor gave, with the keys to encrypt for it (base64url); none yet.
    pub endpoint: Option<String>,
    pub p256dh: Option<String>,
    pub auth: Option<String>,
}

/// Standard base64 of bytes (what the Android side decodes the message with).
fn base64(bytes: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(target_os = "android")]
impl<R: Runtime> Platform<R> {
    fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        args: impl Serialize,
    ) -> Result<T, Refused> {
        self.0
            .run_mobile_plugin::<T>(method, args)
            .map_err(|e| match e {
                tauri::plugin::mobile::PluginInvokeError::InvokeRejected(r) => {
                    Refused::of(r.message.as_deref().unwrap_or("failed"))
                }
                _ => Refused::of("failed"),
            })
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> Platform<R> {
    #[allow(clippy::unused_self)]
    fn call<T: serde::de::DeserializeOwned>(
        &self,
        _method: &str,
        _args: impl Serialize,
    ) -> Result<T, Refused> {
        Err(Refused::of("unsupported"))
    }
}

impl<R: Runtime> Platform<R> {
    /// The public key of `alias` (SEC1 uncompressed, hex), made first when there is none.
    ///
    /// # Errors
    ///
    /// `no_lock` for a key bound to the person on a phone without a screen lock; `failed`.
    pub fn public_key(&self, alias: &str, user_auth: bool) -> Result<String, Refused> {
        self.call::<KeyAnswer>("publicKey", KeyArgs { alias, user_auth })
            .map(|a| a.key)
    }

    /// Remove the key of `alias` (unpairing, `MSEC-12`).
    ///
    /// # Errors
    ///
    /// `failed` when it is still there.
    pub fn delete_key(&self, alias: &str) -> Result<(), Refused> {
        self.call::<serde_json::Value>(
            "deleteKey",
            KeyArgs {
                alias,
                user_auth: false,
            },
        )
        .map(|_| ())
    }

    /// Sign `message` with the key of `alias` (ECDSA P-256 / SHA-256, `r` and `s` as hex). With `user_auth` the person
    /// is asked first (`title`, `subtitle` say what for).
    ///
    /// # Errors
    ///
    /// `cancelled`, `gone`, `no_lock`, `failed`.
    pub fn sign(
        &self,
        alias: &str,
        message: &[u8],
        user_auth: bool,
        title: &str,
        subtitle: &str,
    ) -> Result<String, Refused> {
        self.call::<SignAnswer>(
            "sign",
            SignArgs {
                alias,
                message: base64(message),
                user_auth,
                title,
                subtitle,
            },
        )
        .map(|a| a.signature)
    }

    /// The text of a QR code the person scans; `None` when they went back.
    ///
    /// # Errors
    ///
    /// `failed`.
    pub fn scan_qr(&self) -> Result<Option<String>, Refused> {
        self.call::<ScanAnswer>("scanQr", serde_json::json!({}))
            .map(|a| a.text)
    }

    /// Let the person choose the folder of the cloned vault (read only; the one before is let go).
    ///
    /// # Errors
    ///
    /// `cancelled`, `failed`.
    pub fn pick_folder(&self) -> Result<(), Refused> {
        self.call::<serde_json::Value>("pickFolder", serde_json::json!({}))
            .map(|_| ())
    }

    /// Let go of the folder (unpairing).
    ///
    /// # Errors
    ///
    /// `failed`.
    pub fn forget_folder(&self) -> Result<(), Refused> {
        self.call::<serde_json::Value>("forgetFolder", serde_json::json!({}))
            .map(|_| ())
    }

    /// The entries of `path` (names joined by `/`, empty for the folder itself).
    ///
    /// # Errors
    ///
    /// `no_folder`, `missing`, `failed`.
    pub fn list(&self, path: &str) -> Result<Vec<Entry>, Refused> {
        self.call::<ListAnswer>("listFolder", PathArgs { path, max: 0 })
            .map(|a| a.entries)
    }

    /// The text of the file `path`, at most `max` bytes.
    ///
    /// # Errors
    ///
    /// `no_folder`, `missing`, `too_large`, `failed`.
    pub fn read(&self, path: &str, max: u32) -> Result<String, Refused> {
        self.call::<TextAnswer>("readFile", PathArgs { path, max })
            .map(|a| a.text)
    }

    /// Register with the UnifiedPush distributor for the hub's VAPID key; the notice shows `title` and `text`.
    ///
    /// # Errors
    ///
    /// `no_distributor`, `failed`.
    pub fn push_register(&self, vapid: &str, title: &str, text: &str) -> Result<(), Refused> {
        self.call::<serde_json::Value>("pushRegister", PushArgs { vapid, title, text })
            .map(|_| ())
    }

    /// Unregister and forget the endpoint.
    ///
    /// # Errors
    ///
    /// `failed`.
    pub fn push_unregister(&self) -> Result<(), Refused> {
        self.call::<serde_json::Value>("pushUnregister", serde_json::json!({}))
            .map(|_| ())
    }

    /// What push looks like on this phone.
    ///
    /// # Errors
    ///
    /// `failed`.
    pub fn push_state(&self) -> Result<PushState, Refused> {
        self.call::<PushState>("pushState", serde_json::json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn a_refusal_is_one_of_the_words_or_failed() {
        assert_eq!(Refused::of("cancelled").0, "cancelled");
        assert_eq!(Refused::of("<script>").0, "failed");
    }
}
