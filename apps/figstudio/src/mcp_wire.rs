//! Pure helpers for the browser bridge client (`mcp_web.rs`): what the tab tells the local
//! MCP process, and how it reads what comes back. No browser types, so native tests cover it.
use std::collections::HashSet;

use figstudio_mcp_core::bridge::{ClientMessage, TabState, guid_string, parse_guid};

use crate::mcp_state::{reveal, viewport};
use crate::tab::Tab;

/// Upload slice size: well under the bridge's 1 MiB frame limit.
pub(crate) const CHUNK: u64 = 256 * 1024;

/// The `state` frame: only tabs the bridge has fully received (`ready`), since it rejects
/// unknown tab ids.
pub(crate) fn state_message(tabs: &[Tab], active: usize, ready: &HashSet<u64>) -> ClientMessage {
    let tabs_out = tabs
        .iter()
        .filter(|t| ready.contains(&t.id))
        .map(|t| TabState {
            tab: t.id,
            page: t.page_index().map(|_| guid_string(t.page)),
            selection: t.selection.map(guid_string),
            viewport: viewport(t)
                .filter(|v| v.iter().all(|x| x.is_finite()) && v[2] >= 0. && v[3] >= 0.),
        })
        .collect();
    ClientMessage::State {
        tabs: tabs_out,
        active: tabs
            .get(active)
            .map(|t| t.id)
            .filter(|id| ready.contains(id)),
    }
}

/// `#mcp=<token>` from `location.hash`; the fragment never reaches a server or its logs.
/// The `#room=`/`#exec=` pair keys are parsed and stripped in `mcp_pair.rs`/`mcp_join.rs`.
pub(crate) fn token_from_fragment(hash: &str) -> Option<String> {
    form_urlencoded::parse(hash.trim_start_matches('#').as_bytes())
        .find(|(key, value)| key == "mcp" && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// Which open file backs each tab, so the bridge streams the bytes the tab shows. A file moves
/// picked -> loading -> bound to a tab, and is tied to its load by a pick id, not by name: two
/// files may share a name, and picks may overlap. Generic so native tests need no browser `File`.
pub(crate) struct Uploads<T> {
    next_pick: u32,
    /// Read by the browser, not yet handed to a load.
    picked: std::collections::HashMap<u32, T>,
    /// The file of the one in-flight load of each path (`Pending` allows one per path).
    loading: std::collections::HashMap<std::path::PathBuf, T>,
    by_tab: std::collections::HashMap<u64, T>,
}

impl<T: Clone> Uploads<T> {
    pub fn new() -> Self {
        Self {
            next_pick: 0,
            picked: Default::default(),
            loading: Default::default(),
            by_tab: Default::default(),
        }
    }

    /// A file was read successfully; the returned id travels with it to `start` or `discard`.
    pub fn pick(&mut self, file: T) -> u32 {
        self.next_pick = self.next_pick.wrapping_add(1);
        self.picked.insert(self.next_pick, file);
        self.next_pick
    }

    /// The load of `name` began with picked file `pick`; that load owns the file from now on.
    pub fn start(&mut self, name: std::path::PathBuf, pick: u32) {
        if let Some(file) = self.picked.remove(&pick) {
            self.loading.insert(name, file);
        }
    }

    /// The pick was not loaded (a duplicate open was ignored): release its file.
    pub fn discard(&mut self, pick: u32) {
        self.picked.remove(&pick);
    }

    /// The load of `name` finished as tab `tab`.
    pub fn bind(&mut self, name: &std::path::Path, tab: u64) {
        if let Some(file) = self.loading.remove(name) {
            self.by_tab.insert(tab, file);
        }
    }

    /// The load of `name` failed: release its file.
    pub fn fail(&mut self, name: &std::path::Path) {
        self.loading.remove(name);
    }

    pub fn get(&self, tab: u64) -> Option<T> {
        self.by_tab.get(&tab).cloned()
    }

    pub fn forget(&mut self, tab: u64) {
        self.by_tab.remove(&tab);
    }
}

/// `(offset, length)` slices covering `size` bytes.
pub(crate) fn chunk_ranges(size: u64) -> impl Iterator<Item = (u64, u64)> {
    (0..size.div_ceil(CHUNK)).map(move |i| (i * CHUNK, CHUNK.min(size - i * CHUNK)))
}

/// A filename the bridge accepts: it refuses path separators, `:` and control characters.
pub(crate) fn upload_name(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut end = clean.len().min(200);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    match &clean[..end] {
        "" | "." | ".." => "untitled".to_owned(),
        name => name.to_owned(),
    }
}

/// Answer a `reveal` frame: the node id is untrusted wire text, so a bad one is an error
/// reply, never a panic.
pub(crate) fn reveal_wire(
    tabs: &mut [Tab],
    active: &mut usize,
    tab: u64,
    node: &str,
) -> Result<(), String> {
    let node = parse_guid(node).map_err(|_| "invalid node id".to_owned())?;
    reveal(tabs, active, tab, node)
}

/// A browser error as one line for a toast: the message only, never the JS stack that a
/// `{:?}` of a `JsValue` would print.
#[cfg(target_arch = "wasm32")]
pub(crate) fn js_error_text(error: &wasm_bindgen::JsValue) -> String {
    use wasm_bindgen::JsCast;
    if let Some(error) = error.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    // A DOMException is not a `js_sys::Error` but has a string `message`.
    js_sys::Reflect::get(error, &"message".into())
        .ok()
        .and_then(|m| m.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_else(|| "browser error".to_owned())
}

/// How long a new socket may take to say `welcome` before the tab gives up.
#[cfg(target_arch = "wasm32")]
pub(crate) const CONNECT_TIMEOUT_MS: i32 = 8_000;

/// The toast when the socket never got a `welcome`: a 1006 close, or the timeout. Browsers
/// cause both without a bridge message: Chromium's Local Network Access check, and Safari
/// blocking https -> ws:// as mixed content.
pub(crate) fn connect_failed_message() -> &'static str {
    "Could not connect to the MCP bridge. Is `figstudio-mcp --bridge` running? Allow local \
     network access for this site, or use http://127.0.0.1:8765 (Safari blocks https -> ws://)."
}

/// The toast for a socket that closed with `code`: a close after `welcome` or with a code the
/// bridge chose is reported as such; a 1006 before any `welcome` is a blocked connection.
pub(crate) fn closed_message(welcomed: bool, code: u16) -> &'static str {
    if welcomed || code != 1006 {
        close_reason(code)
    } else {
        connect_failed_message()
    }
}

/// Why the bridge closed the socket, in words for a toast.
pub(crate) fn close_reason(code: u16) -> &'static str {
    match code {
        4401 => "MCP bridge refused the pairing token",
        4426 => "MCP bridge speaks a different protocol version",
        _ => "MCP bridge disconnected",
    }
}

/// Browser paths are just display names, so duplicate stems need a stable tab-id suffix.
pub(crate) fn relay_keys(tabs: &[Tab]) -> Vec<String> {
    let stems: Vec<_> = tabs
        .iter()
        .map(|tab| {
            tab.path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("untitled")
        })
        .collect();
    let mut used = HashSet::new();
    stems
        .iter()
        .zip(tabs)
        .map(|(stem, tab)| {
            let mut key = (*stem).to_owned();
            if stems.iter().filter(|other| *other == stem).count() > 1 {
                key.push_str(&format!("~{}", tab.id));
            }
            while !used.insert(key.clone()) {
                key.push_str(&format!("~{}", tab.id));
            }
            key
        })
        .collect()
}

/// Room URLs are bearer capabilities: exactly 256 bits of lower-case hex.
pub(crate) fn valid_room(room: &str) -> bool {
    room.len() == 67
        && room.starts_with("rm_")
        && room[3..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Same-origin path the page asks once per load (and again on a Connect after a failed
/// probe) to learn whether its origin runs the cloud relay Worker (`web/worker.mjs`).
/// Root-relative, like the `/bridge/` socket URL.
pub(crate) const RELAY_PROBE_PATH: &str = "/mcp/";
/// Header only the relay Worker sets on its probe answer, and the one value it sets.
pub(crate) const RELAY_PROBE_HEADER: &str = "figstudio-relay";
pub(crate) const RELAY_PROBE_VALUE: &str = "1";
/// A probe still unanswered by then counts as a failed request, so a request that never
/// settles cannot hold a Cloud relay connect for the whole page load.
#[cfg(target_arch = "wasm32")]
pub(crate) const RELAY_PROBE_TIMEOUT_MS: i32 = 5_000;

/// Whether this page's origin runs the cloud relay. A static-only deploy (no Worker)
/// answers the probe with a missing asset or a fallback page, neither with the header,
/// so Cloud relay is hidden there instead of opening a socket that can never connect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CloudRelay {
    /// The probe has not answered yet: offer the choice, but do not connect.
    #[default]
    Unknown,
    Available,
    /// The origin answered, without the Worker's header: a static-only deploy.
    Absent,
    /// The probe failed (offline, timed out, a server error, a sign-in redirect the page
    /// may not follow): Cloud relay stays offered and the next connect asks again.
    Unreachable,
}

impl CloudRelay {
    /// `status` is `None` when the request itself failed. The browser follows redirects,
    /// so a sign-in page that loads is judged like any other answer.
    pub(crate) fn from_probe(status: Option<u16>, header: Option<&str>) -> Self {
        match status {
            None | Some(429 | 500..=599) => Self::Unreachable,
            Some(200..=299) if header.is_some_and(|value| value.trim() == RELAY_PROBE_VALUE) => {
                Self::Available
            }
            Some(_) => Self::Absent,
        }
    }

    /// The menu offers Cloud relay unless the origin is known not to run it.
    pub(crate) fn offered(self) -> bool {
        self != Self::Absent
    }
}

/// The toast for a Cloud relay connect on an origin without the relay Worker.
pub(crate) const NO_CLOUD_RELAY: &str =
    "This site has no cloud relay; use figstudio-mcp --browser-proxy or --bridge";
/// The toast for a Cloud relay connect whose probe failed.
pub(crate) const CLOUD_RELAY_UNREACHABLE: &str =
    "Could not check whether this site runs the cloud relay; press Connect to try again";

/// Holds a Cloud relay connect until the probe has answered, so none is dropped or sent
/// down the local bridge: the page load's own connect, Connect, and Join / Apply /
/// Regenerate all go through [`CloudGate::step`].
#[derive(Debug, Default)]
pub(crate) struct CloudGate {
    waiting: Wait,
}

/// Who the waiting connect is for: a static-only deploy answers every page load, so only
/// a connect the user asked for is told there is no relay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Wait {
    #[default]
    None,
    PageLoad,
    Asked,
}

/// What the tab does about Cloud relay this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CloudStep {
    /// Open the socket now.
    pub(crate) connect: bool,
    /// Ask the origin again: the last probe failed and a connect was asked for.
    pub(crate) probe: bool,
    /// Leave Cloud relay: this origin has no relay Worker.
    pub(crate) leave: bool,
    /// A toast for a connect the probe's answer turned down.
    pub(crate) notice: Option<&'static str>,
}

impl CloudGate {
    /// The page load chose Cloud relay (`wanted`): connect once the probe confirms it.
    pub(crate) fn page_load(wanted: bool) -> Self {
        Self {
            waiting: if wanted { Wait::PageLoad } else { Wait::None },
        }
    }

    /// `cloud_mode`: the tab is on Cloud relay (not the local proxy or the upload bridge).
    /// `asked`: a connect was asked for this frame. Outside Cloud relay the request passes
    /// straight through, and a connect still waiting is dropped.
    pub(crate) fn step(&mut self, relay: CloudRelay, cloud_mode: bool, asked: bool) -> CloudStep {
        let waited = std::mem::take(&mut self.waiting);
        if !cloud_mode {
            return CloudStep {
                connect: asked,
                ..CloudStep::default()
            };
        }
        let wait = if asked { Wait::Asked } else { waited };
        let mut step = CloudStep::default();
        match relay {
            CloudRelay::Unknown => self.waiting = wait,
            CloudRelay::Available => step.connect = wait != Wait::None,
            CloudRelay::Absent => {
                step.leave = true;
                step.notice = (wait == Wait::Asked).then_some(NO_CLOUD_RELAY);
            }
            // Only a new request asks again, so one failed probe costs one toast and
            // never becomes a probe loop.
            CloudRelay::Unreachable if asked => {
                step.probe = true;
                self.waiting = Wait::Asked;
            }
            CloudRelay::Unreachable => {
                step.notice = (wait != Wait::None).then_some(CLOUD_RELAY_UNREACHABLE);
            }
        }
        step
    }

    /// The part of [`Self::step`] that applies before the menu draws: an origin known to
    /// lack the relay leaves Cloud relay. A waiting connect is released only by `step`,
    /// after the menu, so switching to Local bridge in that frame still cancels it.
    pub(crate) fn before_menu(&mut self, relay: CloudRelay, cloud_mode: bool) -> CloudStep {
        if relay == CloudRelay::Absent {
            self.step(relay, cloud_mode, false)
        } else {
            CloudStep::default()
        }
    }

    /// A Cloud relay connect is waiting for the probe.
    pub(crate) fn waiting(&self) -> bool {
        self.waiting != Wait::None
    }
}

/// Location supplies the authority (including a development port), not a file name.
/// The caller URL keeps its format; the room in it is now derived (mcp_pair.rs).
pub(crate) fn relay_urls(protocol: &str, host: &str, room: &str) -> Option<(String, String)> {
    let ws = match protocol {
        "https:" => "wss",
        "http:" => "ws",
        _ => return None,
    };
    if host.is_empty() || !valid_room(room) {
        return None;
    }
    Some((
        format!("{ws}://{host}/bridge/{room}"),
        format!("{protocol}//{host}/mcp/{room}"),
    ))
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
pub(crate) enum RelayFrame {
    #[serde(rename = "rpc")]
    Rpc { id: String, body: serde_json::Value },
}

pub(crate) fn parse_rpc(text: &str) -> Option<(String, serde_json::Value)> {
    let RelayFrame::Rpc { id, body } = serde_json::from_str(text).ok()?;
    (!id.is_empty() && body.is_object()).then_some((id, body))
}

/// Serialized `rpc_reply` budget: below the relay's 8 MiB frame cap (`REPLY_LIMIT` in
/// `web/worker.mjs`, which validates this shape) with headroom, so one oversized reply
/// becomes an error result for its own call instead of failing every pending call.
pub(crate) const REPLY_BUDGET: usize = 7_864_320; // 7.5 MiB

/// The reply frame: `status` 200 with the parsed JSON-RPC `body`, or 202 with no body for
/// a notification. The body remains JSON, never double-encoded. A reply whose serialized
/// frame exceeds [`REPLY_BUDGET`] is replaced by a JSON-RPC `isError` result for the same
/// id (the worker's validating side is `webSocketMessage` in `web/worker.mjs`).
pub(crate) fn rpc_reply(id: &str, response: Option<&str>) -> Result<String, serde_json::Error> {
    let (status, body) = match response {
        Some(text) => (200, serde_json::from_str::<serde_json::Value>(text)?),
        None => (202, serde_json::Value::Null),
    };
    let frame = serde_json::json!({"type":"rpc_reply", "id":id, "status":status, "body":body});
    let text = frame.to_string();
    if text.len() <= REPLY_BUDGET {
        return Ok(text);
    }
    let rpc_id = frame["body"]
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({
        "type":"rpc_reply", "id":id, "status":200,
        "body":{"jsonrpc":"2.0", "id":rpc_id, "result":{
            "content":[{"type":"text","text":format!(
                "reply too large ({} bytes); request fewer nodes or a smaller scale", text.len())}],
            "isError":true}}
    })
    .to_string())
}

#[cfg(test)]
pub(crate) fn relay_ready() -> &'static str {
    r#"{"type":"ready","protocol":1}"#
}

fn sanitize_wire_text(input: &str, max_chars: usize) -> String {
    input
        .chars()
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

/// Protocol 2 `ready` frame carrying the device identity, page instance, focus state,
/// and open document summaries for multi-device session routing (`web/worker.mjs`).
pub(crate) fn relay_ready_v2(
    device_id: &str,
    device_label: &str,
    instance: &str,
    focused: bool,
    tabs: &[Tab],
) -> String {
    let file_keys = relay_keys(tabs);
    let docs: Vec<serde_json::Value> = tabs
        .iter()
        .zip(file_keys)
        .take(32)
        .map(|(tab, file_key)| {
            let name = sanitize_wire_text(&tab.title(), 96);
            let mut entry = serde_json::json!({
                "tab": tab.id,
                "name": if name.is_empty() { "untitled".to_owned() } else { name },
                "fileKey": file_key,
                "pages": tab.doc.pages.len(),
            });
            if let Some(key) = &tab.doc_key {
                entry["key"] = serde_json::Value::String(key.clone());
            }
            entry
        })
        .collect();
    let cleaned_label = sanitize_wire_text(device_label, 64);
    let label = if cleaned_label.is_empty() {
        "Browser tab".to_owned()
    } else {
        cleaned_label
    };
    serde_json::json!({
        "type": "ready",
        "protocol": 2,
        "device": {
            "id": device_id,
            "label": label,
        },
        "instance": instance,
        "focused": focused,
        "docs": docs,
    })
    .to_string()
}

/// The reply when the dispatcher's output could not be encoded as a frame: still a frame
/// the relay validates (`webSocketMessage` matches `body.id` against the caller's id), so
/// it carries the caller's id — a null id would come back as an invalid-reply 502.
pub(crate) fn encode_failed_reply(id: &str, rpc_id: &serde_json::Value) -> String {
    serde_json::json!({"type":"rpc_reply", "id":id, "status":500,
        "body":{"jsonrpc":"2.0", "id":rpc_id,
            "error":{"code":-32603,"message":"Could not encode MCP reply"}}})
    .to_string()
}

/// The reply when the tab refuses a call because its in-flight job queue is full: the
/// worker's own busy shape (status 503, `-32000`), so a caller retries both refusals the
/// same way (see `MAX_INFLIGHT` in `mcp_relay.rs`).
pub(crate) fn busy_reply(id: &str, rpc_id: &serde_json::Value) -> String {
    serde_json::json!({"type":"rpc_reply", "id":id, "status":503,
        "body":{"jsonrpc":"2.0", "id":rpc_id,
            "error":{"code":-32000,"message":"Relay is busy; retry later"}}})
    .to_string()
}

#[cfg(test)]
#[path = "mcp_wire_tests.rs"]
mod tests;
