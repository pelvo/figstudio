//! Unified semantic [`EditOp`] types and batch execution engine shared by the GUI editor, CLI,
//! and MCP.
//!
//! Every [`EditOp`] compiles against a staged copy of the document, runs the Auto-Layout &
//! geometry derive pass on affected ancestor containers at the end of the batch, validates all
//! resulting patches against the file's Kiwi schema, and commits as a single atomic [`Txn`] in
//! [`History`].

mod batch;
mod compile_components;
mod compile_props;
mod compile_tree;

pub use crate::edit::guid_wire;
pub use batch::{
    describe_edit_ops_markdown, execute_edit_ops, execute_import_image, execute_paste_html,
};

use serde::{Deserialize, Serialize};

use crate::Guid;
use crate::edit::instances::ComponentPropVal;
use crate::edit::text_runs::TextStyleRunPatch;
use crate::edit::vector_net::{BooleanOp, VectorNetwork};

/// Complete, self-describing semantic edit operation across all Figma capabilities.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Creates a parametric shape or container (`FRAME`, `SECTION`, `GROUP`,
    /// `ROUNDED_RECTANGLE`/`RECTANGLE`, `ELLIPSE`, `LINE`, `REGULAR_POLYGON`, `STAR`).
    CreateShape {
        #[serde(default, with = "guid_wire::opt")]
        id: Option<Guid>,
        kind: String,
        #[serde(with = "guid_wire")]
        parent: Guid,
        #[serde(default)]
        name: Option<String>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        fill_hex: Option<String>,
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        star_inner_scale: Option<f32>,
    },
    /// Creates a new `TEXT` layer.
    CreateText {
        #[serde(default, with = "guid_wire::opt")]
        id: Option<Guid>,
        #[serde(with = "guid_wire")]
        parent: Guid,
        x: f64,
        y: f64,
        characters: String,
        #[serde(default = "default_font_family")]
        font_family: String,
        #[serde(default = "default_font_style")]
        font_style: String,
        #[serde(default = "default_font_size")]
        font_size: f64,
        #[serde(default = "default_black_hex")]
        fill_hex: String,
    },
    /// Creates a new `CANVAS` page.
    CreatePage {
        #[serde(default, with = "guid_wire::opt")]
        id: Option<Guid>,
        name: String,
    },
    /// Deletes a node and all of its descendants.
    DeleteNode {
        #[serde(with = "guid_wire")]
        target: Guid,
    },
    /// Duplicates a node subtree under the same parent with an optional `(dx, dy)` offset.
    DuplicateNode {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default = "default_dup_dx")]
        dx: f64,
        #[serde(default = "default_dup_dy")]
        dy: f64,
    },
    /// Renames a node (`name`).
    Rename {
        #[serde(with = "guid_wire")]
        target: Guid,
        name: String,
    },
    /// Sets `visible` and/or `locked` and/or `opacity` and/or `blend_mode`.
    SetLayerProps {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        locked: Option<bool>,
        #[serde(default)]
        opacity: Option<f64>,
        #[serde(default)]
        blend_mode: Option<String>,
        #[serde(default)]
        clip_content: Option<bool>,
    },
    /// Moves a node to `(x, y)` in parent space (or translates by `(dx, dy)` when `relative`).
    Move {
        #[serde(with = "guid_wire")]
        target: Guid,
        x: f64,
        y: f64,
        #[serde(default)]
        relative: bool,
    },
    /// Resizes a node to `(width, height)`, propagating child constraints (`MIN`/`CENTER`/`MAX`/`STRETCH`/`SCALE`).
    Resize {
        #[serde(with = "guid_wire")]
        target: Guid,
        width: f64,
        height: f64,
    },
    /// Sets rotation in degrees (preserving current `(m02, m12)` translation).
    Rotate {
        #[serde(with = "guid_wire")]
        target: Guid,
        degrees: f64,
    },
    /// Reparents and/or reorders `target` under `new_parent` at `index` (`None` = append/topmost).
    Reparent {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(with = "guid_wire")]
        new_parent: Guid,
        #[serde(default)]
        index: Option<usize>,
    },
    /// Sets uniform or per-corner radii `[tl, tr, br, bl]` and optional squircle `smoothing` (`0..=1`).
    SetCornerRadii {
        #[serde(with = "guid_wire")]
        target: Guid,
        radii: [f32; 4],
        #[serde(default)]
        smoothing: Option<f32>,
    },
    /// Configures `horizontalConstraint` and/or `verticalConstraint` (`MIN`, `CENTER`, `MAX`, `STRETCH`, `SCALE`).
    SetConstraints {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        horizontal: Option<String>,
        #[serde(default)]
        vertical: Option<String>,
    },
    /// Configures Auto-Layout on a container (`NONE`, `HORIZONTAL`, `VERTICAL`, `GRID`).
    SetAutoLayout {
        #[serde(with = "guid_wire")]
        target: Guid,
        mode: String,
        #[serde(default)]
        spacing: Option<f64>,
        #[serde(default)]
        counter_spacing: Option<f64>,
        #[serde(default)]
        padding: Option<[f64; 4]>, // [top, right, bottom, left]
        #[serde(default)]
        primary_sizing: Option<String>, // FIXED | HUG (mapped to RESIZE_TO_FIT_WITH_IMPLICIT_SIZE)
        #[serde(default)]
        counter_sizing: Option<String>,
        #[serde(default)]
        primary_align: Option<String>, // MIN | CENTER | MAX | SPACE_BETWEEN
        #[serde(default)]
        counter_align: Option<String>, // MIN | CENTER | MAX | BASELINE
        #[serde(default)]
        wrap: Option<bool>,
    },
    /// Configures child sizing inside an Auto-Layout parent (`FIXED`, `FILL`, `ABSOLUTE`).
    SetAutoLayoutChild {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        primary_fill: Option<bool>,
        #[serde(default)]
        counter_stretch: Option<bool>,
        #[serde(default)]
        positioning: Option<String>, // AUTO | ABSOLUTE
        #[serde(default)]
        min_size: Option<[f64; 2]>,
        #[serde(default)]
        max_size: Option<[f64; 2]>,
    },
    /// Replaces solid fill colour (`#RRGGBB` / `#RRGGBBAA`), or clears fills when `hex` is `None`.
    SetSolidFill {
        #[serde(with = "guid_wire")]
        target: Guid,
        hex: Option<String>,
    },
    /// Sets an `IMAGE` fill referencing an imported 40-hex SHA-1 image hash.
    SetImageFill {
        #[serde(with = "guid_wire")]
        target: Guid,
        sha1_hex: String,
        #[serde(default = "default_scale_fill")]
        scale_mode: String,
        #[serde(default = "default_one")]
        opacity: f64,
    },
    /// Sets stroke colour, weight, and alignment (`INSIDE`, `CENTER`, `OUTSIDE`).
    SetStroke {
        #[serde(with = "guid_wire")]
        target: Guid,
        hex: Option<String>,
        #[serde(default)]
        weight: Option<f64>,
        #[serde(default)]
        align: Option<String>,
    },
    /// Replaces effects with a shadow and/or blur, or clears effects when empty.
    SetEffects {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        drop_shadow: Option<ShadowSpec>,
        #[serde(default)]
        blur_radius: Option<f64>,
        #[serde(default)]
        background_blur: bool,
    },
    /// Replaces the entire string of a `TEXT` layer (or an inner `TEXT` override when `instance` is set).
    SetText {
        #[serde(with = "guid_wire")]
        target: Guid,
        characters: String,
    },
    /// Splices UTF-16 characters in a `TEXT` layer while preserving mixed style runs.
    SpliceText {
        #[serde(with = "guid_wire")]
        target: Guid,
        utf16_start: usize,
        utf16_delete_len: usize,
        insert: String,
    },
    /// Applies rich typography overrides to a UTF-16 substring `[utf16_start..utf16_end]`.
    StyleTextRange {
        #[serde(with = "guid_wire")]
        target: Guid,
        utf16_start: usize,
        utf16_end: usize,
        style: TextStyleRunPatch,
    },
    /// Sets top-level typography properties (`font_family`, `font_style`, `font_size`, `align_horizontal`, `auto_resize`).
    SetTypography {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        font_family: Option<String>,
        #[serde(default)]
        font_style: Option<String>,
        #[serde(default)]
        font_size: Option<f64>,
        #[serde(default)]
        align_horizontal: Option<String>,
        #[serde(default)]
        auto_resize: Option<String>,
    },
    /// Sets an override on a descendant inside an `INSTANCE` (`override_path` of `overrideKey` GUIDs).
    SetInstanceOverride {
        #[serde(with = "guid_wire")]
        instance: Guid,
        #[serde(default, with = "guid_wire::vec")]
        override_path: Vec<Guid>,
        #[serde(default)]
        characters: Option<String>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        fill_hex: Option<String>,
        #[serde(default)]
        opacity: Option<f64>,
    },
    /// Resets overrides on an `INSTANCE` (all, one path, or one field).
    ResetInstanceOverride {
        #[serde(with = "guid_wire")]
        instance: Guid,
        #[serde(default, with = "guid_wire::opt_vec")]
        override_path: Option<Vec<Guid>>,
        #[serde(default)]
        field: Option<String>,
    },
    /// Sets a component property (`BOOL`, `TEXT`, `INSTANCE_SWAP`) on an `INSTANCE`.
    SetComponentProp {
        #[serde(with = "guid_wire")]
        instance: Guid,
        prop: String,
        #[serde(flatten)]
        val: ComponentPropVal,
    },
    /// Switches an `INSTANCE` to another variant in its `STATE_GROUP` by `("Property", "Value")` pairs.
    SwitchVariant {
        #[serde(with = "guid_wire")]
        instance: Guid,
        variants: Vec<(String, String)>,
    },
    /// Swaps an `INSTANCE`'s master `SYMBOL`.
    SwapInstance {
        #[serde(with = "guid_wire")]
        instance: Guid,
        #[serde(with = "guid_wire")]
        symbol: Guid,
    },
    /// Detaches an `INSTANCE` into a standalone `FRAME` subtree.
    DetachInstance {
        #[serde(with = "guid_wire")]
        instance: Guid,
    },
    /// Converts a `FRAME` into a `SYMBOL` component, or instantiates an existing `SYMBOL`.
    CreateComponent {
        #[serde(with = "guid_wire")]
        frame: Guid,
    },
    InstantiateComponent {
        #[serde(with = "guid_wire")]
        symbol: Guid,
        #[serde(with = "guid_wire")]
        parent: Guid,
        x: f64,
        y: f64,
    },
    /// Creates or binds a local style (`fill`, `stroke`, `text`, `effect`, `grid`).
    BindStyle {
        #[serde(with = "guid_wire")]
        target: Guid,
        slot: String,
        #[serde(default, with = "guid_wire::opt")]
        style: Option<Guid>,
    },
    CreateFillStyle {
        name: String,
        hex: String,
    },
    /// Binds a `COLOR` variable to a paint, sets explicit container mode, or updates a variable's mode value.
    BindPaintVariable {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default = "default_fill_paints")]
        field: String,
        #[serde(default)]
        paint_index: usize,
        #[serde(default, with = "guid_wire::opt")]
        variable: Option<Guid>,
    },
    SetVariableMode {
        #[serde(with = "guid_wire")]
        container: Guid,
        #[serde(with = "guid_wire")]
        collection: Guid,
        #[serde(default, with = "guid_wire::opt")]
        mode: Option<Guid>,
    },
    SetVariableColor {
        #[serde(with = "guid_wire")]
        variable: Guid,
        #[serde(with = "guid_wire")]
        mode: Guid,
        hex: String,
    },
    /// Replaces a `VECTOR` node's `vectorNetworkBlob` (and rebuilds its `fillGeometry`).
    SetVectorNetwork {
        #[serde(with = "guid_wire")]
        target: Guid,
        network: VectorNetwork,
    },
    /// Combines two shape/vector nodes under a new `BOOLEAN_OPERATION` (`UNION`, `SUBTRACT`, `INTERSECT`, `EXCLUDE`).
    BooleanCombine {
        operation: BooleanOp,
        #[serde(with = "guid_wire")]
        a: Guid,
        #[serde(with = "guid_wire")]
        b: Guid,
        #[serde(default)]
        name: Option<String>,
    },
    /// Raw escape hatch: sets or unsets arbitrary Kiwi-validated `NodeChange` fields directly.
    PatchRaw {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        set: serde_json::Map<String, serde_json::Value>,
        #[serde(default)]
        unset: Vec<String>,
    },
}

/// Drop/inner shadow parameters for [`EditOp::SetEffects`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShadowSpec {
    #[serde(default)]
    pub inner: bool,
    pub rgba: [f64; 4],
    pub offset: [f64; 2],
    pub radius: f64,
    #[serde(default)]
    pub spread: f64,
}

/// Summary returned after running (or dry-running) a batch of [`EditOp`]s.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EditBatchReport {
    pub applied: bool,
    pub revision: u64,
    #[serde(default, with = "guid_wire::vec")]
    pub created_ids: Vec<Guid>,
    #[serde(default, with = "guid_wire::vec")]
    pub touched_nodes: Vec<Guid>,
    #[serde(default, with = "guid_wire::vec")]
    pub dirty_pages: Vec<Guid>,
    pub auto_layout_adjustments: usize,
    pub undo_depth: usize,
    pub redo_depth: usize,
}

fn default_font_family() -> String {
    "Inter".into()
}
fn default_font_style() -> String {
    "Regular".into()
}
fn default_font_size() -> f64 {
    16.0
}
fn default_black_hex() -> String {
    "#111111".into()
}
fn default_dup_dx() -> f64 {
    24.0
}
fn default_dup_dy() -> f64 {
    24.0
}
fn default_scale_fill() -> String {
    "FILL".into()
}
fn default_one() -> f64 {
    1.0
}
fn default_fill_paints() -> String {
    "fillPaints".into()
}
