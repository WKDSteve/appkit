//! The local hub of an app, without the app (concept 5.5).
//!
//! Loopback only, every request checked before any route sees it. The order inside [`guard`] is
//! the contract, and each step has a test in `hub-testkit/tests/security.rs`:
//!
//! 1. `Host` is one of our three loopback names (DNS rebinding);
//! 2. a declared body over the limit is refused unread;
//! 3. `Origin` (if sent) and `Sec-Fetch-Site` (if sent) are ours;
//! 4. the method is one the route knows;
//! 5. a mutation without `Origin` is a program client: it needs a bearer
//!    credential and must not look like a browser;
//! 6. the rate limit for mutations (the routes an app names as coarse have a limit of their own,
//!    and the handler counts their writes again: only the body tells a write from a dry run);
//! 7. the credential the route needs (none, the control secret, a session);
//! 8. a mutation carries `application/json`.
//!
//! What an app brings is its routes, a [`Runtime`] for the vault that is read, and the paths it
//! names in [`GuardRules`]. It cannot add, loosen or reorder a step. A route that is added
//! under `/api/` is closed to anything without a session until it is deliberately opened.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;

use appkit_core::authn::{
    fetch_site_ok, Clock, HostPolicy, PairingStore, RateLimiter, Secret, SessionStore, CODE_BYTES,
    TOKEN_BYTES,
};
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, RawQuery, Request, State};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinHandle;

pub mod access;
pub mod background;
pub mod browser;
pub mod client;
pub mod config;
pub mod devices;
pub mod discovery;
pub mod editor;
pub mod engine;
pub mod env;
pub mod fs_watch;
pub mod guard;
#[cfg(windows)]
mod handles;
pub mod http;
pub mod hub;
pub mod instance;
pub mod launch;
pub mod limits;
pub mod loginitems;
pub mod search;
pub mod setup;
pub mod shell_link;
pub mod store;
pub mod streams;
pub mod usage;
pub mod vaults;

pub use self::guard::{bearer, error_body, single, too_many, GuardRules};
pub use self::http::{etag_matches, json_response, no_query, not_modified, parse_query};
pub use self::hub::{serve, BuildRuntime, Core, Page, Pages, RunningServer, Runtime, ServeParts};
pub use self::limits::{HubStatus, Limits};
pub use self::vaults::{
    vault_request, vault_text, vaults_unavailable, BoxFuture, VaultAdmin, VaultInfo, VaultList,
};
pub use appkit_core::classes::{Caller, DataClass};

use self::access::*;
use self::guard::*;
use self::vaults::*;
