use crate::{FigstudioServer, error::McpError, files::Files};
use figstudio_mcp_core::link::{FileHint, best_matches, parse_link};

impl FigstudioServer {
    /// The `fileKey` a tool call should use: an explicit `file_key` wins, then the file named
    /// by a link in `node_id`, else `None` (the single file, as before).
    ///
    /// - `figstudio://<key>?…` names the registered file `<key>`; an unknown key is passed on
    ///   as is, so the tool fails with the usual file-key error.
    /// - A figma.com link selects the file whose stem equals its `<key>`, or whose stem with
    ///   spaces turned into `-` equals its `<slug>` ([`FileHint::matches`]; a ` (N)` download
    ///   suffix is ignored). With no such file its file part is ignored.
    pub fn file_for(&self, file_key: Option<&str>, node_id: Option<&str>) -> Option<String> {
        if let Some(key) = file_key {
            return Some(key.to_owned());
        }
        let hint = parse_link(node_id?).ok()?.file?;
        select(&self.files().keys(), &hint)
    }

    /// The file keys a figma.com or `figstudio://` link matches best: a key match beats a
    /// name match, and every file that ties is listed (two files can share a name).
    pub(crate) fn link_files(&self, link: &str) -> Vec<String> {
        let Some(hint) = parse_link(link).ok().and_then(|link| link.file) else {
            return Vec::new();
        };
        best_matches(self.files().keys().into_iter().filter_map(|key| {
            let rank = hint.match_stem(stem_of(&key))?;
            Some((key, rank))
        }))
    }

    /// The file key and node ID of a call: `node_id`, or in live mode the selection of the tab
    /// `file_key` picks. Live mode returns that tab's key, so the rest of the call reads the same
    /// tab even if the app switches tabs meanwhile.
    ///
    /// # Errors
    /// "nodeId is required" standalone, or when nothing is selected in that tab.
    pub fn target(
        &self,
        file_key: Option<String>,
        node_id: Option<String>,
    ) -> Result<(Option<String>, String), McpError> {
        let Files::Live(live) = self.files() else {
            let node_id = node_id.ok_or_else(|| McpError::node_id_required(false))?;
            return Ok((file_key, node_id));
        };
        let picked = live.pick(file_key.as_deref())?;
        let node_id = match node_id {
            Some(node_id) => node_id,
            None => picked
                .tab()
                .selection
                .ok_or_else(|| McpError::node_id_required(true))?
                .to_string(),
        };
        Ok((Some(picked.key), node_id))
    }
}

fn select(keys: &[String], hint: &FileHint) -> Option<String> {
    match hint {
        FileHint::Exact(key) => Some(key.clone()),
        // A key match beats a name match; among ties the first, as before.
        FileHint::Figma { .. } => best_matches(keys.iter().filter_map(|k| {
            let rank = hint.match_stem(stem_of(k))?;
            Some((k, rank))
        }))
        .first()
        .map(|k| (*k).clone()),
    }
}

/// A file key's stem: the key itself, or for the full path live mode uses when two tabs share
/// a stem (`/a/name.fig`), that path's stem (`name`).
fn stem_of(key: &str) -> &str {
    let path = std::path::Path::new(key);
    let is_fig = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fig"));
    match path.file_stem().and_then(|stem| stem.to_str()) {
        Some(stem) if is_fig && path.parent().is_some_and(|p| !p.as_os_str().is_empty()) => stem,
        _ => key,
    }
}
