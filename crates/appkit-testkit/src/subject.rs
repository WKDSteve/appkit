//! What the security matrix needs to know about an app.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use appkit_core::authn::{ManualClock, Secret};
use appkit_core::identity::AppIdentity;
use appkit_hub::{BoxFuture, HubStatus, Limits, RunningServer};

/// How a test wants the hub started.
pub struct StartConfig {
    /// Must be loopback for the hub to start; the tests also try addresses that are not.
    pub addr: SocketAddr,
    pub limits: Limits,
    /// The clock of the pairing and of the limits: a test moves it.
    pub clock: Arc<ManualClock>,
    pub pairing_ttl: Duration,
    pub control_secret: Secret,
    pub instance: Secret,
    /// What `hello` says about the engine.
    pub status: HubStatus,
}

/// An app whose hub the matrix is run against.
pub trait Subject: Send + Sync {
    /// The names of the app: the matrix reads the paths of its routes from here.
    fn identity(&self) -> &'static AppIdentity;

    /// Start the hub of the app, with no engine and no page, on the address the test names.
    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>>;

    /// The names (below the namespace of the app) of the routes that need a session, whatever
    /// their method. The matrix sends a GET to each of them without one, and with every wrong
    /// kind of one; the guard refuses by the path before the method or the route is looked at.
    fn data_routes(&self) -> Vec<&'static str>;

    /// Whether the hub has an event-stream route named `stream` (below the namespace). The matrix then holds the
    /// stream to the end of its session; a hub without one skips that test. Default `false`.
    fn has_stream(&self) -> bool {
        false
    }

    /// Whether the app lets its page use the camera and the microphone (`GuardRules::with_page_capture`). The matrix
    /// holds the page to exactly this (`the_page_captures_as_the_subject_says`): an app that sets the switch without
    /// saying so here, or says so without setting it, is red. Most apps do not: the default is `false`.
    fn page_may_capture(&self) -> bool {
        false
    }

    /// The name of the machine in the tailnet the hub takes (`GuardRules::with_tailnet_name`), if it takes one. The
    /// matrix then asks the page through that `Host` too: it carries the same permissions there.
    fn tailnet_name(&self) -> Option<&'static str> {
        None
    }
}
