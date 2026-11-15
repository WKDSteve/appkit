//! Test support for the hubs built on `appkit-hub`. It knows no app: an app brings what is its own
//! (an implementation of [`subject::Subject`], a [`dialect::FakeDialect`] and its documents).
//!
//! * [`wire`]: requests written as raw bytes over a TCP socket, because a real HTTP client would
//!   "repair" the very things under test (the `Host` header, a header that is too long, a type).
//! * [`subject`]: what the matrix needs to know about an app: its names, how to start its hub,
//!   and the routes that need a session.
//! * [`security`]: the security tests of the guard (concept 5.5), written once and run for every
//!   app with [`security_matrix!`].
//! * [`subjects`]: `Demo`, a second, minimal app that is built on `appkit-hub` alone and so proves
//!   that the guard knows no app.
//! * [`ports`]: the default ports of the apps that run side by side; no two share one.
//! * [`dialect`] and [`fake_engine`]: a stand-in for the engine of an app, to test the seam of the
//!   engine and the routes without Neovim.
//! * [`window_program`]: whether a program of Windows is a window program, read from its header
//!   (the test of every hub's release build).

pub mod dialect;
pub mod fake_engine;
pub mod ports;
pub mod security;
pub mod subject;
pub mod subjects;
pub mod window_program;
pub mod wire;
