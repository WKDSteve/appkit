//! Building blocks of the local app server, free of any task or Tauri knowledge.
//!
//! The boundary is held from day one (concept 9.6): this crate knows neither
//! tasks nor the shell, so it can move into its own repository when a second
//! app needs the same hardening. Modules arrive with the phase that needs
//! them: `proc` (child processes), `authn` (secrets, host and origin rules,
//! pairing, sessions, rate limit) and `registry` (the discovery file) now;
//! `sse` and `pathguard` follow.

#[cfg(windows)]
mod acl;
pub mod authn;
pub mod child;
pub mod classes;
pub mod code_ref;
pub mod console;
pub mod family;
#[cfg(windows)]
pub mod handles;
pub mod identity;
pub mod job;
pub mod launch;
pub mod proc;
pub mod registry;
