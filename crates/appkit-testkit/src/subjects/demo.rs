//! A second app, built on `appkit-hub` alone: it has its own identity, its own namespace and its own
//! routes, and it knows nothing of `tasks-hub`. That the matrix passes for it is the proof that the
//! guard, the pairing and the limits do not depend on the app.

use std::sync::Arc;

use appkit_core::identity::AppIdentity;
use appkit_hub::{
    serve, BoxFuture, Core, GuardRules, HubStatus, Page, Pages, RunningServer, Runtime, ServeParts,
};
use axum::routing::{get, post};
use axum::Router;

use crate::subject::{StartConfig, Subject};

/// The names of the second app. Another config folder, another `Run` value, another port, another
/// namespace than the tasks hub: nothing here is shared.
pub const IDENTITY: AppIdentity = AppIdentity {
    app: "demo-desktop",
    binary: "demo-hub",
    env_prefix: "DEMO_DESKTOP",
    title: "Demo",
    default_port: 17617,
    route_ns: "demo",
};

/// What the hub of the app keeps for its vault: nothing but the status `hello` reports.
struct DemoRuntime {
    status: HubStatus,
}

impl Runtime for DemoRuntime {
    type Target = ();

    fn status(&self) -> &HubStatus {
        &self.status
    }

    fn stop(&self) {}
}

/// No page: the placeholder of the core is served.
struct NoPages;

impl Pages for NoPages {
    fn page(&self, _request_path: &str) -> Option<Page> {
        None
    }

    fn has_index(&self) -> bool {
        false
    }
}

/// The routes of the app: two reads and one write, under `/api/demo/`.
fn routes(_core: &Arc<Core<DemoRuntime>>) -> Router<Arc<Core<DemoRuntime>>> {
    Router::new()
        .route(&IDENTITY.route("items"), get(|| async { "[]" }))
        .route(&IDENTITY.route("item"), get(|| async { "{}" }))
        .route(&IDENTITY.route("ops"), post(|| async { "{}" }))
}

/// The hub of the second app, with no engine and no page.
pub struct Demo;

impl Subject for Demo {
    fn identity(&self) -> &'static AppIdentity {
        &IDENTITY
    }

    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>> {
        Box::pin(async move {
            serve(
                ServeParts {
                    addr: cfg.addr,
                    limits: cfg.limits,
                    clock: cfg.clock,
                    pairing_ttl: cfg.pairing_ttl,
                    control_secret: cfg.control_secret,
                    instance: cfg.instance,
                    app: IDENTITY.app,
                    version: "0.0.0",
                    build_id: None,
                    guard_rules: GuardRules::new(IDENTITY.route("hello"), [IDENTITY.route("ops")]),
                    ui: Arc::new(NoPages),
                    runtime: DemoRuntime { status: cfg.status },
                    build: Arc::new(|()| DemoRuntime {
                        status: HubStatus {
                            engine_ready: false,
                            problem: None,
                            capabilities: Vec::new(),
                        },
                    }),
                    vaults: None,
                },
                routes,
            )
            .await
        })
    }

    fn data_routes(&self) -> Vec<&'static str> {
        vec!["items", "item", "ops"]
    }
}
