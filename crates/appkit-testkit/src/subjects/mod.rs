//! The app the security matrix runs for in this kit: `Demo`. An app runs it for its own hub.

mod demo;

pub use self::demo::{
    Demo, DemoDevices, DemoInstallable, DemoInstallableRecording, DemoWide,
    IDENTITY as DEMO_IDENTITY, INSTALLABLE_PREFIX, INSTALLABLE_WORKER_POLICY, TAILNET_NAME,
    WIDE_ORIGIN,
};
