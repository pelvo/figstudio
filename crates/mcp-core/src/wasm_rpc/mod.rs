//! SDK-free MCP JSON-RPC adapter for a snapshot of browser tabs.
mod live;
mod schema;
mod tools;

use crate::{
    CoreError,
    output::{Block, Output},
    rendering::Rendering,
};
use figstudio_fig::{Document, Guid};
pub use schema::tool_list;
use serde_json::{Value, json};
use std::sync::Arc;

/// Immutable tab snapshot. IDs and keys must be unique within an environment.
#[derive(Clone, Debug)]
pub struct McpTab {
    /// Stable application tab ID.
    pub id: u64,
    /// Display file name, never a local filesystem path.
    pub name: String,
    /// File stem/key used by tool requests.
    pub file_key: String,
    /// Open document.
    pub doc: Arc<Document>,
    /// Images and their byte-backed source.
    pub rendering: Rendering,
    /// Current page.
    pub page: Guid,
    /// Selected node, if any.
    pub selection: Option<Guid>,
    /// Visible page rectangle [x, y, width, height].
    pub viewport: [f64; 4],
}

/// Browser state captured before dispatching a request off the UI thread.
pub trait McpTabEnv {
    /// Open tabs in display order.
    fn tabs(&self) -> &[McpTab];
    /// Active tab ID.
    fn active_tab(&self) -> Option<u64>;
    /// Queue a UI reveal; failure is reported as a tool error.
    ///
    /// # Errors
    /// Returns [`CoreError`] when the UI cannot accept the command.
    fn reveal(&self, tab: u64, node: Guid) -> Result<(), CoreError>;
}

/// Dispatch one JSON-RPC request. Notifications have no response (HTTP callers send 202).
pub fn handle_request(req_json: &str, env: &dyn McpTabEnv) -> Option<String> {
    let req: Value = match serde_json::from_str(req_json) {
        Ok(req) => req,
        Err(_) => return Some(error(Value::Null, -32700, "Parse error")),
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return Some(error(id, -32600, "Invalid Request"));
    };
    if !matches!(id, Value::Null | Value::String(_) | Value::Number(_)) {
        return Some(error(Value::Null, -32600, "Invalid Request"));
    }
    if req.get("jsonrpc") != Some(&json!("2.0")) {
        return Some(error(id, -32600, "Invalid Request"));
    }
    req.get("id")?;
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Some(error(id, -32602, "Invalid params"));
    }
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion":"2025-03-26", "capabilities":{"tools":{},"prompts":{}},
            "serverInfo":{"name":"figstudio-web-mcp","version":env!("CARGO_PKG_VERSION")}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools":tool_list()})),
        "tools/call" => tools::call(&params, env).map(output),
        "prompts/list" => Ok(crate::prompts::list()),
        "prompts/get" => tools::prompt(&params, env),
        // Notifications have no id and already returned `None` above. A request (with an
        // id) whose method is `notifications/*` is not a method this server implements.
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}).to_string(),
        Err(message) => error(id, -32602, &message),
    })
}

fn error(id: Value, code: i32, message: &str) -> String {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}).to_string()
}

fn output(out: Output) -> Value {
    let content: Vec<_> = out
        .content
        .into_iter()
        .map(|block| match block {
            Block::Text(text) => json!({"type":"text","text":text}),
            Block::Image { data_b64, mime } => {
                json!({"type":"image","data":data_b64,"mimeType":mime})
            }
            Block::Blob {
                data_b64,
                uri,
                mime,
            } => json!({"type":"resource","resource":{"uri":uri,"blob":data_b64,"mimeType":mime}}),
        })
        .collect();
    let mut result = json!({"content":content,"isError":out.is_error});
    if let Some(value) = out.structured {
        result["structuredContent"] = value;
    }
    result
}
