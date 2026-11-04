//! The default ports of the apps that are known to run side by side on one machine.
//!
//! A port is a promise to the other apps: the central hub probes the ports of the apps it lists,
//! and an app that starts on a port that another one owns would answer for it. This is the one
//! list; `tests/ports.rs` fails when two entries share a port, and when an identity the kit
//! knows is not in the list under its own port.

/// One port and who owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortUse {
    pub port: u16,
    pub owner: &'static str,
}

/// Every default port in use. Keep it sorted by port.
pub const APP_PORTS: &[PortUse] = &[
    PortUse {
        port: 8080,
        owner: "ai-desktop (dashboard)",
    },
    PortUse {
        port: 17610,
        owner: "hub-desktop (the central hub)",
    },
    PortUse {
        port: 17611,
        owner: "hub-desktop (static web variants, its own origin, no API)",
    },
    PortUse {
        port: 17615,
        owner: "tasks-desktop",
    },
    PortUse {
        port: 17616,
        owner: "rules-desktop",
    },
    PortUse {
        port: 17617,
        owner: "appkit-testkit Demo",
    },
    PortUse {
        port: 17618,
        owner: "testing-desktop",
    },
    PortUse {
        port: 17619,
        owner: "pult",
    },
    PortUse {
        port: 17650,
        owner: "streamtap (sink)",
    },
    PortUse {
        port: 17698,
        owner: "end-to-end tests",
    },
    PortUse {
        port: 17699,
        owner: "end-to-end tests",
    },
];
