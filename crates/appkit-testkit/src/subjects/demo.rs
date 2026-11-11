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

/// The same hub, whose page may also fetch from one other `https` host (`GuardRules::with_connect_src`).
pub struct DemoWide;

/// The same hub with a desktop page and an installable page under `/m` (`GuardRules::with_installable_page`).
pub struct DemoInstallable;

/// The same, whose installable page may record from the microphone (`GuardRules::with_installable_microphone`).
pub struct DemoInstallableRecording;

/// The prefix and the worker policy `DemoInstallable` names.
pub const INSTALLABLE_PREFIX: &str = "/m";
pub const INSTALLABLE_WORKER_POLICY: &str = "demo-worker";

/// A desktop page at `/` and an installable one at `/m/`, with its worker and manifest.
struct TwoPages;

impl Pages for TwoPages {
    fn page(&self, request_path: &str) -> Option<Page> {
        let (bytes, content_type): (&'static [u8], &'static str) = match request_path {
            "/" | "/index.html" => (
                b"<!doctype html><title>desk</title>",
                "text/html; charset=utf-8",
            ),
            "/m/" => (
                b"<!doctype html><title>phone</title>",
                "text/html; charset=utf-8",
            ),
            "/m/sw.js" => (
                b"self.addEventListener('fetch', () => {});",
                "text/javascript; charset=utf-8",
            ),
            "/m/manifest.webmanifest" => (b"{}", "application/manifest+json"),
            _ => return None,
        };
        Some(Page {
            bytes: axum::body::Bytes::from_static(bytes),
            content_type,
        })
    }

    fn has_index(&self) -> bool {
        true
    }
}

/// The host `DemoWide` names.
pub const WIDE_ORIGIN: &str = "https://api.example.org";

/// The name of the machine in the tailnet of `DemoDevices` (`GuardRules::with_tailnet_name`).
pub const TAILNET_NAME: &str = "pc.example.ts.net";

fn rules() -> GuardRules {
    GuardRules::new(IDENTITY.route("hello"), [IDENTITY.route("ops")])
}

fn start_with(
    cfg: StartConfig,
    guard_rules: GuardRules,
) -> BoxFuture<'static, std::io::Result<RunningServer>> {
    start_with_pages(cfg, guard_rules, Arc::new(NoPages))
}

fn start_with_pages(
    cfg: StartConfig,
    guard_rules: GuardRules,
    ui: Arc<dyn Pages>,
) -> BoxFuture<'static, std::io::Result<RunningServer>> {
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
                guard_rules,
                ui,
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

impl Subject for Demo {
    fn identity(&self) -> &'static AppIdentity {
        &IDENTITY
    }

    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>> {
        start_with(cfg, rules())
    }

    fn data_routes(&self) -> Vec<&'static str> {
        vec!["items", "item", "ops"]
    }
}

impl Subject for DemoWide {
    fn identity(&self) -> &'static AppIdentity {
        &IDENTITY
    }

    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>> {
        start_with(cfg, rules().with_connect_src(WIDE_ORIGIN))
    }

    fn data_routes(&self) -> Vec<&'static str> {
        vec!["items", "item", "ops"]
    }
}

/// The hub with an installable page and paired devices (`GuardRules::with_devices`), reached also through the
/// tailnet as [`TAILNET_NAME`], its list of devices in a folder of
/// its own that is removed when the value is dropped.
pub struct DemoDevices {
    dir: appkit_scratch::Scratch,
}

impl DemoDevices {
    /// A new folder for the list of devices (a scratch folder, removed when the value is dropped, also after a
    /// failed test).
    #[must_use]
    pub fn new() -> Self {
        Self {
            dir: appkit_scratch::Scratch::new("testkit-devices"),
        }
    }

    /// The file the hub keeps its devices in.
    #[must_use]
    pub fn file(&self) -> std::path::PathBuf {
        self.dir.join("devices.json")
    }
}

impl Default for DemoDevices {
    fn default() -> Self {
        Self::new()
    }
}

impl Subject for DemoDevices {
    fn identity(&self) -> &'static AppIdentity {
        &IDENTITY
    }

    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>> {
        start_with_pages(
            cfg,
            rules()
                .with_installable_page(INSTALLABLE_PREFIX, INSTALLABLE_WORKER_POLICY)
                .with_devices(self.file())
                .with_tailnet_name(TAILNET_NAME),
            Arc::new(TwoPages),
        )
    }

    fn data_routes(&self) -> Vec<&'static str> {
        vec!["items", "item", "ops"]
    }
}

impl Subject for DemoInstallable {
    fn identity(&self) -> &'static AppIdentity {
        &IDENTITY
    }

    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>> {
        start_with_pages(
            cfg,
            rules().with_installable_page(INSTALLABLE_PREFIX, INSTALLABLE_WORKER_POLICY),
            Arc::new(TwoPages),
        )
    }

    fn data_routes(&self) -> Vec<&'static str> {
        vec!["items", "item", "ops"]
    }
}

impl Subject for DemoInstallableRecording {
    fn identity(&self) -> &'static AppIdentity {
        &IDENTITY
    }

    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>> {
        start_with_pages(
            cfg,
            rules()
                .with_installable_page(INSTALLABLE_PREFIX, INSTALLABLE_WORKER_POLICY)
                .with_installable_microphone(),
            Arc::new(TwoPages),
        )
    }

    fn data_routes(&self) -> Vec<&'static str> {
        vec!["items", "item", "ops"]
    }
}
