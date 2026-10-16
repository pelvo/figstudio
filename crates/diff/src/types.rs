use figstudio_fig::{Guid, Val};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffReport {
    pub old: FileMeta,
    pub new: FileMeta,
    pub summary: Summary,
    /// Changed, added and removed pages, including internal canvases.
    pub pages: Vec<PageDiff>,
    pub complete: bool,
    pub truncated: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    /// Empty: documents do not retain their source path.
    pub path_label: String,
    pub format_version: u32,
    pub page_count: usize,
    pub node_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Pages present only in the new document.
    pub pages_added: usize,
    /// Pages present only in the old document.
    pub pages_removed: usize,
    /// Matched pages with changed fields or descendants.
    pub pages_changed: usize,
    /// Unmatched new nodes, excluding page/document containers.
    pub nodes_added: usize,
    /// Unmatched old nodes, excluding page/document containers.
    pub nodes_removed: usize,
    /// Matched changed nodes, including moves and ungrouped nodes.
    pub nodes_changed: usize,
    /// Matched nodes whose parent or sibling position changed.
    pub nodes_moved: usize,
    /// Stored leaf deltas, excluding redundant parentIndex rows on moves.
    pub properties_changed: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageDiff {
    pub id: Guid,
    pub old_name: Option<String>,
    pub new_name: Option<String>,
    pub status: PageStatus,
    pub added: Vec<NodeRef>,
    pub removed: Vec<NodeRef>,
    pub changed: Vec<NodeDiff>,
    /// Cross-page movers whose detailed row is on their destination page.
    #[serde(default)]
    pub moved_out: Vec<Guid>,
    /// Optional render lane result; PNG links are set by the caller.
    pub render: Option<RenderDiff>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderDiff {
    pub old_png: Option<String>,
    pub new_png: Option<String>,
    pub diff_png: Option<String>,
    /// Pixels whose worst channel delta exceeded the threshold; for an added
    /// or removed page, the ink pixels of the present side.
    pub pixels_changed: Option<u64>,
    /// Max per-pixel channel delta on the shared canvas, ignoring the threshold.
    pub worst_channel_delta: Option<u8>,
    /// Shared canvas width in pixels; `None` when neither side rendered.
    pub width: Option<u32>,
    /// Shared canvas height in pixels; `None` when neither side rendered.
    pub height: Option<u32>,
    /// Renderer counters for the old side; `None` when it was not rendered.
    pub old_stats: Option<RenderStats>,
    /// Renderer counters for the new side; `None` when it was not rendered.
    pub new_stats: Option<RenderStats>,
}

/// Renderer work counters copied from `figstudio_render::Stats` (that type is
/// not serializable, so the report stores the same fields).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderStats {
    pub nodes: usize,
    pub instances: usize,
    pub text_stored_glyphs: usize,
    pub text_system_fonts: usize,
    pub text_missing: usize,
    pub text_truncation_ignored: usize,
    pub missing_images: usize,
    pub images_loading: usize,
    pub missing_symbols: usize,
    pub masks: usize,
    pub shadows: usize,
    pub inner_shadows: usize,
    pub approximated_effects: usize,
    pub blurs: usize,
    pub skipped_effects: usize,
    pub unsupported_paints: usize,
    pub approximated_blends: usize,
    pub approximated_paints: usize,
    pub approximated_strokes: usize,
    pub approximated_instance_ink: usize,
    pub culled: usize,
    pub budget_exhausted: usize,
    pub depth_skipped: usize,
    pub preparation_skipped: usize,
    pub attempted_work: usize,
    pub unrecorded_tiles: usize,
    pub gpu_fallback_tiles: usize,
}

impl From<&figstudio_render::Stats> for RenderStats {
    fn from(stats: &figstudio_render::Stats) -> Self {
        let figstudio_render::Stats {
            nodes,
            instances,
            text_stored_glyphs,
            text_system_fonts,
            text_missing,
            text_truncation_ignored,
            missing_images,
            images_loading,
            missing_symbols,
            masks,
            shadows,
            inner_shadows,
            approximated_effects,
            blurs,
            skipped_effects,
            unsupported_paints,
            approximated_blends,
            approximated_paints,
            approximated_strokes,
            approximated_instance_ink,
            culled,
            budget_exhausted,
            depth_skipped,
            preparation_skipped,
            attempted_work,
            unrecorded_tiles,
            gpu_fallback_tiles,
        } = stats;
        Self {
            nodes: *nodes,
            instances: *instances,
            text_stored_glyphs: *text_stored_glyphs,
            text_system_fonts: *text_system_fonts,
            text_missing: *text_missing,
            text_truncation_ignored: *text_truncation_ignored,
            missing_images: *missing_images,
            images_loading: *images_loading,
            missing_symbols: *missing_symbols,
            masks: *masks,
            shadows: *shadows,
            inner_shadows: *inner_shadows,
            approximated_effects: *approximated_effects,
            blurs: *blurs,
            skipped_effects: *skipped_effects,
            unsupported_paints: *unsupported_paints,
            approximated_blends: *approximated_blends,
            approximated_paints: *approximated_paints,
            approximated_strokes: *approximated_strokes,
            approximated_instance_ink: *approximated_instance_ink,
            culled: *culled,
            budget_exhausted: *budget_exhausted,
            depth_skipped: *depth_skipped,
            preparation_skipped: *preparation_skipped,
            attempted_work: *attempted_work,
            unrecorded_tiles: *unrecorded_tiles,
            gpu_fallback_tiles: *gpu_fallback_tiles,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageStatus {
    Added,
    Removed,
    Changed,
    Unchanged,
}

/// One unmatched subtree root; descendants excludes the root itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRef {
    pub id: Guid,
    pub kind: String,
    pub name: String,
    pub path: Vec<String>,
    pub descendants: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeDiff {
    pub id: Guid,
    pub kind: String,
    pub name: String,
    pub path: Vec<String>,
    pub moved: Option<Move>,
    pub changes: Vec<PropertyChange>,
    pub text: Option<TextChange>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyChange {
    /// Object names and array indices separated by dots.
    pub field: String,
    pub old: Option<Val>,
    pub new: Option<Val>,
    pub category: Category,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    Text,
    Font,
    Fill,
    Stroke,
    Geometry,
    Layout,
    Effect,
    Component,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextChange {
    pub old: String,
    pub new: String,
    pub runs: Vec<RunChange>,
}

/// Stored style-run index and its leaf deltas; no rendered run resolution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunChange {
    /// Index into the stored style override table, not a character offset.
    pub index: usize,
    /// Leaf deltas within this stored style entry.
    pub changes: Vec<PropertyChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    /// Previous parent identity; None for a root.
    pub old_parent: Option<Guid>,
    /// Destination parent identity; None for a root.
    pub new_parent: Option<Guid>,
    /// Previous fractional sibling position.
    pub old_position: String,
    /// Destination fractional sibling position.
    pub new_position: String,
}
