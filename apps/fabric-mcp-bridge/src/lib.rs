//! `fabric-mcp-bridge`: the MCP bridge of the agent channel (work package CD-W06,
//! `docs/plans/calendar-desktop-w06-design.md`).
//!
//! The bridge speaks MCP on stdin and stdout, in the modern revision `2026-07-28` and the legacy
//! revision `2025-11-25` (W06 3.2), lists the ten tools of the contract's catalog (W06 3.5), and
//! relays each tool call as one `agent_request` frame to the running broker over its Unix socket
//! (W06 4). Comments such as `(W06 3.4)` or `(A12)` cite sections and decisions of the design.
//!
//! Everything but the socket and the key file has no platform code and is public here, so it
//! builds and is used on every system (W06 9). On a system without Unix sockets every tool call
//! answers `broker_not_running`.

pub mod bridge;
pub mod cli;
pub mod config_text;

/// The MCP side: JSON-RPC lines, the era rule and the methods, the tools and the results.
pub mod mcp {
    pub mod jsonrpc;
    pub mod render;
    pub mod server;
    pub mod tools;
}

/// The broker side: frames, the provisional handshake, the key and the socket.
pub mod broker {
    pub mod frames;
    pub mod handshake;
    pub mod key;
    #[cfg(unix)]
    pub mod socket;
}

pub use config_text::{Client, DebugPaths, config_text};

/// The bridge's name, as `serverInfo` and stderr carry it.
pub const NAME: &str = "fabric-mcp-bridge";

/// The crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Lowercase hexadecimal of bytes.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    fabric_desktop_contracts::digest::hex(bytes)
}

/// 128 random bits from the system's source, or `None` when it fails.
#[must_use]
pub fn random_128() -> Option<[u8; 16]> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes)
}
