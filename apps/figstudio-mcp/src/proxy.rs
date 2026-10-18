//! Local zero-upload browser proxy (`--browser-proxy`), inspired by `googlecolab/colab-mcp`.
//!
//! Binds a loopback WebSocket endpoint (`ws://127.0.0.1:<port>/bridge`), generates a
//! 256-bit random token, and forwards MCP JSON-RPC calls over `stdio` directly to the
//! browser tab's in-page Wasm executor (`apps/figstudio/src/mcp_relay.rs`) without
//! uploading `.fig` files.

use crate::bridge::{DEFAULT_ORIGINS, pairing_token};
use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use figstudio_mcp_core::wasm_rpc::tool_list;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Notify, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

/// WebSocket subprotocol marker shared with `apps/figstudio/src/mcp_pair.rs` and `web/executor-auth.mjs`.
pub const EXEC_PROTOCOL: &str = "figstudio-exec-v1";
const SOCKET_CAP: usize = 16;
const MAX_PENDING: usize = 128;
const MAX_DOCS: usize = 32;
const DOC_BUDGET_BYTES: usize = 12_000;
const MAX_DOC_NAME: usize = 96;
const MAX_LABEL: usize = 64;
const MAX_ID: usize = 64;
const AMBIGUOUS_SESSION_CODE: i32 = -32002;
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_OPEN_TIMEOUT_SECS: u64 = 60;

/// Configuration for [`ProxyState`]. `Debug` redacts the secret token.
#[derive(Clone)]
pub struct ProxyConfig {
    /// 64-character lowercase hex token verified on `/bridge` WebSocket upgrades.
    pub token: String,
    /// Exact browser `Origin` headers allowed to pair with `/bridge`.
    pub origins: Vec<String>,
    /// Primary web app origin opened by `open_figstudio_browser`.
    pub launch_origin: String,
    /// Whether `open_figstudio_browser` launches the system browser command.
    pub launch_browser: bool,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("origins", &self.origins)
            .field("launch_origin", &self.launch_origin)
            .field("launch_browser", &self.launch_browser)
            .finish_non_exhaustive()
    }
}

impl ProxyConfig {
    /// Build a configuration from extra `--origin` flags (and `FIGSTUDIO_WEB_ORIGIN`).
    ///
    /// # Errors
    /// Returns [`getrandom::Error`] if OS entropy fails.
    pub fn new(extra_origins: &[String], launch_browser: bool) -> Result<Self, getrandom::Error> {
        let token = pairing_token()?;
        let mut origins: Vec<String> = DEFAULT_ORIGINS.iter().map(|s| (*s).to_owned()).collect();
        if let Ok(env_origin) = std::env::var("FIGSTUDIO_WEB_ORIGIN") {
            let trimmed = env_origin.trim().trim_end_matches('/').to_owned();
            if !trimmed.is_empty() && !origins.contains(&trimmed) {
                origins.push(trimmed);
            }
        }
        for origin in extra_origins {
            let trimmed = origin.trim().trim_end_matches('/').to_owned();
            if !trimmed.is_empty() && !origins.contains(&trimmed) {
                origins.push(trimmed);
            }
        }
        let launch_origin = extra_origins
            .first()
            .map(|s| s.trim().trim_end_matches('/').to_owned())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::var("FIGSTUDIO_WEB_ORIGIN")
                    .ok()
                    .map(|s| s.trim().trim_end_matches('/').to_owned())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_ORIGINS[0].to_owned());
        Ok(Self {
            token,
            origins,
            launch_origin,
            launch_browser,
        })
    }

    /// URL with the `#mcpProxyPort=<port>&mcpProxyToken=<token>` fragment that pairs a tab.
    #[must_use]
    pub fn launch_url(&self, port: u16) -> String {
        format!(
            "{}/#mcpProxyPort={port}&mcpProxyToken={}",
            self.launch_origin.trim_end_matches('/'),
            self.token
        )
    }
}

#[derive(Clone, Debug)]
struct TabReady {
    session_id: String,
    device_id: String,
    device_label: String,
    focused: bool,
    docs: Vec<Value>,
    order: u64,
    ready_at: Instant,
}

#[derive(Clone)]
struct ConnectedTab {
    conn_id: u64,
    tx: mpsc::UnboundedSender<String>,
    order: u64,
    ready: Option<TabReady>,
}

struct PendingCall {
    conn_id: u64,
    rpc_id: Value,
    tx: oneshot::Sender<(u16, Value)>,
}

/// Shared state of a `--browser-proxy` process.
pub struct ProxyState {
    pub config: ProxyConfig,
    pub port: u16,
    order_seq: AtomicU64,
    conn_seq: AtomicU64,
    req_seq: AtomicU64,
    tabs: Mutex<HashMap<u64, ConnectedTab>>,
    pending: Mutex<HashMap<String, PendingCall>>,
    stdio_pin: Mutex<Option<String>>,
    native_session_tools: Mutex<HashSet<String>>,
    pub ready_notify: Notify,
    pub tools_changed_notify: Notify,
    pub shutdown: CancellationToken,
}

impl std::fmt::Debug for ProxyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyState")
            .field("config", &self.config)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// 128-bit SHA-256 session identifier (`s_<32 hex>`) matching `deriveSessionId` in `web/worker.mjs`.
#[must_use]
pub fn derive_session_id(device_id: &str, instance: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"figstudio/session/v1\0");
    hasher.update(device_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(instance.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(34);
    out.push_str("s_");
    for b in &digest[..16] {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn clean_str(val: Option<&Value>, max_chars: usize) -> String {
    let Some(s) = val.and_then(Value::as_str) else {
        return String::new();
    };
    s.chars()
        .map(|c| {
            if matches!(c, '\r' | '\n' | '\t') {
                ' '
            } else {
                c
            }
        })
        .filter(|&c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
                )
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(max_chars)
        .collect()
}

fn sanitize_docs(raw_docs: Option<&Value>) -> Vec<Value> {
    let Some(arr) = raw_docs.and_then(Value::as_array) else {
        return Vec::new();
    };
    struct Candidate {
        base: Value,
        base_bytes: usize,
        full: Value,
        full_bytes: usize,
    }
    let mut candidates = Vec::new();
    for item in arr.iter().take(MAX_DOCS) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let tab = obj
            .get("tab")
            .and_then(Value::as_u64)
            .unwrap_or(candidates.len() as u64);
        let name = {
            let n = clean_str(obj.get("name"), MAX_DOC_NAME);
            if n.is_empty() {
                "untitled".to_owned()
            } else {
                n
            }
        };
        let mut base = json!({ "tab": tab, "name": name });
        let key = clean_str(obj.get("key"), 32);
        if !key.is_empty() {
            base["key"] = Value::String(key);
        }
        if let Some(pages) = obj.get("pages").and_then(Value::as_u64) {
            base["pages"] = json!(pages);
        }
        let base_bytes = base.to_string().len() + 1;
        let file_key = obj
            .get("fileKey")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let (full, full_bytes) = if let Some(fk) = file_key {
            let mut f = base.clone();
            f["fileKey"] = Value::String(fk.to_owned());
            let len = f.to_string().len() + 1;
            (f, len)
        } else {
            (base.clone(), base_bytes)
        };
        candidates.push(Candidate {
            base,
            base_bytes,
            full,
            full_bytes,
        });
    }
    let mut out = Vec::new();
    let mut used_bytes = 2usize;
    for (i, cur) in candidates.iter().enumerate() {
        if used_bytes + cur.full_bytes <= DOC_BUDGET_BYTES {
            out.push(cur.full.clone());
            used_bytes += cur.full_bytes;
            continue;
        }
        if used_bytes + cur.base_bytes > DOC_BUDGET_BYTES {
            continue;
        }
        let mut later_bytes = 0usize;
        for next in &candidates[i + 1..] {
            if used_bytes + later_bytes + next.full_bytes <= DOC_BUDGET_BYTES {
                later_bytes += next.full_bytes;
            } else if used_bytes + later_bytes + next.base_bytes <= DOC_BUDGET_BYTES {
                later_bytes += next.base_bytes;
            }
        }
        if used_bytes + cur.base_bytes + later_bytes <= DOC_BUDGET_BYTES {
            out.push(cur.base.clone());
            used_bytes += cur.base_bytes;
        }
    }
    out
}

pub fn relay_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "open_figstudio_browser",
            "description": "Open the figstudio browser app connected to this local proxy (if no browser tab is connected yet) and wait for it to become ready.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "timeoutSeconds": {
                        "type": "integer",
                        "description": "Maximum seconds to wait for the browser tab to connect (1-120, default 60)."
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "list_sessions",
            "description": "List connected figstudio browser sessions on this proxy, their device labels, and open documents (`docs[].fileKey` is the tool `fileKey` argument; `docs[].key` is the 16-hex content SHA-256 hash).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "select_session",
            "description": "Pin a target figstudio session id (`s_…`) for subsequent tool calls from this MCP client, or pass an empty string to clear the pin.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "Session id from `list_sessions` (e.g. `s_a1b2c3d4e5f60718…`), or `\"\"` to clear."
                    }
                },
                "required": ["session"],
                "additionalProperties": false
            }
        }),
    ]
}

fn augment_tools(
    raw_tools: Vec<Value>,
    native_session_tools: &Mutex<HashSet<String>>,
) -> Vec<Value> {
    let session_schema = json!({
        "type": "string",
        "description": "Target figstudio session id from `list_sessions` (required when multiple devices are connected)."
    });
    let mut existing = HashSet::new();
    let mut ns = native_session_tools
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut out: Vec<Value> = raw_tools
        .into_iter()
        .map(|mut tool| {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            existing.insert(name.clone());
            if let Some(props) = tool
                .get_mut("inputSchema")
                .and_then(|s| s.get_mut("properties"))
                .and_then(Value::as_object_mut)
            {
                if props.contains_key("session") {
                    ns.insert(name);
                } else {
                    ns.remove(&name);
                    props.insert("session".to_owned(), session_schema.clone());
                }
            }
            tool
        })
        .collect();
    for rt in relay_tools() {
        let name = rt.get("name").and_then(Value::as_str).unwrap_or_default();
        if !existing.contains(name) {
            out.push(rt);
        }
    }
    out
}

impl ProxyState {
    #[must_use]
    pub fn new(config: ProxyConfig, port: u16, shutdown: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            config,
            port,
            order_seq: AtomicU64::new(0),
            conn_seq: AtomicU64::new(0),
            req_seq: AtomicU64::new(0),
            tabs: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            stdio_pin: Mutex::new(None),
            native_session_tools: Mutex::new(HashSet::new()),
            ready_notify: Notify::new(),
            tools_changed_notify: Notify::new(),
            shutdown,
        })
    }

    fn next_order(&self) -> u64 {
        self.order_seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn get_pinned(&self) -> Option<String> {
        self.stdio_pin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_pinned(&self, target: Option<String>) {
        *self
            .stdio_pin
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = target;
    }

    fn ready_tabs(&self) -> Vec<(ConnectedTab, TabReady)> {
        let tabs = self.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ready: Vec<(ConnectedTab, TabReady)> = tabs
            .values()
            .filter_map(|t| t.ready.clone().map(|r| (t.clone(), r)))
            .collect();
        ready.sort_by_key(|a| std::cmp::Reverse(a.1.order));
        ready
    }

    pub fn list_sessions_payload(&self) -> Value {
        let ready = self.ready_tabs();
        let open_count = self
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let now = Instant::now();
        let sessions: Vec<Value> = ready
            .iter()
            .map(|(_, r)| {
                json!({
                    "session": r.session_id,
                    "device": { "label": r.device_label },
                    "focused": r.focused,
                    "docs": r.docs,
                    "secondsSinceReady": now.saturating_duration_since(r.ready_at).as_secs(),
                })
            })
            .collect();
        json!({
            "sessions": sessions,
            "selectedSession": self.get_pinned(),
            "socketsInUse": open_count,
            "socketCap": SOCKET_CAP,
        })
    }

    async fn handle_open_browser(&self, id: Value, args: Option<&Value>) -> (u16, Value) {
        let timeout_secs = args
            .and_then(|a| a.get("timeoutSeconds"))
            .and_then(Value::as_u64)
            .map_or(DEFAULT_OPEN_TIMEOUT_SECS, |s| s.clamp(1, 120));
        if !self.ready_tabs().is_empty() {
            let payload = self.list_sessions_payload();
            let out = json!({
                "connected": true,
                "launched": false,
                "sessions": payload["sessions"],
            });
            return (
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&out).unwrap_or_default() }],
                        "structuredContent": out,
                    }
                }),
            );
        }
        let url = self.config.launch_url(self.port);
        if self.config.launch_browser {
            open_system_browser(&url);
        }
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            // Register the waiter before checking `ready_tabs()` so a `ready` frame
            // arriving between the check and `.await` is never lost.
            let notified = self.ready_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.ready_tabs().is_empty() {
                let payload = self.list_sessions_payload();
                let out = json!({
                    "connected": true,
                    "launched": self.config.launch_browser,
                    "sessions": payload["sessions"],
                });
                return (
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&out).unwrap_or_default() }],
                            "structuredContent": out,
                        }
                    }),
                );
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let out = json!({
                    "connected": false,
                    "launched": self.config.launch_browser,
                    "message": format!("Timed out after {timeout_secs}s waiting for a figstudio browser tab to connect to ws://127.0.0.1:{}/bridge.", self.port),
                });
                return (
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "isError": true,
                            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&out).unwrap_or_default() }],
                            "structuredContent": out,
                        }
                    }),
                );
            }
            let _ = tokio::time::timeout(remaining, notified).await;
        }
    }

    /// Dispatch one JSON-RPC message over `stdio`. Returns `None` for notifications.
    pub async fn dispatch_rpc(self: &Arc<Self>, body: Value) -> Option<(u16, Value)> {
        if body.is_array() {
            return Some((
                400,
                rpc_error(
                    Value::Null,
                    -32600,
                    "Batch requests are not supported",
                    None,
                ),
            ));
        }
        let Some(obj) = body.as_object() else {
            return Some((
                400,
                rpc_error(Value::Null, -32600, "Invalid JSON-RPC message", None),
            ));
        };
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some((
                400,
                rpc_error(Value::Null, -32600, "Invalid JSON-RPC message", None),
            ));
        }
        if let Some(id_val) = obj.get("id")
            && !matches!(id_val, Value::Null | Value::String(_) | Value::Number(_))
        {
            return Some((
                400,
                rpc_error(Value::Null, -32600, "Invalid JSON-RPC message", None),
            ));
        }
        let method = obj.get("method").and_then(Value::as_str).unwrap_or("");
        if !obj.contains_key("method")
            && obj.contains_key("id")
            && (obj.contains_key("result") != obj.contains_key("error"))
        {
            return None;
        }
        if method.is_empty() || obj.contains_key("result") || obj.contains_key("error") {
            let id = obj.get("id").cloned().unwrap_or(Value::Null);
            return Some((400, rpc_error(id, -32600, "Invalid JSON-RPC request", None)));
        }
        let id = obj.get("id").cloned()?;

        // Always handle `initialize` and `ping` directly in the proxy so `tools.listChanged`
        // is advertised consistently whether or not a browser tab is already connected, and
        // `ping` never blocks on a tab.
        if method == "initialize" {
            let ver = obj
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or("2025-03-26");
            return Some((
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": ver,
                        "capabilities": { "tools": { "listChanged": true }, "prompts": {} },
                        "serverInfo": { "name": "figstudio-mcp", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": "Connected via figstudio-mcp --browser-proxy. If no browser tab is open yet, call `open_figstudio_browser` to launch and pair the browser automatically."
                    }
                }),
            ));
        }
        if method == "ping" {
            return Some((200, json!({ "jsonrpc": "2.0", "id": id, "result": {} })));
        }

        if method == "tools/call" {
            let tool_name = obj
                .get("params")
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = obj.get("params").and_then(|p| p.get("arguments"));
            if tool_name == "open_figstudio_browser" {
                return Some(self.handle_open_browser(id, args).await);
            }
            if tool_name == "list_sessions" {
                let payload = self.list_sessions_payload();
                return Some((
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&payload).unwrap_or_default() }],
                            "structuredContent": payload,
                        }
                    }),
                ));
            }
            if tool_name == "select_session" {
                let Some(target_raw) = args.and_then(|a| a.get("session")).and_then(Value::as_str)
                else {
                    return Some((
                        400,
                        rpc_error(
                            id,
                            -32602,
                            "`select_session` requires a string `session` argument",
                            None,
                        ),
                    ));
                };
                let target = target_raw.trim();
                if target.is_empty() {
                    self.set_pinned(None);
                    let payload = self.list_sessions_payload();
                    return Some((
                        200,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{ "type": "text", "text": serde_json::to_string_pretty(&json!({ "cleared": true, "sessions": payload["sessions"] })).unwrap_or_default() }]
                            }
                        }),
                    ));
                }
                let matches: Vec<_> = self
                    .ready_tabs()
                    .into_iter()
                    .filter(|(_, r)| r.session_id == target)
                    .collect();
                if matches.len() != 1 {
                    let payload = self.list_sessions_payload();
                    let msg = if matches.is_empty() {
                        format!(
                            "Session '{target}' is not connected. Live sessions: {}",
                            payload["sessions"]
                        )
                    } else {
                        format!(
                            "Session '{target}' matches multiple executors; cannot pin an ambiguous session."
                        )
                    };
                    return Some((
                        200,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "isError": true,
                                "content": [{ "type": "text", "text": msg }]
                            }
                        }),
                    ));
                }
                self.set_pinned(Some(target.to_owned()));
                let r = &matches[0].1;
                let chosen = json!({
                    "session": r.session_id,
                    "device": { "label": r.device_label },
                    "focused": r.focused,
                    "docs": r.docs,
                });
                return Some((
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&json!({ "selected": chosen })).unwrap_or_default() }]
                        }
                    }),
                ));
            }
        }

        let ready = self.ready_tabs();
        if ready.is_empty() {
            if method == "tools/list" {
                let tools = augment_tools(tool_list(), &self.native_session_tools);
                return Some((
                    200,
                    json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } }),
                ));
            }
            return Some((
                503,
                rpc_error(
                    id,
                    -32000,
                    "No browser tab connected; call `open_figstudio_browser` to launch and connect the browser",
                    None,
                ),
            ));
        }

        let mut forward_body = body.clone();
        let target_tab = if method == "tools/call" {
            match self.resolve_tool_target(&id, &mut forward_body, &ready) {
                Ok(t) => t,
                Err((st, err_val)) => return Some((st, err_val)),
            }
        } else {
            ready[0].clone()
        };

        let req_id = format!("req_{}", self.req_seq.fetch_add(1, Ordering::Relaxed) + 1);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if pending.len() >= MAX_PENDING {
                return Some((
                    503,
                    rpc_error(id, -32000, "Relay is busy; retry later", None),
                ));
            }
            pending.insert(
                req_id.clone(),
                PendingCall {
                    conn_id: target_tab.0.conn_id,
                    rpc_id: id.clone(),
                    tx,
                },
            );
        }
        let frame = json!({ "type": "rpc", "id": req_id, "body": forward_body }).to_string();
        if target_tab.0.tx.send(frame).is_err() {
            self.fail_conn(target_tab.0.conn_id);
        }
        let (status, mut reply_body) = match tokio::time::timeout(RPC_TIMEOUT, rx).await {
            Ok(Ok(res)) => res,
            _ => {
                self.pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&req_id);
                return Some((504, rpc_error(id, -32000, "Browser tab timed out", None)));
            }
        };

        if let Some(res_obj) = reply_body.get_mut("result").and_then(Value::as_object_mut) {
            if method == "tools/list"
                && let Some(tools_arr) = res_obj.get("tools").and_then(Value::as_array).cloned()
            {
                let augmented = augment_tools(tools_arr, &self.native_session_tools);
                res_obj.insert("tools".to_owned(), Value::Array(augmented));
            } else if method == "tools/call" {
                let mut meta = res_obj
                    .get("_meta")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                meta.insert(
                    "figstudio/session".to_owned(),
                    Value::String(target_tab.1.session_id.clone()),
                );
                meta.insert(
                    "figstudio/device".to_owned(),
                    Value::String(target_tab.1.device_label.clone()),
                );
                res_obj.insert("_meta".to_owned(), Value::Object(meta));
            }
        }
        Some((status, reply_body))
    }

    fn resolve_tool_target(
        &self,
        id: &Value,
        forward_body: &mut Value,
        ready: &[(ConnectedTab, TabReady)],
    ) -> Result<(ConnectedTab, TabReady), (u16, Value)> {
        let tool_name = forward_body
            .get("params")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let is_native_session = self
            .native_session_tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(tool_name);
        let explicit = if is_native_session {
            String::new()
        } else {
            forward_body
                .get("params")
                .and_then(|p| p.get("arguments"))
                .and_then(|a| a.get("session"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_owned()
        };

        if !explicit.is_empty() {
            let matches: Vec<_> = ready
                .iter()
                .filter(|(_, r)| r.session_id == explicit)
                .collect();
            if matches.is_empty() {
                return Err((
                    200,
                    rpc_error(
                        id.clone(),
                        AMBIGUOUS_SESSION_CODE,
                        &format!(
                            "Session '{explicit}' is not connected; choose one of the live sessions"
                        ),
                        Some(self.list_sessions_payload()),
                    ),
                ));
            }
            if matches.len() > 1 {
                return Err((
                    200,
                    rpc_error(
                        id.clone(),
                        AMBIGUOUS_SESSION_CODE,
                        &format!(
                            "Session '{explicit}' matches multiple executors; cannot route unambiguously"
                        ),
                        Some(self.list_sessions_payload()),
                    ),
                ));
            }
            if let Some(args_obj) = forward_body
                .get_mut("params")
                .and_then(|p| p.get_mut("arguments"))
                .and_then(Value::as_object_mut)
            {
                args_obj.remove("session");
            }
            return Ok(matches[0].clone());
        }

        if let Some(pinned) = self.get_pinned() {
            let matches: Vec<_> = ready
                .iter()
                .filter(|(_, r)| r.session_id == pinned)
                .collect();
            if matches.len() == 1 {
                return Ok(matches[0].clone());
            }
            return Err((
                200,
                rpc_error(
                    id.clone(),
                    AMBIGUOUS_SESSION_CODE,
                    &format!(
                        "Pinned session '{pinned}' is no longer connected; call `select_session` or pass `session`"
                    ),
                    Some(self.list_sessions_payload()),
                ),
            ));
        }

        let devices: HashSet<&str> = ready.iter().map(|(_, r)| r.device_id.as_str()).collect();
        let sessions: HashSet<&str> = ready.iter().map(|(_, r)| r.session_id.as_str()).collect();
        if devices.len() <= 1 && sessions.len() == ready.len() {
            let chosen = ready
                .iter()
                .find(|(_, r)| r.focused)
                .unwrap_or(&ready[0])
                .clone();
            return Ok(chosen);
        }

        Err((
            200,
            rpc_error(
                id.clone(),
                AMBIGUOUS_SESSION_CODE,
                "Several figstudio sessions are connected across devices; pass `session` or call `select_session`",
                Some(self.list_sessions_payload()),
            ),
        ))
    }

    fn fail_conn(&self, conn_id: u64) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let dead: Vec<String> = pending
            .iter()
            .filter(|&(_, v)| v.conn_id == conn_id)
            .map(|(k, _)| k.clone())
            .collect();
        for k in dead {
            if let Some(call) = pending.remove(&k) {
                let _ = call.tx.send((
                    503,
                    rpc_error(call.rpc_id, -32000, "Browser tab disconnected", None),
                ));
            }
        }
    }
}

fn rpc_error(id: Value, code: i32, message: &str, data: Option<Value>) -> Value {
    let mut err = json!({ "code": code, "message": message });
    if let Some(d) = data {
        err["data"] = d;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": err })
}

fn open_system_browser(url: &str) {
    if let Some(browser) = std::env::var_os("BROWSER").filter(|b| !b.is_empty()) {
        let _ = std::process::Command::new(browser)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Upgrade `/bridge` WebSocket connections after verifying `Origin` and `Sec-WebSocket-Protocol`.
pub async fn upgrade_bridge(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| state.config.origins.iter().any(|allowed| allowed == o));
    if !origin_ok {
        return (StatusCode::FORBIDDEN, "Cross-origin relay refused").into_response();
    }
    let offered: Vec<&str> = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let token_ok = offered.len() == 2
        && offered[0] == EXEC_PROTOCOL
        && bool::from(offered[1].as_bytes().ct_eq(state.config.token.as_bytes()));
    if !token_ok {
        return (StatusCode::FORBIDDEN, "Missing or invalid executor proof").into_response();
    }
    if state
        .tabs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len()
        >= SOCKET_CAP
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many browser tabs connected",
        )
            .into_response();
    }
    ws.protocols([EXEC_PROTOCOL])
        .max_message_size(8 * 1024 * 1024)
        .max_frame_size(8 * 1024 * 1024)
        .on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<ProxyState>) {
    let conn_id = state.conn_seq.fetch_add(1, Ordering::Relaxed) + 1;
    let first_order = state.next_order();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let over_cap = {
        let mut tabs = state.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        if tabs.len() >= SOCKET_CAP {
            true
        } else {
            tabs.insert(
                conn_id,
                ConnectedTab {
                    conn_id,
                    tx,
                    order: first_order,
                    ready: None,
                },
            );
            false
        }
    };
    if over_cap {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: 1013,
                reason: "Too many browser tabs connected".into(),
            })))
            .await;
        return;
    }

    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
            Some(outbound) = rx.recv() => {
                if socket.send(Message::Text(outbound.into())).await.is_err() {
                    break;
                }
            }
            inbound = socket.recv() => {
                let Some(Ok(msg)) = inbound else { break; };
                match msg {
                    Message::Text(text) => {
                        let s: &str = &text;
                        if s == "ping" {
                            if socket.send(Message::Text("pong".into())).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        handle_ws_frame(&state, conn_id, first_order, s);
                    }
                    Message::Ping(bytes) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Message::Pong(_) => {}
                    Message::Binary(_) | Message::Close(_) => break,
                }
            }
        }
    }

    let was_last_ready = {
        let mut tabs = state.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        let removed = tabs.remove(&conn_id);
        removed.is_some_and(|t| t.ready.is_some()) && tabs.values().all(|t| t.ready.is_none())
    };
    state.fail_conn(conn_id);
    if was_last_ready {
        state.tools_changed_notify.notify_one();
    }
}

fn handle_ws_frame(state: &Arc<ProxyState>, conn_id: u64, first_order: u64, text: &str) {
    let Ok(reply) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(obj) = reply.as_object() else {
        return;
    };
    let msg_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
    if msg_type == "ready" {
        let proto = obj.get("protocol").and_then(Value::as_u64).unwrap_or(0);
        if proto != 1 && proto != 2 {
            return;
        }
        let order = state.next_order();
        let ready_info = if proto == 1 {
            let sid = derive_session_id("v1", &first_order.to_string());
            TabReady {
                session_id: sid,
                device_id: format!("v1:{first_order}"),
                device_label: "Browser tab".to_owned(),
                focused: false,
                docs: Vec::new(),
                order,
                ready_at: Instant::now(),
            }
        } else {
            let dev = obj.get("device");
            let dev_id = {
                let d = clean_str(dev.and_then(|d| d.get("id")), MAX_ID);
                if d.is_empty() {
                    format!("anon_{order}")
                } else {
                    d
                }
            };
            let dev_label = {
                let l = clean_str(dev.and_then(|d| d.get("label")), MAX_LABEL);
                if l.is_empty() {
                    "Browser tab".to_owned()
                } else {
                    l
                }
            };
            let inst = {
                let i = clean_str(obj.get("instance"), MAX_ID);
                if i.is_empty() { order.to_string() } else { i }
            };
            let focused = obj.get("focused").and_then(Value::as_bool).unwrap_or(false);
            let docs = sanitize_docs(obj.get("docs"));
            let sid = derive_session_id(&dev_id, &inst);
            TabReady {
                session_id: sid,
                device_id: dev_id,
                device_label: dev_label,
                focused,
                docs,
                order,
                ready_at: Instant::now(),
            }
        };
        let became_first_ready = {
            let mut tabs = state.tabs.lock().unwrap_or_else(PoisonError::into_inner);
            let had_any_ready = tabs.values().any(|t| t.ready.is_some());
            if let Some(tab) = tabs.get_mut(&conn_id) {
                tab.order = order;
                tab.ready = Some(ready_info);
            }
            !had_any_ready
        };
        state.ready_notify.notify_waiters();
        if became_first_ready {
            state.tools_changed_notify.notify_one();
        }
        return;
    }

    if msg_type != "rpc_reply" {
        return;
    }
    let Some(id_str) = obj.get("id").and_then(Value::as_str) else {
        return;
    };
    let entry = {
        let mut pending = state.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let matches_conn = pending.get(id_str).is_some_and(|p| p.conn_id == conn_id);
        if matches_conn {
            pending.remove(id_str)
        } else {
            None
        }
    };
    let Some(call) = entry else {
        return;
    };
    let status = obj
        .get("status")
        .and_then(Value::as_u64)
        .and_then(|s| u16::try_from(s).ok())
        .unwrap_or(0);
    let body_val = obj.get("body").cloned().unwrap_or(Value::Null);
    let valid = (200..=599).contains(&status)
        && !matches!(status, 204 | 205 | 304)
        && if status == 202 {
            body_val.is_null()
        } else {
            body_val.as_object().is_some_and(|b| {
                b.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
                    && b.get("id") == Some(&call.rpc_id)
                    && (b.contains_key("result") != b.contains_key("error"))
            })
        };
    if !valid {
        let _ = call.tx.send((
            502,
            rpc_error(call.rpc_id, -32000, "Invalid browser reply", None),
        ));
        return;
    }
    let _ = call.tx.send((status, body_val));
}

/// Bind a loopback listener for `--browser-proxy`: tries `3845..=3855` first, then `127.0.0.1:0`.
///
/// # Errors
/// Returns an I/O error only if even binding an ephemeral loopback port fails.
pub async fn bind_proxy_listener(
    exact: Option<SocketAddr>,
) -> std::io::Result<tokio::net::TcpListener> {
    if let Some(addr) = exact {
        return tokio::net::TcpListener::bind(addr).await;
    }
    if let Ok(listener) = crate::bind_first(crate::default_http_addrs()).await {
        return Ok(listener);
    }
    tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await
}

/// Serve the WebSocket `/bridge` route on `listener` (and optional IPv6 loopback).
///
/// # Errors
/// Returns an I/O error if the listener fails.
pub async fn serve_proxy_ws(
    listener: tokio::net::TcpListener,
    state: Arc<ProxyState>,
) -> std::io::Result<()> {
    let port = listener.local_addr()?.port();
    let v6_companion = if listener.local_addr()?.ip() == Ipv4Addr::LOCALHOST {
        tokio::net::TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, port)))
            .await
            .ok()
    } else {
        None
    };

    let router = axum::Router::new()
        .route("/bridge", axum::routing::get(upgrade_bridge))
        .with_state(state.clone());

    let shutdown = state.shutdown.clone();
    let mut tasks = vec![tokio::spawn({
        let router = router.clone();
        let shutdown = shutdown.clone();
        async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        }
    })];
    if let Some(v6) = v6_companion {
        let shutdown = shutdown.clone();
        tasks.push(tokio::spawn(async move {
            axum::serve(v6, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        }));
    }
    for task in tasks {
        if let Ok(Err(e)) = task.await {
            shutdown.cancel();
            return Err(e);
        }
    }
    Ok(())
}

/// Run the newline-delimited JSON-RPC `stdio` loop against `state`, emitting
/// `notifications/tools/list_changed` whenever a browser tab connects or disconnects.
///
/// # Errors
/// Returns an I/O error if reading stdin fails unexpectedly.
pub async fn serve_proxy_stdio(state: Arc<ProxyState>) -> std::io::Result<()> {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer_shutdown = state.shutdown.clone();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        loop {
            tokio::select! {
                _ = writer_shutdown.cancelled() => break,
                maybe_line = out_rx.recv() => {
                    let Some(mut line) = maybe_line else { break; };
                    line.push('\n');
                    if stdout.write_all(line.as_bytes()).await.is_err()
                        || stdout.flush().await.is_err()
                    {
                        break;
                    }
                }
            }
        }
    });

    let notify_state = state.clone();
    let notify_tx = out_tx.clone();
    let notifier = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = notify_state.shutdown.cancelled() => break,
                () = notify_state.tools_changed_notify.notified() => {
                    let _ = notify_tx.send(
                        json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}).to_string(),
                    );
                }
            }
        }
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => break,
            line_res = lines.next_line() => {
                let Ok(Some(line)) = line_res else { break; };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let state = state.clone();
                let out_tx = out_tx.clone();
                let raw = trimmed.to_owned();
                tokio::spawn(async move {
                    let body: Value = match serde_json::from_str(&raw) {
                        Ok(v) => v,
                        Err(_) => {
                            let _ = out_tx.send(rpc_error(Value::Null, -32700, "Parse error", None).to_string());
                            return;
                        }
                    };
                    if let Some((status, reply)) = state.dispatch_rpc(body).await
                        && status != 202
                    {
                        let _ = out_tx.send(reply.to_string());
                    }
                });
            }
        }
    }
    state.shutdown.cancel();
    notifier.abort();
    drop(out_tx);
    let _ = writer.await;
    Ok(())
}
