use crate::Region;
use figstudio_fig::Guid;
use serde::{Deserialize, Serialize};

/// A changed screen, compared in its own coordinates rather than on the page canvas.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenDiff {
    pub old_id: Option<Guid>,
    pub new_id: Option<Guid>,
    pub name: String,
    pub kind: String,
    pub status: ScreenStatus,
    pub matched_by: Option<MatchedBy>,
    pub moved: bool,
    pub width: u32,
    pub height: u32,
    pub pixels_changed: Option<u64>,
    pub thumb_old: Option<String>,
    pub thumb_new: Option<String>,
    pub regions: Vec<Region>,
    /// Report line identities owned by this screen, including lines outside any region.
    #[serde(default)]
    pub nodes: Vec<Guid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScreenStatus {
    Added,
    Removed,
    Changed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchedBy {
    Id,
    NameSize,
}
