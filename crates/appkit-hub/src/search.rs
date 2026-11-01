//! Full-text search over the bodies of the documents (the tasks, the rules).
//!
//! The engine alone reads the files (concept 3.2), so the text comes from
//! it: the call the app names (`bodies`) hands over `{ id, etag, body }` of the open documents.
//! The hub keeps that text in memory and searches it there, which is what
//! lets a query answer in milliseconds instead of one Neovim process each.
//! Nothing here parses a frontmatter or opens a file.
//!
//! The index is built in the background (a moment after the hub starts) and
//! follows the changes the store reports: the ids that changed are asked for
//! again, a big change or a lost event builds it anew.
//!
//! Matching folds case one character to one character, so that a position in
//! the folded text is the same position in the text that is shown.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use tokio::task::JoinHandle;

use crate::engine::{Engine, Method};
use crate::store::{Change, SnapshotStore};

/// Documents asked for in one call (the engine ends a page earlier by bytes).
const PAGE: u64 = 1000;
/// Pages read before the index gives up (a loop that does not end).
const MAX_PAGES: usize = 50;
/// More changed documents than this are not asked for one by one: the index is
/// built again.
const INCREMENTAL_MAX: usize = 50;
/// Tries when the vault changes while the pages are read.
const BUILD_TRIES: usize = 3;
/// The most text the index holds (the bodies, once; the folded copy is as
/// large again). Beyond it the rest is not indexed and the answer says so.
pub const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
/// Pages followed for the named ids of a change.
const MAX_ID_PAGES: usize = 20;
/// The wait before the first build, so that the first screen is not queued
/// behind it.
const FIRST_BUILD_DELAY: Duration = Duration::from_secs(2);

pub const MAX_QUERY_CHARS: usize = 200;
pub const MAX_TOKENS: usize = 8;
pub const MIN_QUERY_CHARS: usize = 2;
pub const MAX_HITS: usize = 50;
/// How much text before and after the first match a snippet carries.
const BEFORE: usize = 60;
const AFTER: usize = 100;

#[derive(Deserialize)]
struct Page {
    rev: Option<String>,
    #[serde(default)]
    items: Vec<Item>,
    next_offset: Option<u64>,
}

#[derive(Deserialize)]
struct Item {
    id: String,
    etag: Option<String>,
    #[serde(default)]
    body: String,
}

struct Entry {
    id: String,
    etag: Option<String>,
    body: String,
    folded: String,
}

impl Entry {
    fn new(item: Item) -> Self {
        let folded = fold(&item.body);
        Self {
            id: item.id,
            etag: item.etag,
            body: item.body,
            folded,
        }
    }
}

struct Corpus {
    rev: Option<String>,
    entries: Vec<Entry>,
    /// Not every text is in: the limit was reached.
    partial: bool,
}

/// Lowercase, one character for one (the first of its lowercase form).
#[must_use]
pub fn fold(text: &str) -> String {
    text.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

/// A query, read: folded words, all of which must be in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub tokens: Vec<String>,
    /// The words in their order, joined by one space: a text that holds it
    /// whole ranks higher.
    pub phrase: String,
}

impl Query {
    /// # Errors
    ///
    /// A word of why the query is not one.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        if text.chars().any(char::is_control) {
            return Err("the query holds a control character");
        }
        let folded = fold(text);
        let mut tokens: Vec<String> = Vec::new();
        for word in folded.split_whitespace() {
            if !tokens.iter().any(|t| t == word) {
                tokens.push(word.to_owned());
            }
        }
        let chars: usize = tokens.iter().map(|t| t.chars().count()).sum();
        if tokens.is_empty() || chars < MIN_QUERY_CHARS {
            return Err("the query is too short");
        }
        if text.chars().count() > MAX_QUERY_CHARS {
            return Err("the query is too long");
        }
        if tokens.len() > MAX_TOKENS {
            return Err("the query has too many words");
        }
        let phrase = folded.split_whitespace().collect::<Vec<_>>().join(" ");
        Ok(Self { tokens, phrase })
    }
}

/// A piece of a body around a match. `marks` are `[start, end)` in UTF-16
/// code units of `text`, which is what a script indexes by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    pub marks: Vec<[u32; 2]>,
    pub more_before: bool,
    pub more_after: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub score: u32,
    pub snippet: Snippet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub total: usize,
    pub hits: Vec<Hit>,
}

fn matches_in(folded: &str, query: &Query) -> Option<u32> {
    let mut score = 0u32;
    for token in &query.tokens {
        let count = folded.matches(token.as_str()).take(5).count();
        if count == 0 {
            return None;
        }
        score += u32::try_from(count).unwrap_or(5);
    }
    if query.tokens.len() > 1 && folded.contains(query.phrase.as_str()) {
        score += 10;
    }
    Some(score)
}

/// Every `[start, end)` of a token in `folded`, in characters, ordered and
/// without overlap.
fn token_ranges(folded: &str, tokens: &[String], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut found: Vec<(usize, usize)> = Vec::new();
    let chars: Vec<(usize, char)> = folded.char_indices().collect();
    // Byte offsets of the window.
    let window_from = chars.get(from).map_or(folded.len(), |c| c.0);
    let window_to = chars.get(to).map_or(folded.len(), |c| c.0);
    let window = &folded[window_from..window_to];
    for token in tokens {
        let token_chars = token.chars().count();
        for (at, _) in window.match_indices(token.as_str()) {
            let start = from + window[..at].chars().count();
            found.push((start, start + token_chars));
        }
    }
    found.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in found {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

fn snippet(entry: &Entry, query: &Query) -> Snippet {
    let folded_chars = entry.folded.chars().count();
    // The first place the whole phrase is, else the first token.
    let anchor_text = if query.tokens.len() > 1 && entry.folded.contains(query.phrase.as_str()) {
        query.phrase.as_str()
    } else {
        query.tokens.first().map_or("", String::as_str)
    };
    let anchor_chars = entry
        .folded
        .find(anchor_text)
        .map_or(0, |at| entry.folded[..at].chars().count());
    let start = anchor_chars.saturating_sub(BEFORE);
    let end = (anchor_chars + anchor_text.chars().count() + AFTER).min(folded_chars);
    let body_chars: Vec<char> = entry.body.chars().collect();
    let text: String = body_chars[start.min(body_chars.len())..end.min(body_chars.len())]
        .iter()
        .map(|c| if c.is_control() { ' ' } else { *c })
        .collect();
    // UTF-16 offsets of every character of the window.
    let mut units: Vec<u32> = Vec::with_capacity(end - start + 1);
    let mut at = 0u32;
    for c in text.chars() {
        units.push(at);
        at += u32::try_from(c.len_utf16()).unwrap_or(1);
    }
    units.push(at);
    let marks = token_ranges(&entry.folded, &query.tokens, start, end)
        .into_iter()
        .filter_map(|(s, e)| {
            let (s, e) = (s - start, e - start);
            Some([*units.get(s)?, *units.get(e)?])
        })
        .collect();
    Snippet {
        text,
        marks,
        more_before: start > 0,
        more_after: end < folded_chars,
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How the index is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing was read yet, or it is being read.
    Building,
    Ready {
        /// How many documents are in the index.
        items: usize,
        partial: bool,
    },
    /// The last build failed; the next change tries again.
    Failed,
}

struct State {
    corpus: Option<Arc<Corpus>>,
    building: bool,
    failed: bool,
}

pub struct SearchIndex {
    engine: Arc<dyn Engine>,
    /// The call that hands over the bodies, in pages.
    bodies: Method,
    max_bytes: usize,
    state: Mutex<State>,
}

impl SearchIndex {
    #[must_use]
    pub fn new(engine: Arc<dyn Engine>, bodies: Method) -> Arc<Self> {
        Self::with_limit(engine, bodies, MAX_INDEX_BYTES)
    }

    /// An index that holds at most `max_bytes` of text.
    #[must_use]
    pub fn with_limit(engine: Arc<dyn Engine>, bodies: Method, max_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            engine,
            bodies,
            max_bytes,
            state: Mutex::new(State {
                corpus: None,
                building: false,
                failed: false,
            }),
        })
    }

    #[must_use]
    pub fn status(&self) -> Status {
        let state = lock(&self.state);
        match (&state.corpus, state.failed) {
            (Some(c), _) => Status::Ready {
                items: c.entries.len(),
                partial: c.partial,
            },
            (None, true) => Status::Failed,
            (None, false) => Status::Building,
        }
    }

    /// The hits for a query; `None` while there is no index.
    #[must_use]
    pub fn search(&self, query: &Query, limit: usize) -> Option<Outcome> {
        let corpus = lock(&self.state).corpus.clone()?;
        let mut scored: Vec<(u32, usize)> = corpus
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| matches_in(&e.folded, query).map(|s| (s, i)))
            .collect();
        // Better first; among equals the plan order the engine gave.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let total = scored.len();
        let hits = scored
            .into_iter()
            .take(limit.clamp(1, MAX_HITS))
            .filter_map(|(score, i)| {
                let entry = corpus.entries.get(i)?;
                Some(Hit {
                    id: entry.id.clone(),
                    score,
                    snippet: snippet(entry, query),
                })
            })
            .collect();
        Some(Outcome { total, hits })
    }

    async fn page(&self, params: &serde_json::Value) -> Result<Page, String> {
        let reply = self
            .engine
            .call(self.bodies, Some(params))
            .await
            .map_err(|e| e.to_string())?;
        serde_json::from_str::<Page>(&reply.text).map_err(|e| format!("a bodies page: {e}"))
    }

    /// Read every body and replace the index.
    ///
    /// # Errors
    ///
    /// What the engine said, or that the vault kept changing.
    pub async fn build(&self) -> Result<(), String> {
        {
            let mut state = lock(&self.state);
            if state.building {
                return Ok(());
            }
            state.building = true;
        }
        let result = self.read_all().await;
        let mut state = lock(&self.state);
        state.building = false;
        match result {
            Ok(corpus) => {
                state.corpus = Some(Arc::new(corpus));
                state.failed = false;
                Ok(())
            }
            Err(e) => {
                state.failed = state.failed || state.corpus.is_none();
                Err(e)
            }
        }
    }

    async fn read_all(&self) -> Result<Corpus, String> {
        'tries: for _ in 0..BUILD_TRIES {
            let mut entries: Vec<Entry> = Vec::new();
            let mut rev: Option<String> = None;
            let mut offset = 0u64;
            let mut bytes = 0usize;
            for _ in 0..MAX_PAGES {
                let page = self
                    .page(&json!({ "offset": offset, "limit": PAGE }))
                    .await?;
                if !entries.is_empty() && page.rev != rev {
                    // The vault changed under the reading.
                    continue 'tries;
                }
                rev = page.rev;
                for item in page.items {
                    bytes += item.body.len();
                    if bytes > self.max_bytes {
                        return Ok(Corpus {
                            rev,
                            entries,
                            partial: true,
                        });
                    }
                    entries.push(Entry::new(item));
                }
                match page.next_offset {
                    Some(next) if next > offset => offset = next,
                    _ => {
                        return Ok(Corpus {
                            rev,
                            entries,
                            partial: false,
                        })
                    }
                }
            }
            return Err("the bodies did not end".to_owned());
        }
        Err("the vault kept changing while it was read".to_owned())
    }

    /// Follow a change the store reported.
    pub async fn apply(&self, change: &Change) {
        let wanted: Vec<&String> = change.changed.iter().chain(change.added.iter()).collect();
        let (current, failed) = {
            let state = lock(&self.state);
            (state.corpus.clone(), state.failed)
        };
        // The text of this very state is in the index already (the first
        // snapshot of the hub says so after the index was built before it).
        // Not after a failed read: the ids of that change were never read.
        if !failed
            && current
                .as_ref()
                .zip(change.rev.as_ref())
                .is_some_and(|(c, rev)| c.rev.as_ref() == Some(rev))
        {
            return;
        }
        let rebuild =
            failed || current.is_none() || change.truncated || wanted.len() > INCREMENTAL_MAX;
        if rebuild {
            if let Err(e) = self.build().await {
                eprintln!("search index: {e}");
            }
            return;
        }
        let Some(current) = current else { return };
        if wanted.is_empty() && change.removed.is_empty() {
            return;
        }
        // The named ids, a page at a time: a page ends at the bytes it may
        // hold, and what it names beyond that comes on the next one.
        let mut fetched = Page {
            rev: current.rev.clone(),
            items: Vec::new(),
            next_offset: None,
        };
        if !wanted.is_empty() {
            let mut offset = 0u64;
            for _ in 0..MAX_ID_PAGES {
                match self
                    .page(&json!({ "ids": wanted, "offset": offset, "limit": PAGE }))
                    .await
                {
                    Ok(p) => {
                        fetched.rev = p.rev;
                        fetched.items.extend(p.items);
                        match p.next_offset {
                            Some(next) if next > offset => offset = next,
                            _ => break,
                        }
                    }
                    Err(e) => {
                        eprintln!("search index: {e}");
                        // The ids of this change keep their old text; the
                        // next change builds the whole index again.
                        lock(&self.state).failed = true;
                        return;
                    }
                }
            }
            // A page of a later state than the change: the ids of the
            // changes in between were not read. Build from one state.
            if change.rev.is_some() && fetched.rev.is_some() && fetched.rev != change.rev {
                if let Err(e) = self.build().await {
                    eprintln!("search index: {e}");
                }
                return;
            }
        }
        let page = fetched;
        let gone = |id: &str| {
            change.removed.iter().any(|r| r == id) || change.changed.iter().any(|c| c == id)
        };
        let mut entries: Vec<Entry> = Vec::with_capacity(current.entries.len());
        for e in &current.entries {
            if !gone(&e.id) {
                entries.push(Entry {
                    id: e.id.clone(),
                    etag: e.etag.clone(),
                    body: e.body.clone(),
                    folded: e.folded.clone(),
                });
            }
        }
        let mut bytes: usize = entries.iter().map(|e| e.body.len()).sum();
        let mut partial = current.partial;
        for item in page.items {
            bytes += item.body.len();
            if bytes > self.max_bytes {
                partial = true;
                continue;
            }
            entries.push(Entry::new(item));
        }
        let mut state = lock(&self.state);
        state.corpus = Some(Arc::new(Corpus {
            rev: page.rev,
            entries,
            partial,
        }));
        state.failed = false;
    }
}

/// Run the index for the life of the hub: a first build after a moment, then
/// what the store reports.
pub fn spawn_indexer(index: Arc<SearchIndex>, store: &Arc<SnapshotStore>) -> JoinHandle<()> {
    let mut changes = store.subscribe_quiet();
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_BUILD_DELAY).await;
        if let Err(e) = index.build().await {
            eprintln!("search index: {e}");
        }
        loop {
            match changes.recv().await {
                Ok(change) => index.apply(&change).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    if let Err(e) = index.build().await {
                        eprintln!("search index: {e}");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}

/// The text of the `q` of a request, percent-decoded, and `limit`.
///
/// # Errors
///
/// A word of what is wrong with the query string.
pub fn parse_request(raw: Option<&str>) -> Result<(String, usize), &'static str> {
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Err("q is needed");
    };
    let mut q: Option<String> = None;
    let mut limit = 20usize;
    for pair in raw.split('&') {
        let (key, value) = pair.split_once('=').ok_or("a query part has no value")?;
        // What `encodeURIComponent` leaves alone, and the escapes. Nothing
        // else is on the line.
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"%-_.!~*'()".contains(&b))
        {
            return Err("a query value is not escaped text");
        }
        match key {
            "q" => {
                if q.is_some() {
                    return Err("q is given twice");
                }
                q = Some(percent_decode(value)?);
            }
            "limit" => {
                limit = value
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=MAX_HITS).contains(n))
                    .ok_or("limit is 1 to 50")?;
            }
            _ => return Err("unknown query parameter"),
        }
    }
    q.map(|q| (q, limit)).ok_or("q is needed")
}

fn percent_decode(value: &str) -> Result<String, &'static str> {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or("a broken escape")?;
            let text = std::str::from_utf8(hex).map_err(|_| "a broken escape")?;
            out.push(u8::from_str_radix(text, 16).map_err(|_| "a broken escape")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "the query is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::MethodSpec;

    /// An app that is not the one this kit was made from: its call is rules.bodies.
    static SPEC: MethodSpec = MethodSpec::read("bodies", "rules.bodies");
    const BODIES: Method = Method::new(&SPEC);

    fn entry(id: &str, body: &str) -> Entry {
        Entry::new(Item {
            id: id.to_owned(),
            etag: None,
            body: body.to_owned(),
        })
    }

    fn corpus(entries: Vec<Entry>) -> Arc<Corpus> {
        Arc::new(Corpus {
            rev: None,
            entries,
            partial: false,
        })
    }

    fn run(entries: Vec<Entry>, q: &str) -> Outcome {
        let index = SearchIndex::new(Arc::new(NoEngine), BODIES);
        lock(&index.state).corpus = Some(corpus(entries));
        index
            .search(&Query::parse(q).unwrap(), 20)
            .expect("an index")
    }

    struct NoEngine;
    impl Engine for NoEngine {
        fn call<'a>(
            &'a self,
            _method: Method,
            _params: Option<&'a serde_json::Value>,
        ) -> crate::engine::CallFuture<'a> {
            Box::pin(async { Err(crate::engine::EngineError::Spawn("no engine".to_owned())) })
        }
    }

    /// Answers each call with the next of the given documents.
    struct Scripted(Mutex<Vec<String>>);
    impl Engine for Scripted {
        fn call<'a>(
            &'a self,
            _method: Method,
            _params: Option<&'a serde_json::Value>,
        ) -> crate::engine::CallFuture<'a> {
            let text = {
                let mut left = lock(&self.0);
                if left.is_empty() {
                    String::from("{}")
                } else {
                    left.remove(0)
                }
            };
            Box::pin(async move {
                Ok(crate::engine::Reply {
                    kind: "rules.bodies".to_owned(),
                    text,
                })
            })
        }
    }

    fn page(rev: &str, ids: &[(&str, usize)], next: Option<u64>) -> String {
        let items: Vec<serde_json::Value> = ids
            .iter()
            .map(|(id, size)| json!({ "id": id, "etag": "sha256:0", "body": "x".repeat(*size) }))
            .collect();
        json!({ "rev": rev, "items": items, "next_offset": next }).to_string()
    }

    /// Writes down the names of the calls it is asked, and answers with an empty page.
    struct Recording(Mutex<Vec<&'static str>>);
    impl Engine for Recording {
        fn call<'a>(
            &'a self,
            method: Method,
            _params: Option<&'a serde_json::Value>,
        ) -> crate::engine::CallFuture<'a> {
            lock(&self.0).push(method.as_str());
            Box::pin(async {
                Ok(crate::engine::Reply {
                    kind: "rules.bodies".to_owned(),
                    text: "{}".to_owned(),
                })
            })
        }
    }

    #[tokio::test]
    async fn the_index_asks_with_the_call_the_app_gave_and_no_other() {
        static OTHER: MethodSpec = MethodSpec::read("their_bodies", "other.bodies");
        let engine = Arc::new(Recording(Mutex::new(Vec::new())));
        let index = SearchIndex::new(engine.clone(), Method::new(&OTHER));
        index.build().await.unwrap();
        assert_eq!(*lock(&engine.0), ["their_bodies"]);
        let engine = Arc::new(Recording(Mutex::new(Vec::new())));
        SearchIndex::new(engine.clone(), BODIES)
            .build()
            .await
            .unwrap();
        assert_eq!(*lock(&engine.0), ["bodies"]);
    }

    #[tokio::test]
    async fn pages_are_followed_and_one_state_is_required() {
        let engine = Arc::new(Scripted(Mutex::new(vec![
            page("rev:1", &[("a/one", 3), ("a/two", 3)], Some(2)),
            page("rev:1", &[("a/three", 3)], None),
        ])));
        let index = SearchIndex::new(engine, BODIES);
        index.build().await.unwrap();
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 3,
                partial: false
            }
        );
        // The vault changed between two pages: the reading starts over.
        let engine = Arc::new(Scripted(Mutex::new(vec![
            page("rev:1", &[("a/one", 3)], Some(1)),
            page("rev:2", &[("a/two", 3)], None),
            page("rev:2", &[("a/one", 3), ("a/two", 3)], None),
        ])));
        let index = SearchIndex::new(engine, BODIES);
        index.build().await.unwrap();
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 2,
                partial: false
            }
        );
    }

    #[tokio::test]
    async fn text_beyond_the_limit_is_left_out_and_said() {
        let engine = Arc::new(Scripted(Mutex::new(vec![page(
            "rev:1",
            &[("a/one", 100), ("a/two", 100), ("a/three", 100)],
            None,
        )])));
        let index = SearchIndex::with_limit(engine, BODIES, 250);
        index.build().await.unwrap();
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 2,
                partial: true
            }
        );
    }

    #[tokio::test]
    async fn the_named_ids_of_a_change_are_read_page_after_page() {
        let engine = Arc::new(Scripted(Mutex::new(vec![
            page("rev:1", &[("a/one", 3)], None),
            // Two pages for the ids that changed.
            page("rev:2", &[("a/two", 3)], Some(1)),
            page("rev:2", &[("a/three", 3)], None),
        ])));
        let index = SearchIndex::new(engine, BODIES);
        index.build().await.unwrap();
        let change = Change {
            seq: 2,
            rev: Some("rev:2".to_owned()),
            cause: "fs",
            changed: vec![],
            added: vec!["a/two".to_owned(), "a/three".to_owned()],
            removed: vec![],
            truncated: false,
        };
        index.apply(&change).await;
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 3,
                partial: false
            }
        );
    }

    fn change(seq: u64, rev: &str, added: &[&str]) -> Change {
        Change {
            seq,
            rev: Some(rev.to_owned()),
            cause: "fs",
            changed: vec![],
            added: added.iter().map(|s| (*s).to_owned()).collect(),
            removed: vec![],
            truncated: false,
        }
    }

    #[tokio::test]
    async fn a_page_of_a_later_state_than_the_change_builds_the_index_again() {
        let engine = Arc::new(Scripted(Mutex::new(vec![
            page("rev:1", &[("a/one", 3)], None),
            // Two saves within one delay: the page for the first change is
            // already the state of the second.
            page("rev:3", &[("a/two", 3)], None),
            page("rev:3", &[("a/one", 3), ("a/two", 3), ("a/four", 3)], None),
        ])));
        let index = SearchIndex::new(engine, BODIES);
        index.build().await.unwrap();
        index.apply(&change(2, "rev:2", &["a/two"])).await;
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 3,
                partial: false
            }
        );
        // The second change finds its state in the index and returns.
        index.apply(&change(3, "rev:3", &["a/four"])).await;
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 3,
                partial: false
            }
        );
    }

    #[tokio::test]
    async fn a_page_that_fails_is_made_up_by_a_build_on_the_next_change() {
        let engine = Arc::new(Scripted(Mutex::new(vec![
            page("rev:1", &[("a/one", 3)], None),
            "not a page".to_owned(),
            page("rev:2", &[("a/one", 3), ("a/two", 3)], None),
        ])));
        let index = SearchIndex::new(engine, BODIES);
        index.build().await.unwrap();
        index.apply(&change(2, "rev:2", &["a/two"])).await;
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 1,
                partial: false
            }
        );
        // Even a change of the state the index carries is no reason to skip.
        index.apply(&change(3, "rev:1", &["a/three"])).await;
        assert_eq!(
            index.status(),
            Status::Ready {
                items: 2,
                partial: false
            }
        );
    }

    #[test]
    fn a_query_is_words_and_limits() {
        let q = Query::parse("  Größe   TASKS  ").unwrap();
        assert_eq!(q.tokens, ["größe", "tasks"]);
        assert_eq!(q.phrase, "größe tasks");
        for bad in [
            "",
            " ",
            "a",
            "ab\ncd",
            &"x".repeat(201),
            "a b c d e f g h i",
        ] {
            assert!(Query::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Query::parse("ab").is_ok());
        // The same word twice is one word.
        assert_eq!(Query::parse("foo FOO").unwrap().tokens, ["foo"]);
    }

    #[test]
    fn every_word_must_be_there_and_a_phrase_ranks_first() {
        let out = run(
            vec![
                entry("a/one", "the quick brown fox"),
                entry("a/two", "brown dogs, quick cats"),
                entry("a/three", "only quick"),
            ],
            "quick brown",
        );
        assert_eq!(out.total, 2);
        assert_eq!(out.hits[0].id, "a/one", "the phrase whole ranks first");
        assert_eq!(out.hits[1].id, "a/two");
    }

    #[test]
    fn case_is_not_a_difference_and_positions_are_the_texts() {
        let out = run(vec![entry("a/x", "Ärger mit ÖL und STRASSE")], "öl strasse");
        assert_eq!(out.total, 1);
        let s = &out.hits[0].snippet;
        let units: Vec<u16> = s.text.encode_utf16().collect();
        let marked: Vec<String> = s
            .marks
            .iter()
            .map(|[a, b]| String::from_utf16(&units[*a as usize..*b as usize]).unwrap())
            .collect();
        assert_eq!(marked, ["ÖL", "STRASSE"]);
    }

    #[test]
    fn marks_count_in_utf16_units_after_a_character_outside_the_plane() {
        let out = run(vec![entry("a/x", "😀😀 needle here")], "needle");
        let s = &out.hits[0].snippet;
        // Each emoji is two units; a script indexes by units.
        assert_eq!(s.marks, vec![[5, 11]]);
        let units: Vec<u16> = s.text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[5..11]).unwrap(), "needle");
    }

    #[test]
    fn a_snippet_is_a_window_and_says_there_is_more() {
        let long = format!("{} NEEDLE {}", "a".repeat(300), "b".repeat(300));
        let out = run(vec![entry("a/x", &long)], "needle");
        let s = &out.hits[0].snippet;
        assert!(s.more_before && s.more_after);
        assert!(s.text.chars().count() <= BEFORE + 6 + AFTER);
        assert_eq!(s.marks.len(), 1);
        let short = run(vec![entry("a/y", "needle")], "needle");
        assert!(!short.hits[0].snippet.more_before && !short.hits[0].snippet.more_after);
    }

    #[test]
    fn control_characters_in_a_snippet_are_blanks() {
        let out = run(
            vec![entry("a/x", "line one\nneedle\u{1b}[31m two\r\n")],
            "needle",
        );
        let s = &out.hits[0].snippet;
        assert!(!s.text.chars().any(char::is_control), "{:?}", s.text);
    }

    #[test]
    fn nothing_matches_nothing() {
        let out = run(vec![entry("a/x", "alpha")], "zzz");
        assert_eq!(out.total, 0);
        assert!(out.hits.is_empty());
    }

    #[test]
    fn the_request_is_escaped_text_and_a_limit() {
        assert_eq!(
            parse_request(Some("q=gr%C3%B6%C3%9Fe%20task&limit=5")).unwrap(),
            ("größe task".to_owned(), 5)
        );
        assert_eq!(
            parse_request(Some("q=abc")).unwrap(),
            ("abc".to_owned(), 20)
        );
        for bad in [
            "",
            "limit=5",
            "q=a&q=b",
            "q=%ZZ",
            "q=%E4",
            "q=a b",
            "q=a+b",
            "q=a&x=1",
            "q=a&limit=0",
            "q=a&limit=51",
            "q=a;b",
            "q=a<b",
        ] {
            assert!(parse_request(Some(bad)).is_err(), "{bad:?}");
        }
        assert!(parse_request(None).is_err());
    }
}
