//! The link of a paired phone to its hub: pairing, the device login, and every request of the page.
//!
//! The page never builds an address and never holds the session: it names a route of its app, and the link adds the
//! base of the way in use, the origin, the token, a time limit and a byte limit. A route outside the app's own
//! namespace is not sent at all. Before the phone signs anything, the hub has to name the id it named at pairing
//! (`MSEC-04`): another program on the same name or port gets no signature.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::state::Paired;
use crate::way::{HubAddress, PairLink};

/// A future that can be sent across threads.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The routes of the core the link uses (`appkit-hub`, `guard.rs`).
pub const DEVICE_PAIR_PATH: &str = "/auth/device/pair";
pub const DEVICE_CHALLENGE_PATH: &str = "/auth/device/challenge";
pub const DEVICE_SESSION_PATH: &str = "/auth/device/session";

/// What a device signs to open a session: the same text as `appkit_hub::devices::login_message`, byte for byte.
#[must_use]
pub fn login_message(origin: &str, device: &str, challenge: &str) -> String {
    format!("appkit-device-login/1\n{origin}\n{device}\n{challenge}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// One request as a [`Transport`] sends it.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

/// An answer: the status, the two headers the page reads, and the body (never more than the limit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

/// Why a request has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// It never reached the hub (no route, refused, no name, TLS): trying again is safe. The word says which, for
    /// the page to say what to do ("Tailscale is off"); it holds no address and no content.
    Unreachable(&'static str),
    /// It may have arrived and the answer did not come (in time, or whole): a write is not sent again by itself
    /// (`MOB-07`).
    AnswerLost,
    /// The answer was larger than the limit; it was not read further.
    TooLarge,
}

/// How long a request may take and how large its answer may be (`MSEC-08`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
            max_bytes: 4 * 1024 * 1024,
        }
    }
}

/// The network under the link: reqwest in an app (`net`), a stand-in in a test.
pub trait Transport: Send + Sync {
    fn send(&self, request: Request, limits: Limits) -> BoxFuture<'_, Result<Response, NetError>>;
}

/// Why a key could not sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignError {
    /// The key is not there any more (the app's data was cleared, a restore on another phone): pair again.
    Gone,
    /// The person did not confirm (the biometric prompt was cancelled).
    Cancelled,
    /// Anything else, as a short word without content.
    Failed(&'static str),
}

/// The device key: it signs and shows its public half, nothing more. On a phone the Android Keystore holds it and it
/// cannot be exported (`AND-05`).
pub trait Signer: Send + Sync {
    /// The public key, SEC1 uncompressed P-256 (65 bytes), lower-case hex.
    fn public_key(&self) -> BoxFuture<'_, Result<String, SignError>>;
    /// An ECDSA P-256 / SHA-256 signature of `message`: `r` and `s` (64 bytes), lower-case hex.
    fn sign<'a>(&'a self, message: &'a [u8]) -> BoxFuture<'a, Result<String, SignError>>;
}

/// A key in memory: for tests and a desktop preview. Never for a phone.
#[cfg(any(test, feature = "software-signer"))]
pub struct SoftwareSigner(p256::ecdsa::SigningKey);

#[cfg(any(test, feature = "software-signer"))]
impl SoftwareSigner {
    /// A new random key.
    ///
    /// # Panics
    ///
    /// When the system has no random source.
    #[must_use]
    pub fn random() -> Self {
        loop {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).expect("the system has a random source");
            if let Ok(key) = p256::ecdsa::SigningKey::from_slice(&bytes) {
                return Self(key);
            }
        }
    }
}

#[cfg(any(test, feature = "software-signer"))]
impl Signer for SoftwareSigner {
    fn public_key(&self) -> BoxFuture<'_, Result<String, SignError>> {
        let key = crate::hex(self.0.verifying_key().to_sec1_point(false).as_bytes());
        Box::pin(async move { Ok(key) })
    }

    fn sign<'a>(&'a self, message: &'a [u8]) -> BoxFuture<'a, Result<String, SignError>> {
        use p256::ecdsa::signature::Signer as _;
        let signature: p256::ecdsa::Signature = self.0.sign(message);
        Box::pin(async move { Ok(crate::hex(&signature.to_bytes())) })
    }
}

/// The app a link is for: the name its hub says in `hello`, and the namespace of its routes (`testing` gives
/// `/api/testing/...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRoutes {
    pub app: String,
    pub namespace: String,
}

impl AppRoutes {
    #[must_use]
    pub fn new(app: &str, namespace: &str) -> Self {
        Self {
            app: app.to_owned(),
            namespace: namespace.to_owned(),
        }
    }

    fn prefix(&self) -> String {
        format!("/api/{}/", self.namespace)
    }

    /// Is `path` (with an optional query) a plain route of this app? Only those are sent.
    #[must_use]
    pub fn allows(&self, path: &str) -> bool {
        let (route, query) = match path.split_once('?') {
            Some((r, q)) => (r, Some(q)),
            None => (path, None),
        };
        let Some(rest) = route.strip_prefix(&self.prefix()) else {
            return false;
        };
        path.len() <= 2048
            && !rest.is_empty()
            && rest.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            })
            && query.is_none_or(|q| {
                q.bytes()
                    .all(|b| b.is_ascii_graphic() && b != b'#' && b != b'?')
            })
    }
}

/// Why the link could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Net(NetError),
    Sign(SignError),
    /// The hub refused with this status and code (a pairing code that is spent, a full list of devices).
    Refused {
        status: u16,
        code: String,
    },
    /// The hub does not take this device any more (revoked, or never paired here): forget the pairing and pair again.
    Unpaired,
    /// The program there names another id than the hub paired with: nothing was signed (`MSEC-04`).
    OtherHub,
    /// The link or the hub is of another app.
    OtherApp,
    /// The route is not one of the app's: nothing was sent.
    NotAllowed,
    /// The answer is not what the hub sends.
    BadReply,
}

impl From<NetError> for LinkError {
    fn from(e: NetError) -> Self {
        Self::Net(e)
    }
}

impl From<SignError> for LinkError {
    fn from(e: SignError) -> Self {
        Self::Sign(e)
    }
}

/// The JSON body of a refusal of the hub: `{"error": {"code": ...}}`.
fn refusal(response: &Response) -> LinkError {
    let code = serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|v| v["error"]["code"].as_str().map(str::to_owned))
        .filter(|c| {
            c.len() <= 48
                && c.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        .unwrap_or_default();
    LinkError::Refused {
        status: response.status,
        code,
    }
}

fn object(response: &Response) -> Result<Value, LinkError> {
    serde_json::from_slice::<Value>(&response.body)
        .ok()
        .filter(Value::is_object)
        .ok_or(LinkError::BadReply)
}

fn hex_field(doc: &Value, key: &str, bytes: usize) -> Result<String, LinkError> {
    doc[key]
        .as_str()
        .filter(|v| crate::is_hex(v, bytes))
        .map(str::to_owned)
        .ok_or(LinkError::BadReply)
}

/// A token as the hub makes it: 32 bytes of hex.
fn token_field(doc: &Value) -> Result<String, LinkError> {
    hex_field(doc, "token", 32)
}

async fn post_json(
    transport: &dyn Transport,
    address: &HubAddress,
    path: &str,
    body: &Value,
    limits: Limits,
) -> Result<Response, NetError> {
    transport
        .send(
            Request {
                method: Method::Post,
                url: format!("{}{path}", address.base()),
                headers: vec![
                    ("Origin", address.origin().to_owned()),
                    ("Content-Type", "application/json".to_owned()),
                ],
                body: body.to_string().into_bytes(),
            },
            limits,
        )
        .await
}

/// Pair this phone with the hub of a pairing link: check that the hub is of `routes.app`, send the public key with the
/// one-time code and the name the person gave the phone, and keep what the hub answered. The link that comes back
/// already holds the first session.
///
/// # Errors
///
/// [`LinkError`]: the link is of another app, the hub refused (the code is spent or old, the list is full), the key
/// could not be read, or the network failed. Nothing is kept then.
pub async fn pair(
    transport: Arc<dyn Transport>,
    signer: Arc<dyn Signer>,
    routes: &AppRoutes,
    link: &PairLink,
    name: &str,
    limits: Limits,
) -> Result<(Paired, Link), LinkError> {
    if link.app != routes.namespace {
        return Err(LinkError::OtherApp);
    }
    let hello = transport
        .send(
            Request {
                method: Method::Get,
                url: format!("{}/api/{}/hello", link.address.base(), routes.namespace),
                headers: vec![("Origin", link.address.origin().to_owned())],
                body: Vec::new(),
            },
            limits,
        )
        .await?;
    if hello.status != 200 {
        return Err(refusal(&hello));
    }
    if object(&hello)?["app"].as_str() != Some(routes.app.as_str()) {
        return Err(LinkError::OtherApp);
    }
    let key = signer.public_key().await?;
    let answer = post_json(
        transport.as_ref(),
        &link.address,
        DEVICE_PAIR_PATH,
        &json!({ "code": link.code, "name": name, "key": key }),
        limits,
    )
    .await?;
    if answer.status != 200 {
        return Err(refusal(&answer));
    }
    let doc = object(&answer)?;
    let paired = Paired {
        address: link.address.clone(),
        app: routes.namespace.clone(),
        device: hex_field(&doc, "device", 16)?,
        hub: hex_field(&doc, "hub", 16)?,
        name: name.to_owned(),
    };
    let token = token_field(&doc)?;
    let link = Link::new(transport, signer, routes.clone(), paired.clone(), limits);
    *link.session.lock().await = Some(token);
    Ok((paired, link))
}

/// A paired phone's way to its hub.
pub struct Link {
    transport: Arc<dyn Transport>,
    signer: Arc<dyn Signer>,
    routes: AppRoutes,
    paired: Paired,
    limits: Limits,
    /// The session token: in memory only (`MSEC-01`), one login at a time.
    session: tokio::sync::Mutex<Option<String>>,
}

impl Link {
    /// The link of a phone that was paired before (its kept state). No request is made yet.
    #[must_use]
    pub fn new(
        transport: Arc<dyn Transport>,
        signer: Arc<dyn Signer>,
        routes: AppRoutes,
        paired: Paired,
        limits: Limits,
    ) -> Self {
        Self {
            transport,
            signer,
            routes,
            paired,
            limits,
            session: tokio::sync::Mutex::new(None),
        }
    }

    #[must_use]
    pub fn paired(&self) -> &Paired {
        &self.paired
    }

    /// Open a session: ask a challenge, check that the hub names the id it named at pairing, sign, and get a token.
    async fn login(&self) -> Result<String, LinkError> {
        let address = &self.paired.address;
        let asked = post_json(
            self.transport.as_ref(),
            address,
            DEVICE_CHALLENGE_PATH,
            &json!({ "device": self.paired.device }),
            self.limits,
        )
        .await?;
        if asked.status != 200 {
            return Err(refusal(&asked));
        }
        let doc = object(&asked)?;
        // Before anything is signed: is this the hub of the pairing?
        if doc["hub"].as_str() != Some(self.paired.hub.as_str()) {
            return Err(LinkError::OtherHub);
        }
        let challenge = hex_field(&doc, "challenge", 32)?;
        let message = login_message(address.origin(), &self.paired.device, &challenge);
        let signature = self.signer.sign(message.as_bytes()).await?;
        let answer = post_json(
            self.transport.as_ref(),
            address,
            DEVICE_SESSION_PATH,
            &json!({ "device": self.paired.device, "challenge": challenge, "signature": signature }),
            self.limits,
        )
        .await?;
        match answer.status {
            200 => token_field(&object(&answer)?),
            401 => Err(LinkError::Unpaired),
            _ => Err(refusal(&answer)),
        }
    }

    async fn send_with(
        &self,
        token: &str,
        method: Method,
        path: &str,
        body: &[u8],
        if_none_match: Option<&str>,
    ) -> Result<Response, NetError> {
        let address = &self.paired.address;
        let mut headers = vec![
            ("Origin", address.origin().to_owned()),
            ("Authorization", format!("Bearer {token}")),
        ];
        if method == Method::Post {
            headers.push(("Content-Type", "application/json".to_owned()));
        }
        if let Some(tag) = if_none_match {
            headers.push(("If-None-Match", tag.to_owned()));
        }
        self.transport
            .send(
                Request {
                    method,
                    url: format!("{}{path}", address.base()),
                    headers,
                    body: body.to_vec(),
                },
                self.limits,
            )
            .await
    }

    /// A request of the page to a route of its app, with the session (opened first when there is none). When the
    /// hub does not take the session (it restarted), the link logs in again and sends once more: the guard refused
    /// before the route ran, so nothing happened twice. A `POST` carries JSON.
    ///
    /// # Errors
    ///
    /// [`LinkError::NotAllowed`] for a route that is not the app's (nothing is sent); [`LinkError::Unpaired`] when the
    /// hub does not take the device any more; [`LinkError::OtherHub`]; or the network's error.
    pub async fn fetch(
        &self,
        method: Method,
        path: &str,
        body: &[u8],
        if_none_match: Option<&str>,
    ) -> Result<Response, LinkError> {
        if !self.routes.allows(path)
            || if_none_match.is_some_and(|t| {
                t.len() > 200 || !t.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
            })
        {
            return Err(LinkError::NotAllowed);
        }
        let mut session = self.session.lock().await;
        if session.is_none() {
            *session = Some(self.login().await?);
        }
        let token = session.clone().unwrap_or_default();
        let response = self
            .send_with(&token, method, path, body, if_none_match)
            .await?;
        if response.status != 401 {
            return Ok(response);
        }
        *session = None;
        let token = self.login().await?;
        *session = Some(token.clone());
        Ok(self
            .send_with(&token, method, path, body, if_none_match)
            .await?)
    }

    /// End the session here (the hub keeps the device). The next request logs in again.
    pub async fn drop_session(&self) {
        *self.session.lock().await = None;
    }

    /// A read of a route with the kept copy (`MOB-08`): asks the hub with the tag of the copy; a new answer replaces
    /// it, "not modified" keeps it (as fresh); when the hub cannot be reached the copy is given, marked as kept, with
    /// its time. A refusal of the hub is given as it is and not kept. When the hub does not take the device any more
    /// the copy is forgotten (`MSEC-12`).
    ///
    /// # Errors
    ///
    /// As [`Link::fetch`], when there is no copy to fall back on; [`LinkError::BadReply`] for an answer that is not
    /// text.
    pub async fn read(
        &self,
        route: &str,
        cache: &crate::cache::Cache,
        now: u64,
    ) -> Result<Read, LinkError> {
        let kept = cache.get(route);
        let tag = kept.as_ref().and_then(|k| k.etag.clone());
        match self.fetch(Method::Get, route, &[], tag.as_deref()).await {
            Ok(r) if r.status == 304 && kept.is_some() => {
                let kept = kept.unwrap_or_else(|| unreachable!());
                let _ = cache.put(
                    route,
                    crate::cache::Entry {
                        fetched_at: now,
                        ..kept.clone()
                    },
                );
                Ok(Read {
                    status: 200,
                    body: kept.body,
                    etag: kept.etag,
                    fetched_at: now,
                    kept: false,
                })
            }
            Ok(r) => {
                let body = String::from_utf8(r.body).map_err(|_| LinkError::BadReply)?;
                if r.status == 200 {
                    let _ = cache.put(
                        route,
                        crate::cache::Entry {
                            etag: r.etag.clone(),
                            fetched_at: now,
                            body: body.clone(),
                        },
                    );
                }
                Ok(Read {
                    status: r.status,
                    body,
                    etag: r.etag,
                    fetched_at: now,
                    kept: false,
                })
            }
            Err(LinkError::Net(NetError::Unreachable(_) | NetError::AnswerLost))
                if kept.is_some() =>
            {
                let kept = kept.unwrap_or_else(|| unreachable!());
                Ok(Read {
                    status: 200,
                    body: kept.body,
                    etag: kept.etag,
                    fetched_at: kept.fetched_at,
                    kept: true,
                })
            }
            Err(LinkError::Unpaired) => {
                let _ = cache.forget();
                Err(LinkError::Unpaired)
            }
            Err(e) => Err(e),
        }
    }
}

/// What [`Link::read`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub status: u16,
    pub body: String,
    pub etag: Option<String>,
    /// When the hub sent it, seconds since 1970.
    pub fetched_at: u64,
    /// The hub was not reached: this is the kept copy (the page says so, with its age).
    pub kept: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_routes_of_the_app_are_sent() {
        let r = AppRoutes::new("testing-desktop", "testing");
        for ok in [
            "/api/testing/fleet",
            "/api/testing/repo?key=lib.nvim",
            "/api/testing/sheet-step/sheets?key=a%2Fb",
        ] {
            assert!(r.allows(ok), "{ok}");
        }
        for bad in [
            "/api/testing",
            "/api/testing/",
            "/api/testing//fleet",
            "/api/testing/../control/pair",
            "/api/testing/./fleet",
            "/api/control/pair",
            "/api/vaults/select",
            "/auth/exchange",
            "/api/tasks/snapshot",
            "/api/testingx/fleet",
            "https://evil.example/api/testing/fleet",
            "/api/testing/fleet?a=b#c",
            "/api/testing/fleet?a b",
            "/api/testing/fl eet",
            "/api/testing/fleet\r\nX: y",
        ] {
            assert!(!r.allows(bad), "{bad:?}");
        }
        assert!(!r.allows(&format!("/api/testing/{}", "a".repeat(3000))));
    }

    #[test]
    fn the_login_message_is_the_hubs() {
        assert_eq!(
            login_message("https://pc.example.ts.net", "d", "c"),
            "appkit-device-login/1\nhttps://pc.example.ts.net\nd\nc"
        );
    }
}
