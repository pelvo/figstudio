//! Shared, wasm-safe wire protocol for the local browser bridge.
use crate::{CoreError, node_id};
use figstudio_fig::Guid;
use serde::{Deserialize, Serialize};

/// Version checked before accepting uploads or state.
pub const PROTOCOL_VERSION: u32 = 1;

/// A browser tab's current view. IDs use `session:local` strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    /// Browser-local tab identifier.
    pub tab: u64,
    /// Visible page, if any.
    pub page: Option<String>,
    /// Selected node, if any.
    pub selection: Option<String>,
    /// Visible rectangle in page coordinates: x, y, width, height.
    pub viewport: Option<[f64; 4]>,
}

/// Text frames sent by the browser. File bytes follow `Open` in binary frames.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Pair with the local process.
    Hello { token: String, version: u32 },
    /// Start an upload; `name` is a filename, not a path.
    Open { tab: u64, name: String, size: u64 },
    /// Finish the current upload.
    End { tab: u64 },
    /// Replace the current view state.
    State {
        tabs: Vec<TabState>,
        active: Option<u64>,
    },
    /// Close a document tab.
    Close { tab: u64 },
    /// Acknowledge a reveal or return its error.
    Reply { id: u64, error: Option<String> },
}

/// Text frames sent by the bridge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Authentication succeeded.
    Welcome { version: u32 },
    /// A rejected message or upload (contains no file content). `tab` names the upload that
    /// failed, so the browser stops reporting it; absent for other errors and older bridges.
    Error {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<u64>,
    },
    /// Select and fit a node in a browser tab.
    Reveal { id: u64, tab: u64, node: String },
}

/// Encode a document ID without exposing document content.
pub fn guid_string(id: Guid) -> String {
    format!("{}:{}", id.0, id.1)
}

/// Parse a wire GUID, rejecting instance paths and URLs.
///
/// # Errors
/// Returns an invalid-node-ID error for anything other than `session:local`.
pub fn parse_guid(input: &str) -> Result<Guid, CoreError> {
    if !input.contains(':') || input.contains('-') {
        return Err(CoreError::invalid_node_id());
    }
    node_id::parse_guid(input)
}
