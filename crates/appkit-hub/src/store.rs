//! The snapshot the hub holds, and the events that tell it changed.
//!
//! One snapshot of the vault is kept. It is served from memory until
//! something says the vault changed: the watcher (after a quiet moment), the
//! timer (a full scan, the net under lost events) or a client asking for a
//! rescan. A change is then one question to the engine. The new document is
//! compared with the old one by id and `etag`, and the difference is
//! sent to the event stream as a *hint*: a client that gets one reads again.
//!
//! The hub never reads a document file here. It only compares what the engine
//! said. What the app calls its documents (the key of the list in the snapshot, the name of the
//! event, the problems that pass by) is a [`SnapshotShape`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};
use tokio::task::JoinHandle;

use crate::engine::{Engine, EngineError, Method};
use crate::fs_watch::Hint;

/// The most ids one event names for each of changed, added and removed.
/// Beyond that the event says `truncated` and the client reads again, which
/// it does anyway.
const MAX_IDS: usize = 200;

/// What the store must know about the documents of the app.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotShape {
    /// The call that returns the snapshot.
    pub call: Method,
    /// The key of the list of documents in the snapshot (`tasks`); each has an `id`, an `etag`
    /// and `problems` with a `code` each.
    pub items_key: &'static str,
    /// The name of the server-sent event that tells a change (`tasks`).
    pub event: &'static str,
    /// Problems the engine reports while a file is in the middle of a change: a snapshot that
    /// holds one is read once more after a moment. (`done` puts the copy in place before the
    /// original is gone: two documents of one id for a moment; an editor's save may rename a
    /// backup.)
    pub transitional: &'static [&'static str],
}

struct Lite {
    digest: Option<String>,
    rev: Option<String>,
    items: Vec<LiteItem>,
}

/// Reads a snapshot for the few things the store needs and skips the rest, with the key of the
/// list of documents taken from the shape (a derive cannot name a key at run time).
struct LiteSeed<'a>(&'a str);

impl<'de> DeserializeSeed<'de> for LiteSeed<'_> {
    type Value = Lite;

    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Lite, D::Error> {
        de.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for LiteSeed<'_> {
    type Value = Lite;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a snapshot")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Lite, A::Error> {
        let mut lite = Lite {
            digest: None,
            rev: None,
            items: Vec::new(),
        };
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "digest" => lite.digest = map.next_value()?,
                "rev" => lite.rev = map.next_value()?,
                k if k == self.0 => lite.items = map.next_value()?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(lite)
    }
}

/// The snapshot, read for what the store needs; `None` when it is not a snapshot.
fn parse_lite(text: &str, shape: &SnapshotShape) -> Option<Lite> {
    let mut de = serde_json::Deserializer::from_str(text);
    let lite = LiteSeed(shape.items_key).deserialize(&mut de).ok()?;
    de.end().ok()?;
    Some(lite)
}

#[derive(Deserialize)]
struct LiteItem {
    id: String,
    etag: Option<String>,
    #[serde(default)]
    problems: Vec<LiteProblem>,
}

#[derive(Deserialize)]
struct LiteProblem {
    code: String,
}

/// A snapshot as the hub holds it.
pub struct Cached {
    /// The document, as the engine wrote it. Reference counted: answering
    /// from the cache copies nothing.
    pub text: Bytes,
    /// Its digest: the ETag.
    pub etag: Option<String>,
    pub rev: Option<String>,
    /// How many changes the hub has seen since it started.
    pub seq: u64,
    fetched: Instant,
    /// Document id to etag. `None` when the document could not be read for ids:
    /// a change is then told as "anything may differ", not as "all removed".
    ids: Option<BTreeMap<String, Option<String>>>,
}

/// What a client of the event stream is told. Ids only, never a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub seq: u64,
    pub rev: Option<String>,
    /// What woke the scan: `fs`, `timer`, `rescan`, `lost`, or `fetch` (a
    /// request found the held snapshot too old and read it again).
    pub cause: &'static str,
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub truncated: bool,
}

impl Change {
    /// The server-sent event for it.
    #[must_use]
    pub fn to_sse(&self, event: &str) -> String {
        let data = json!({
            "rev": self.rev,
            "seq": self.seq,
            "cause": self.cause,
            "changed_ids": self.changed,
            "added": self.added,
            "removed": self.removed,
            "truncated": self.truncated,
        });
        format!("event: {event}\ndata: {data}\n\n")
    }
}

/// Who may hold the snapshot and for how long.
#[derive(Debug, Clone)]
pub struct StoreOptions {
    /// A snapshot older than this is fetched again when asked for. `None`:
    /// only a change invalidates it (the watcher and the timer are there).
    pub max_age: Option<Duration>,
    /// How long to wait before a transitional state is read once more.
    pub settle: Duration,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            max_age: Some(Duration::from_secs(1)),
            settle: Duration::from_millis(500),
        }
    }
}

pub struct SnapshotStore {
    engine: Option<Arc<dyn Engine>>,
    shape: SnapshotShape,
    options: StoreOptions,
    /// What the snapshot is asked with, every time.
    params: Option<Value>,
    state: Mutex<Option<Arc<Cached>>>,
    /// One fetch at a time; the others wait and then find it done.
    fetching: AsyncMutex<()>,
    seq: AtomicU64,
    events: broadcast::Sender<Arc<Change>>,
    /// Receivers of `events` that belong to the hub itself (the search
    /// index): they do not make a listener.
    internal: std::sync::atomic::AtomicUsize,
    rescan: Notify,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl SnapshotStore {
    #[must_use]
    pub fn new(
        engine: Option<Arc<dyn Engine>>,
        shape: SnapshotShape,
        options: StoreOptions,
    ) -> Arc<Self> {
        Self::with_params(engine, shape, options, None)
    }

    /// A store whose snapshot is asked with these parameters, for an engine that cannot find what
    /// it reads by itself (a child with an allow-listed environment). They are the same for every
    /// question: there is one snapshot.
    #[must_use]
    pub fn with_params(
        engine: Option<Arc<dyn Engine>>,
        shape: SnapshotShape,
        options: StoreOptions,
        params: Option<Value>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        Arc::new(Self {
            engine,
            shape,
            options,
            params,
            state: Mutex::new(None),
            fetching: AsyncMutex::new(()),
            seq: AtomicU64::new(0),
            events,
            internal: std::sync::atomic::AtomicUsize::new(0),
            rescan: Notify::new(),
        })
    }

    /// Is there an engine to ask?
    #[must_use]
    pub fn has_engine(&self) -> bool {
        self.engine.is_some()
    }

    /// Is any event stream open? Nobody to tell, nothing to scan for.
    #[must_use]
    pub fn has_listeners(&self) -> bool {
        self.events.receiver_count() > self.internal.load(Ordering::Acquire)
    }

    /// A receiver for a part of the hub: what it follows does not make the
    /// full scan run for nobody (see `has_listeners`).
    #[must_use]
    pub fn subscribe_quiet(&self) -> broadcast::Receiver<Arc<Change>> {
        self.internal.fetch_add(1, Ordering::AcqRel);
        self.events.subscribe()
    }

    fn cached(&self) -> Option<Arc<Cached>> {
        lock(&self.state).clone()
    }

    /// The snapshot the hub holds, of any age, without asking the engine for
    /// one: for what only wants to know the state the hub stands in. (A
    /// change replaces it; its age says nothing about whether it is true.)
    #[must_use]
    pub fn latest(&self) -> Option<Arc<Cached>> {
        self.cached()
    }

    fn fresh_enough(&self, c: &Cached) -> bool {
        self.options
            .max_age
            .is_none_or(|max| c.fetched.elapsed() < max)
    }

    /// The snapshot: from memory, or fetched once for everyone who asks at
    /// the same time.
    ///
    /// # Errors
    ///
    /// What the engine said, or `Unavailable` when there is none.
    pub async fn current(&self) -> Result<Arc<Cached>, EngineError> {
        if let Some(c) = self.cached().filter(|c| self.fresh_enough(c)) {
            return Ok(c);
        }
        let _gate = self.fetching.lock().await;
        // Whoever held the gate may have done it.
        if let Some(c) = self.cached().filter(|c| self.fresh_enough(c)) {
            return Ok(c);
        }
        self.fetch_and_install("fetch").await
    }

    async fn fetch_text(&self) -> Result<String, EngineError> {
        let Some(engine) = &self.engine else {
            return Err(EngineError::Spawn("no engine".to_owned()));
        };
        Ok(engine
            .call(self.shape.call, self.params.as_ref())
            .await?
            .text)
    }

    /// Ask the engine, wait out a transitional state once, install the
    /// answer and tell the stream what changed. Holds the `fetching` gate.
    async fn fetch_and_install(&self, cause: &'static str) -> Result<Arc<Cached>, EngineError> {
        let mut text = self.fetch_text().await?;
        let mut lite = parse_lite(&text, &self.shape);
        if lite
            .as_ref()
            .is_some_and(|l| has_transitional(l, &self.shape))
        {
            tokio::time::sleep(self.options.settle).await;
            text = self.fetch_text().await?;
            lite = parse_lite(&text, &self.shape);
        }
        Ok(self.install(text, lite, cause))
    }

    fn install(&self, text: String, lite: Option<Lite>, cause: &'static str) -> Arc<Cached> {
        let (digest, rev, ids) = match lite {
            Some(l) => (
                l.digest,
                l.rev,
                Some(
                    l.items
                        .into_iter()
                        .map(|t| (t.id, t.etag))
                        .collect::<BTreeMap<_, _>>(),
                ),
            ),
            None => (None, None, None),
        };
        let etag = digest.filter(|d| plain_digest(d));

        let mut slot = lock(&self.state);
        let previous = slot.clone();
        let same = previous.as_ref().is_some_and(|p| match (&p.etag, &etag) {
            (Some(a), Some(b)) => a == b,
            _ => p.text == text.as_bytes(),
        });
        // The document did not change: keep the old one (and its sequence).
        if let (true, Some(p)) = (same, &previous) {
            let refreshed = Arc::new(Cached {
                text: p.text.clone(),
                etag: p.etag.clone(),
                rev: p.rev.clone(),
                seq: p.seq,
                fetched: Instant::now(),
                ids: p.ids.clone(),
            });
            *slot = Some(Arc::clone(&refreshed));
            return refreshed;
        }

        let seq = if previous.is_some() {
            self.seq.fetch_add(1, Ordering::AcqRel) + 1
        } else {
            self.seq.load(Ordering::Acquire)
        };
        let cached = Arc::new(Cached {
            text: Bytes::from(text),
            etag,
            rev,
            seq,
            fetched: Instant::now(),
            ids,
        });
        *slot = Some(Arc::clone(&cached));
        drop(slot);

        // The first snapshot is not a change of what a client holds (it
        // reads before it listens), but a part of the hub that built
        // something from the vault before it must know the vault is read:
        // it is told "anything may differ", and compares the revision.
        if let Some(p) = previous {
            let change = diff(&p, &cached, cause);
            let _ = self.events.send(Arc::new(change));
        } else {
            let _ = self.events.send(Arc::new(Change {
                seq: cached.seq,
                rev: cached.rev.clone(),
                cause,
                changed: Vec::new(),
                added: Vec::new(),
                removed: Vec::new(),
                truncated: true,
            }));
        }
        cached
    }

    /// Scan the vault now and tell the stream what changed.
    ///
    /// # Errors
    ///
    /// What the engine said.
    pub async fn refresh(&self, cause: &'static str) -> Result<(), EngineError> {
        let _gate = self.fetching.lock().await;
        self.fetch_and_install(cause).await.map(|_| ())
    }

    /// Ask for a scan from outside (a client that got the focus again).
    /// Many asks while one is running are one scan.
    pub fn request_rescan(&self) {
        self.rescan.notify_one();
    }

    /// Follow the changes: a receiver that gets every `Change` from now on,
    /// and where the stream stands.
    #[must_use]
    pub fn subscribe(&self) -> (broadcast::Receiver<Arc<Change>>, u64, Option<String>) {
        // Subscribe first: a change between the two lines is then not lost.
        let rx = self.events.subscribe();
        let current = self.cached();
        let seq = current.as_ref().map_or(0, |c| c.seq);
        let rev = current.and_then(|c| c.rev.clone());
        (rx, seq, rev)
    }
}

/// A digest is a header value: letters, digits and a colon, nothing else.
fn plain_digest(digest: &str) -> bool {
    !digest.is_empty()
        && digest.len() <= 128
        && digest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b':')
}

fn has_transitional(lite: &Lite, shape: &SnapshotShape) -> bool {
    lite.items
        .iter()
        .flat_map(|t| &t.problems)
        .any(|p| shape.transitional.contains(&p.code.as_str()))
}

/// What differs between two snapshots, as ids.
fn diff(old: &Cached, new: &Cached, cause: &'static str) -> Change {
    let (Some(old_ids), Some(new_ids)) = (&old.ids, &new.ids) else {
        // One of the two could not be read for ids: no list would be true.
        // The event says so, and the client reads again, as it does anyway.
        return Change {
            seq: new.seq,
            rev: new.rev.clone(),
            cause,
            changed: Vec::new(),
            added: Vec::new(),
            removed: Vec::new(),
            truncated: true,
        };
    };
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for (id, etag) in new_ids {
        match old_ids.get(id) {
            None => added.push(id.clone()),
            Some(before) if before != etag => changed.push(id.clone()),
            Some(_) => {}
        }
    }
    for id in old_ids.keys() {
        if !new_ids.contains_key(id) {
            removed.push(id.clone());
        }
    }
    let truncated = [&changed, &added, &removed]
        .iter()
        .any(|l| l.len() > MAX_IDS);
    for list in [&mut changed, &mut added, &mut removed] {
        list.truncate(MAX_IDS);
    }
    Change {
        seq: new.seq,
        rev: new.rev.clone(),
        cause,
        changed,
        added,
        removed,
        truncated,
    }
}

/// The timings of the refresher.
#[derive(Debug, Clone)]
pub struct RefreshOptions {
    /// How long the vault must be quiet before it is scanned (a `git pull`
    /// of hundreds of files is one scan).
    pub debounce: Duration,
    /// The longest a steady stream of changes may keep the scan waiting.
    pub max_wait: Duration,
    /// The full scan that catches what the watcher lost.
    pub interval: Duration,
}

impl Default for RefreshOptions {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(300),
            max_wait: Duration::from_secs(2),
            interval: Duration::from_secs(45),
        }
    }
}

/// Run the refresher until the hints end or the task is aborted.
pub fn spawn_refresher(
    store: Arc<SnapshotStore>,
    mut hints: UnboundedReceiver<Hint>,
    options: RefreshOptions,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + options.interval,
            options.interval,
        );
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let cause = tokio::select! {
                hint = hints.recv() => match hint {
                    None => return,
                    Some(first) => {
                        let lost = settle(&mut hints, first, &options).await;
                        if lost { "lost" } else { "fs" }
                    }
                },
                _ = ticker.tick() => {
                    // The full scan is the net under lost events for the
                    // streams that are open. With none open it costs a
                    // Neovim process every period for nobody (a scan of a
                    // real vault takes about a second); the held snapshot
                    // has an age, and a request that finds it too old
                    // reads the vault again.
                    if !store.has_listeners() {
                        continue;
                    }
                    "timer"
                },
                () = store.rescan.notified() => "rescan",
            };
            if let Err(e) = store.refresh(cause).await {
                // The old snapshot stays; the next change or tick tries again.
                eprintln!("scan: {e}");
            }
        }
    })
}

/// Wait until no hint has come for `debounce` (but not longer than
/// `max_wait` from the first). True when one of them was a lost event.
async fn settle(
    hints: &mut UnboundedReceiver<Hint>,
    first: Hint,
    options: &RefreshOptions,
) -> bool {
    let started = tokio::time::Instant::now();
    let mut lost = first == Hint::Lost;
    loop {
        let wake = (tokio::time::Instant::now() + options.debounce).min(started + options.max_wait);
        tokio::select! {
            more = hints.recv() => match more {
                Some(h) => lost |= h == Hint::Lost,
                None => return lost,
            },
            () = tokio::time::sleep_until(wake) => return lost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::MethodSpec;

    /// An app that is not the one this kit was made from: its list is called `rules`.
    static SPEC: MethodSpec = MethodSpec::bare("snapshot", "rules.snapshot");
    const SHAPE: SnapshotShape = SnapshotShape {
        call: Method::new(&SPEC),
        items_key: "rules",
        event: "rules",
        transitional: &["duplicate-id", "unreadable"],
    };

    fn cached(ids: &[(&str, &str)]) -> Cached {
        Cached {
            text: Bytes::new(),
            etag: None,
            rev: Some("rev:1".to_owned()),
            seq: 1,
            fetched: Instant::now(),
            ids: Some(
                ids.iter()
                    .map(|(i, e)| ((*i).to_owned(), Some((*e).to_owned())))
                    .collect(),
            ),
        }
    }

    /// An engine in the test that keeps what it was asked with.
    #[derive(Default)]
    struct Asked(Mutex<Vec<Option<Value>>>);

    impl Engine for Asked {
        fn call<'a>(
            &'a self,
            method: Method,
            params: Option<&'a Value>,
        ) -> crate::engine::CallFuture<'a> {
            Box::pin(async move {
                lock(&self.0).push(params.cloned());
                Ok(crate::engine::Reply {
                    kind: method.reply_kind().to_owned(),
                    text: r#"{"rules":[]}"#.to_owned(),
                })
            })
        }
    }

    #[tokio::test]
    async fn every_read_of_the_snapshot_carries_the_parameters_of_the_store() {
        let engine = Arc::new(Asked::default());
        let params = json!({ "state_dir": "/state" });
        let store = SnapshotStore::with_params(
            Some(Arc::clone(&engine) as Arc<dyn Engine>),
            SHAPE,
            StoreOptions {
                max_age: None,
                ..StoreOptions::default()
            },
            Some(params.clone()),
        );
        store.current().await.unwrap();
        store.refresh("fs").await.unwrap();
        assert_eq!(
            *lock(&engine.0),
            [Some(params.clone()), Some(params)],
            "the fetch of a request and the scan of a change"
        );

        // Without any: as ever.
        let plain = Arc::new(Asked::default());
        let store = SnapshotStore::new(
            Some(Arc::clone(&plain) as Arc<dyn Engine>),
            SHAPE,
            StoreOptions::default(),
        );
        store.current().await.unwrap();
        assert_eq!(*lock(&plain.0), [None]);
    }

    #[test]
    fn a_document_without_ids_is_not_all_removed() {
        let old = cached(&[("a/x", "1"), ("a/y", "1")]);
        let mut unreadable = cached(&[]);
        unreadable.ids = None;
        let c = diff(&old, &unreadable, "fs");
        assert!(c.truncated);
        assert!(c.changed.is_empty() && c.added.is_empty() && c.removed.is_empty());
        // And back: the first readable document after it names nothing either.
        let c = diff(&unreadable, &old, "fs");
        assert!(c.truncated);
        assert!(c.added.is_empty());
    }

    #[test]
    fn a_diff_names_changed_added_and_removed() {
        let old = cached(&[("a/x", "1"), ("a/y", "1"), ("a/z", "1")]);
        let new = cached(&[("a/x", "1"), ("a/y", "2"), ("a/w", "1")]);
        let c = diff(&old, &new, "fs");
        assert_eq!(c.changed, ["a/y"]);
        assert_eq!(c.added, ["a/w"]);
        assert_eq!(c.removed, ["a/z"]);
        assert!(!c.truncated);
        assert_eq!(c.cause, "fs");
    }

    #[test]
    fn a_big_change_is_cut_and_says_so() {
        let old = cached(&[]);
        let many: Vec<(String, String)> = (0..500)
            .map(|i| (format!("a/t{i}"), "1".to_owned()))
            .collect();
        let refs: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let new = cached(&refs);
        let c = diff(&old, &new, "fs");
        assert_eq!(c.added.len(), MAX_IDS);
        assert!(c.truncated);
    }

    #[test]
    fn the_event_is_one_sse_message_without_a_title() {
        let c = Change {
            seq: 7,
            rev: Some("rev:abc".to_owned()),
            cause: "timer",
            changed: vec!["a/b".to_owned()],
            added: vec![],
            removed: vec![],
            truncated: false,
        };
        let text = c.to_sse(SHAPE.event);
        assert!(text.starts_with("event: rules\ndata: {"));
        assert!(text.ends_with("}\n\n"));
        assert_eq!(text.matches('\n').count(), 3, "{text:?}");
        let data = text.lines().nth(1).unwrap().strip_prefix("data: ").unwrap();
        let v: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(v["seq"], 7);
        assert_eq!(v["changed_ids"], json!(["a/b"]));
    }

    #[test]
    fn a_digest_is_the_etag_only_when_it_is_plain() {
        assert!(plain_digest("sha256:0123456789abcdef"));
        for bad in ["", "a b", "a\"b", "a\r\nb", "sha256:\u{e4}"] {
            assert!(!plain_digest(bad), "{bad:?}");
        }
        assert!(!plain_digest(&"a".repeat(129)));
    }

    #[test]
    fn a_transitional_problem_is_seen() {
        let lite = |t: &str| parse_lite(t, &SHAPE).unwrap();
        let clean = r#"{"rules":[{"id":"a/b","problems":[]}]}"#;
        let dup = r#"{"rules":[{"id":"a/b","problems":[{"code":"duplicate-id"}]}]}"#;
        let broken = r#"{"rules":[{"id":"a/b","problems":[{"code":"missing-title"}]}]}"#;
        assert!(!has_transitional(&lite(clean), &SHAPE));
        assert!(has_transitional(&lite(dup), &SHAPE));
        assert!(!has_transitional(&lite(broken), &SHAPE));
        // Which codes pass by is the app's word, not the kit's.
        let only_broken = SnapshotShape {
            transitional: &["missing-title"],
            ..SHAPE
        };
        assert!(has_transitional(&lite(broken), &only_broken));
        assert!(!has_transitional(&lite(dup), &only_broken));
    }

    #[test]
    fn the_list_is_read_under_the_key_the_shape_names_and_nothing_else_is() {
        let text = r#"{"digest":"sha256:ab","rev":"r1","extra":{"deep":[1,2]},
            "rules":[{"id":"a/x","etag":"e1","problems":[]},{"id":"a/y","etag":null}],
            "tasks":[{"id":"not/read"}]}"#;
        let lite = parse_lite(text, &SHAPE).unwrap();
        assert_eq!(lite.digest.as_deref(), Some("sha256:ab"));
        assert_eq!(lite.rev.as_deref(), Some("r1"));
        let ids: Vec<&str> = lite.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["a/x", "a/y"]);
        // A document that holds no list under the key has none (the key of another app is not it).
        let other = parse_lite(r#"{"tasks":[{"id":"a/x"}]}"#, &SHAPE).unwrap();
        assert!(other.items.is_empty());
    }

    #[test]
    fn something_that_is_not_a_snapshot_is_none() {
        for bad in [
            "",
            "[]",
            "42",
            "{",
            r#"{"rules":[]} trailing"#,
            r#"{"rules":5}"#,
        ] {
            assert!(parse_lite(bad, &SHAPE).is_none(), "{bad:?}");
        }
        assert!(parse_lite(r#"{"rules":[]}"#, &SHAPE).is_some());
    }
}
