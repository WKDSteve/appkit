//! Who is here: the connections on which the guard accepted a session.
//!
//! A hub that was put in the background has no window to close. This is what it looks at instead:
//! a page (or a paired phone) is here while a connection is open that carried a request with a
//! session, such as the event stream every page holds. `hello`, the page shell and a request with
//! the control secret do not count: a program that only asks who runs on the port never keeps a
//! hub alive. When nobody has been here for the grace time, [`Presence::left_alone`] resolves and
//! the app ends its hub, unless the person gave it an order to stay.
//!
//! The same count carries the one other reason a hub ends by itself: a local program with the
//! control secret asked it to (`<program> stop`, see [`Presence::ask_to_end`]). An order to stay
//! does not hold against that: the person at the terminal said so just now.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long a hub that was told to stay waits before it asks again whether that still holds. Long: the answer may
/// cost a look at the login entry of the system, and a hidden program that asks for it every few minutes is what an
/// endpoint protection reads as an attack.
const ASK_AGAIN: Duration = Duration::from_secs(60 * 60);

struct State {
    here: usize,
    /// Since when nobody is here; meaningful while `here` is 0. A hub nobody ever came to is
    /// alone since its start.
    alone_since: Instant,
}

/// The count of who is here, shared by the server and the app.
#[derive(Clone)]
pub struct Presence(Arc<Mutex<State>>, Arc<tokio::sync::watch::Sender<bool>>);

impl Default for Presence {
    fn default() -> Self {
        Self::new()
    }
}

impl Presence {
    #[must_use]
    pub fn new() -> Self {
        Self(
            Arc::new(Mutex::new(State {
                here: 0,
                alone_since: Instant::now(),
            })),
            Arc::new(tokio::sync::watch::Sender::new(false)),
        )
    }

    /// A local program with the control secret asked the hub to end (`<program> stop`). The hub
    /// ends itself in an orderly way; who asked ends it the hard way only when it does not.
    pub fn ask_to_end(&self) {
        self.1.send_replace(true);
    }

    /// Resolves once the hub was asked to end ([`Presence::ask_to_end`]); at once when it was
    /// asked before.
    pub async fn asked_to_end(&self) {
        let mut asked = self.1.subscribe();
        // The sender lives in `self`, so the wait cannot fail.
        let _ = asked.wait_for(|asked| *asked).await;
    }

    /// The mark of one connection: nobody yet, until the guard accepts a session on it.
    #[must_use]
    pub fn connection(&self) -> Arc<Here> {
        Arc::new(Here {
            presence: self.clone(),
            counted: AtomicBool::new(false),
        })
    }

    /// For how long nobody has been here; `None` while somebody is.
    #[must_use]
    pub fn alone_for(&self) -> Option<Duration> {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        (state.here == 0).then(|| state.alone_since.elapsed())
    }

    /// Somebody is on the way: a local program got a pairing code for a page it is about to open. Nobody is here
    /// yet, but the time alone starts again, so a hub at the end of its grace does not end under that page.
    pub fn expect_somebody(&self) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.alone_since = Instant::now();
    }

    /// Resolves once nobody has been here for `grace` and `stay` says there is no order to stay,
    /// or at once when the hub was asked to end ([`Presence::ask_to_end`]).
    /// `stay` is asked only when the time is over (it may read a file or the login entry), and
    /// again after a while when it said yes: an order that is taken back ends the wait.
    pub async fn left_alone<F: Fn() -> bool>(&self, grace: Duration, stay: F) {
        tokio::select! {
            () = self.asked_to_end() => {}
            () = self.nobody_for(grace, stay) => {}
        }
    }

    async fn nobody_for<F: Fn() -> bool>(&self, grace: Duration, stay: F) {
        loop {
            let wait = match self.alone_for() {
                Some(alone) if alone >= grace => {
                    if !stay() {
                        return;
                    }
                    grace.max(ASK_AGAIN)
                }
                Some(alone) => grace.saturating_sub(alone),
                None => grace,
            };
            tokio::time::sleep(wait.max(Duration::from_millis(20))).await;
        }
    }
}

/// One connection, as the count sees it. It leaves when the connection ends.
pub struct Here {
    presence: Presence,
    counted: AtomicBool,
}

impl Here {
    /// A session was accepted on this connection: somebody is here. Counted once per connection.
    pub fn arrive(&self) {
        if !self.counted.swap(true, Ordering::SeqCst) {
            let mut state = self
                .presence
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.here += 1;
        }
    }

    /// The count this connection belongs to.
    #[must_use]
    pub fn presence(&self) -> &Presence {
        &self.presence
    }

    /// The connection ended. Called by the server; a mark that is merely dropped leaves as well.
    pub fn leave(&self) {
        if self.counted.swap(false, Ordering::SeqCst) {
            let mut state = self
                .presence
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.here = state.here.saturating_sub(1);
            if state.here == 0 {
                state.alone_since = Instant::now();
            }
        }
    }
}

impl Drop for Here {
    fn drop(&mut self) {
        self.leave();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_counts_once_and_only_after_a_session() {
        let presence = Presence::new();
        let conn = presence.connection();
        assert!(presence.alone_for().is_some(), "a connection is nobody yet");
        conn.arrive();
        conn.arrive();
        assert!(presence.alone_for().is_none());
        let other = presence.connection();
        other.arrive();
        conn.leave();
        assert!(
            presence.alone_for().is_none(),
            "the other one is still here"
        );
        drop(other);
        assert!(presence.alone_for().is_some(), "a dropped mark has left");
        conn.leave();
        assert!(
            presence.alone_for().is_some(),
            "leaving twice takes nobody else away"
        );
    }

    #[tokio::test]
    async fn the_time_alone_starts_when_the_last_one_leaves() {
        let presence = Presence::new();
        let conn = presence.connection();
        conn.arrive();
        tokio::time::sleep(Duration::from_millis(60)).await;
        conn.leave();
        let alone = presence.alone_for().expect("alone now");
        assert!(alone < Duration::from_millis(50), "{alone:?}");
    }

    /// `<program> stop` asks the hub to end: that ends the wait at once, with somebody here and
    /// with an order to stay, and it is remembered for who asks later.
    #[tokio::test]
    async fn a_hub_that_was_asked_to_end_does_not_wait() {
        let presence = Presence::new();
        let conn = presence.connection();
        conn.arrive();
        let waiting = presence.left_alone(Duration::from_secs(3600), || true);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), waiting.as_mut())
                .await
                .is_err(),
            "nobody asked yet"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), presence.asked_to_end())
                .await
                .is_err(),
            "nobody asked yet"
        );
        presence.clone().ask_to_end();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("asked to end");
        tokio::time::timeout(Duration::from_secs(5), presence.asked_to_end())
            .await
            .expect("asked before");
    }

    #[tokio::test]
    async fn left_alone_waits_while_somebody_is_here_and_for_the_grace_after() {
        let presence = Presence::new();
        let conn = presence.connection();
        conn.arrive();
        let grace = Duration::from_millis(80);
        let waiting = presence.left_alone(grace, || false);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(250), waiting.as_mut())
                .await
                .is_err(),
            "nobody left yet"
        );
        let left = Instant::now();
        conn.leave();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("alone for the grace time");
        assert!(left.elapsed() >= grace, "{:?}", left.elapsed());
    }

    #[tokio::test]
    async fn a_page_on_its_way_starts_the_time_alone_again_and_is_nobody_yet() {
        let presence = Presence::new();
        tokio::time::sleep(Duration::from_millis(80)).await;
        presence.expect_somebody();
        let alone = presence.alone_for().expect("nobody is here yet");
        assert!(alone < Duration::from_millis(60), "{alone:?}");
    }

    #[tokio::test]
    async fn a_hub_nobody_came_to_is_alone_from_its_start() {
        let presence = Presence::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            presence.left_alone(Duration::from_millis(40), || false),
        )
        .await
        .expect("nobody ever came");
    }

    #[tokio::test]
    async fn an_order_to_stay_keeps_the_wait_open() {
        let presence = Presence::new();
        let asked = std::sync::atomic::AtomicUsize::new(0);
        let waiting = presence.left_alone(Duration::from_millis(30), || {
            asked.fetch_add(1, Ordering::SeqCst);
            true
        });
        assert!(tokio::time::timeout(Duration::from_millis(300), waiting)
            .await
            .is_err());
        // Asked when the grace was over, and not in a loop since.
        assert_eq!(asked.load(Ordering::SeqCst), 1);
    }
}
