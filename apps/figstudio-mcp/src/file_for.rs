use crate::FigstudioServer;
use figstudio_mcp_core::link::{FileHint, parse_link};

impl FigstudioServer {
    /// The `fileKey` a tool call should use: an explicit `file_key` wins, then the file named
    /// by a link in `node_id`, else `None` (the single file, as before).
    ///
    /// - `figstudio://<key>?…` names the registered file `<key>`; an unknown key is passed on
    ///   as is, so the tool fails with the usual file-key error.
    /// - A figma.com link selects the file whose stem equals its `<key>`, or whose stem with
    ///   spaces turned into `-` equals its `<slug>`. With no such file its file part is ignored.
    pub fn file_for(&self, file_key: Option<&str>, node_id: Option<&str>) -> Option<String> {
        if let Some(key) = file_key {
            return Some(key.to_owned());
        }
        let hint = parse_link(node_id?).ok()?.file?;
        select(&self.files().keys(), &hint)
    }
}

fn select(stems: &[String], hint: &FileHint) -> Option<String> {
    match hint {
        FileHint::Exact(key) => Some(key.clone()),
        FileHint::Figma { key, slug } => stems
            .iter()
            .find(|stem| *stem == key)
            .or_else(|| {
                let slug = slug.as_deref()?;
                stems.iter().find(|stem| stem.replace(' ', "-") == slug)
            })
            .cloned(),
    }
}
