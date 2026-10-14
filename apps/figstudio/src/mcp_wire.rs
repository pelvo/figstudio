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
            page: t.doc.pages.contains(&t.page).then(|| guid_string(t.page)),
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

pub(crate) fn new_room() -> Result<String, getrandom::Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)?;
    Ok(format!("rm_{}", hex::encode(bytes)))
}

/// Location supplies the authority (including a development port), not a file name.
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

/// The body remains JSON, never double-encoded. Notifications have no HTTP body.
pub(crate) fn rpc_reply(id: &str, response: Option<&str>) -> Result<String, serde_json::Error> {
    let (status, body) = match response {
        Some(text) => (200, serde_json::from_str::<serde_json::Value>(text)?),
        None => (202, serde_json::Value::Null),
    };
    Ok(serde_json::json!({"type":"rpc_reply", "id":id, "status":status, "body":body}).to_string())
}

pub(crate) fn relay_ready() -> &'static str {
    r#"{"type":"ready","protocol":1}"#
}

#[cfg(test)]
#[path = "mcp_wire_tests.rs"]
mod tests;
