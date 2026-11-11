//! The real way out of the hub's Web Push (feature `push-net`): reqwest with rustls and the Mozilla roots, no redirect,
//! no proxy of the system, a time limit, and the answer read no further than its status. Only to endpoints
//! `push::allowed_endpoint` took when the device subscribed.

use crate::push::{PushRequest, PushTransport, SEND_TIMEOUT};
use crate::BoxFuture;

/// The transport of a hub that sends Web Push.
pub struct NetPush {
    client: reqwest::Client,
}

impl NetPush {
    /// # Errors
    ///
    /// The TLS backend could not be set up (a fault of the build).
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(SEND_TIMEOUT)
            .build()
            .map_err(|e| format!("no HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

impl PushTransport for NetPush {
    fn send(&self, request: PushRequest) -> BoxFuture<'_, Option<u16>> {
        Box::pin(async move {
            if !crate::push::allowed_endpoint(&request.endpoint) {
                return None;
            }
            let mut builder = self.client.post(&request.endpoint).body(request.body);
            for (name, value) in &request.headers {
                builder = builder.header(*name, value);
            }
            builder.send().await.ok().map(|r| r.status().as_u16())
        })
    }
}
