use crate::{FigstudioServer, error::McpError, files::Files};
use figstudio_mcp_core::link::{FileHint, parse_link};

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

fn select(stems: &[String], hint: &FileHint) -> Option<String> {
    match hint {
        FileHint::Exact(key) => Some(key.clone()),
        FileHint::Figma { key, .. } => stems
            .iter()
            .find(|stem| *stem == key)
            .or_else(|| stems.iter().find(|stem| hint.matches(stem)))
            .cloned(),
    }
}
