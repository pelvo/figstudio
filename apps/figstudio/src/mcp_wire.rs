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

/// Which open file backs each tab, so the bridge streams the bytes the tab shows. Keyed by tab
/// id, not file name: two files may share a name. Generic so native tests need no browser `File`.
pub(crate) struct Uploads<T> {
    /// Picked files not yet bound to a tab (the tab id exists only after its load).
    pending: Vec<(std::path::PathBuf, T)>,
    by_tab: std::collections::HashMap<u64, T>,
}

impl<T: Clone> Uploads<T> {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            by_tab: std::collections::HashMap::new(),
        }
    }

    /// A file was picked under `name`; a newer pick of the same name replaces the pending one.
    pub fn pick(&mut self, name: std::path::PathBuf, file: T) {
        self.pending.retain(|(n, _)| *n != name);
        self.pending.push((name, file));
    }

    /// Tab `tab` was opened from `name`: it takes the pending file of that name, if any.
    pub fn bind(&mut self, name: &std::path::Path, tab: u64) {
        if let Some(i) = self.pending.iter().position(|(n, _)| n == name) {
            let (_, file) = self.pending.remove(i);
            self.by_tab.insert(tab, file);
        }
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

/// Why the bridge closed the socket, in words for a toast.
pub(crate) fn close_reason(code: u16) -> &'static str {
    match code {
        4401 => "MCP bridge refused the pairing token",
        4426 => "MCP bridge speaks a different protocol version",
        _ => "MCP bridge disconnected",
    }
}

#[cfg(test)]
#[path = "mcp_wire_tests.rs"]
mod tests;
