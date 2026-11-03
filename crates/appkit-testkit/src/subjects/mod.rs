//! The app the security matrix runs for in this kit: `Demo`. An app runs it for its own hub.

mod demo;

pub use self::demo::{Demo, DemoWide, IDENTITY as DEMO_IDENTITY, WIDE_ORIGIN};
