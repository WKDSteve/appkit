//! The event streams of a hub: how many a session holds open, and the body that carries one.
//!
//! An app writes the events of its stream itself (what changed differs per app); the count per
//! session, the answer when a session holds too many, and the response that sends what the task of
//! the connection writes are the same in every app and live here.

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::http::header::{self, HeaderValue};
use axum::http::StatusCode;
use axum::response::Response;
use futures_core::Stream;
use tokio::sync::mpsc;

use crate::guard::error_body;

/// The event streams that are open, by session token.
#[derive(Default)]
pub struct StreamCounter {
    by_token: Mutex<HashMap<String, usize>>,
}

/// One open stream; the place is given back when it is dropped.
pub struct StreamGuard {
    counter: Arc<StreamCounter>,
    token: String,
}

impl StreamCounter {
    /// A place for one more stream of `token`, or `None` when it holds `max` already.
    #[must_use]
    pub fn acquire(self: &Arc<Self>, token: &str, max: usize) -> Option<StreamGuard> {
        let mut map = self
            .by_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A refused stream leaves no entry behind: the map holds only tokens with a stream open.
        if map.get(token).copied().unwrap_or(0) >= max {
            return None;
        }
        *map.entry(token.to_owned()).or_insert(0) += 1;
        Some(StreamGuard {
            counter: Arc::clone(self),
            token: token.to_owned(),
        })
    }

    /// How many streams `token` holds open.
    #[must_use]
    pub fn open(&self, token: &str) -> usize {
        self.by_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(token)
            .copied()
            .unwrap_or(0)
    }

    /// How many tokens hold a stream open: a token whose last stream ended is forgotten.
    #[must_use]
    pub fn tokens(&self) -> usize {
        self.by_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        let mut map = self
            .counter
            .by_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(open) = map.get_mut(&self.token) {
            *open = open.saturating_sub(1);
            if *open == 0 {
                map.remove(&self.token);
            }
        }
    }
}

/// The answer to a session that holds as many streams as it may: `429 too_many_streams`.
#[must_use]
pub fn too_many_streams() -> Response {
    error_body(
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_streams",
        "this session has too many open event streams",
    )
}

/// The body of an event stream: what the task of the connection sends.
struct EventStream {
    rx: mpsc::Receiver<Bytes>,
}

impl Stream for EventStream {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|item| item.map(Ok))
    }
}

/// The response of an event stream (`text/event-stream`): it sends what arrives on `rx` and ends
/// when every sender is gone. The task that holds the sender sees the client leave by
/// `Sender::closed`.
#[must_use]
pub fn sse_response(rx: mpsc::Receiver<Bytes>) -> Response {
    let mut response = Response::new(Body::from_stream(EventStream { rx }));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_gets_as_many_streams_as_it_may_and_a_place_comes_back_when_one_ends() {
        let counter = Arc::new(StreamCounter::default());
        let a = counter.acquire("t", 2).unwrap();
        let b = counter.acquire("t", 2).unwrap();
        assert!(counter.acquire("t", 2).is_none());
        // Another session counts on its own.
        let other = counter.acquire("u", 2).unwrap();
        assert_eq!(counter.open("t"), 2);
        drop(a);
        assert_eq!(counter.open("t"), 1);
        let c = counter.acquire("t", 2).unwrap();
        drop((b, c, other));
        // A token whose last stream ended is not kept.
        assert_eq!(counter.open("t"), 0);
        assert_eq!(counter.tokens(), 0);
    }

    #[test]
    fn a_limit_of_none_opens_no_stream() {
        let counter = Arc::new(StreamCounter::default());
        assert!(counter.acquire("t", 0).is_none());
        // A refused stream leaves nothing behind.
        assert_eq!(counter.tokens(), 0);
    }

    #[tokio::test]
    async fn the_stream_sends_what_the_task_sends_as_an_event_stream() {
        let (tx, rx) = mpsc::channel::<Bytes>(4);
        let response = sse_response(rx);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream; charset=utf-8"
        );
        tx.send(Bytes::from_static(b"event: hello\ndata: {}\n\n"))
            .await
            .unwrap();
        drop(tx);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body[..], b"event: hello\ndata: {}\n\n");
    }

    #[test]
    fn too_many_streams_is_a_429() {
        assert_eq!(too_many_streams().status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
