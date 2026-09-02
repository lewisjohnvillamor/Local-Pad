//! WebSocket message types exchanged with the phone controller. The
//! TypeScript mirror lives in web/src/protocol/messages.ts.

use localpad_core::frame::InputFrame;
use localpad_core::layout::{Layout, Orientation, OutputMode};
use serde::{Deserialize, Serialize};

/// Compact layout listing sent in Welcome so the phone can offer its own
/// layout picker without another round trip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutSummary {
    pub id: String,
    pub name: String,
    pub orientation: Orientation,
    pub output: OutputMode,
}

/// Messages the phone may send. Anything that fails to parse is dropped;
/// repeated garbage closes the connection.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    /// Must be the first message on the socket.
    Auth { token: String },
    Frame(InputFrame),
    Key { code: String, down: bool },
    MouseButton { button: String, down: bool },
    /// Sent on visibilitychange / pagehide: neutralize all inputs.
    Neutral,
    Recenter,
    Heartbeat { t: f64 },
    SetLayout { id: String },
    Bye,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerMessage {
    Welcome {
        device_id: String,
        device_name: String,
        layout: Layout,
        layouts: Vec<LayoutSummary>,
        server_version: String,
        heartbeat_interval_ms: u64,
    },
    Layout { layout: Layout },
    HeartbeatAck { t: f64 },
    Error { code: ErrorCode, message: String },
    Bye { reason: String },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AuthRequired,
    BadToken,
    Busy,
}

/// Maximum accepted WebSocket message size in bytes.
pub const MAX_WS_MESSAGE_BYTES: usize = localpad_core::frame::MAX_FRAME_BYTES;

/// Heartbeat interval the client must keep while active.
pub const HEARTBEAT_INTERVAL_MS: u64 = 1000;

/// The controller is stale after two missed heartbeats.
pub const STALE_AFTER_MS: u64 = 2500;
