//! The real network of the link: reqwest with rustls and the Mozilla roots. No redirects (an answer that points
//! elsewhere is an answer, never followed with the token), no cookies, no proxy of the system, a time limit for the
//! whole request and a byte limit checked while the body is read (`MSEC-08`).

use std::time::Duration;

use crate::link::{BoxFuture, Limits, Method, NetError, Request, Response, Transport};

/// The transport of an app.
pub struct NetTransport {
    client: reqwest::Client,
}

/// How long a connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

impl NetTransport {
    /// # Errors
    ///
    /// The TLS backend could not be set up (a fault of the build).
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| format!("no HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

/// The word for an error before an answer: it says what to check, never where.
fn unreachable(error: &reqwest::Error) -> NetError {
    if error.is_connect() {
        NetError::Unreachable("connect")
    } else if error.is_timeout() {
        NetError::Unreachable("timeout")
    } else {
        NetError::Unreachable("request")
    }
}

impl Transport for NetTransport {
    fn send(&self, request: Request, limits: Limits) -> BoxFuture<'_, Result<Response, NetError>> {
        Box::pin(async move {
            let method = match request.method {
                Method::Get => reqwest::Method::GET,
                Method::Post => reqwest::Method::POST,
            };
            let mut builder = self
                .client
                .request(method, &request.url)
                .timeout(limits.timeout);
            for (name, value) in &request.headers {
                builder = builder.header(*name, value);
            }
            if request.method == Method::Post {
                builder = builder.body(request.body);
            }
            let started = tokio::time::Instant::now();
            let mut answer = match builder.send().await {
                Ok(a) => a,
                // Not connected: nothing reached the hub. Anything later (a timeout after the request was written,
                // a reset): it may have.
                Err(e) if e.is_connect() || e.is_builder() => return Err(unreachable(&e)),
                Err(_) => return Err(NetError::AnswerLost),
            };
            if answer
                .content_length()
                .is_some_and(|n| n > limits.max_bytes as u64)
            {
                return Err(NetError::TooLarge);
            }
            let header = |name: reqwest::header::HeaderName| {
                answer
                    .headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .filter(|v| v.len() <= 200)
                    .map(str::to_owned)
            };
            let content_type = header(reqwest::header::CONTENT_TYPE);
            let etag = header(reqwest::header::ETAG);
            let status = answer.status().as_u16();
            let mut body = Vec::new();
            loop {
                let left = limits.timeout.saturating_sub(started.elapsed());
                let chunk = match tokio::time::timeout(left, answer.chunk()).await {
                    Ok(Ok(Some(chunk))) => chunk,
                    Ok(Ok(None)) => break,
                    Ok(Err(_)) | Err(_) => return Err(NetError::AnswerLost),
                };
                if body.len() + chunk.len() > limits.max_bytes {
                    return Err(NetError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Response {
                status,
                content_type,
                etag,
                body,
            })
        })
    }
}
