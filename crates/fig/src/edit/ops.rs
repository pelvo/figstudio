//! Unified semantic [`EditOp`] batch execution engine shared by the GUI editor, CLI, and MCP.
//!
//! Every [`EditOp`] compiles against a staged copy of the document, runs the Auto-Layout &
//! geometry derive pass automatically at the end of the batch, validates all resulting patches
//! against the file's Kiwi schema, and commits as a single atomic [`Txn`] in [`History`].

pub use crate::edit::guid_wire;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::clipboard::{decode_clipboard_html, paste_kiwi_into};
use crate::edit::autolayout::{append_auto_layout_patches, compute_auto_layout};
use crate::edit::geometry::{
    CreateShapeParams, PathCmd, create_shape_node, encode_path_commands, make_path_entry,
    matrix_2x3, rebuild_shape_geometry, resize_node_with_constraints,
};
use crate::edit::instances::{
    ComponentPropVal, convert_frame_to_component, detach_instance, instantiate_component,
    reset_instance_override, set_component_prop_assignment, set_instance_override,
    swap_instance_symbol, switch_instance_variant,
};
use crate::edit::styles_vars::{
    bind_paint_color_variable, bind_style, blur_effect, create_local_style, hex_paint, image_paint,
    import_image_bytes, set_explicit_variable_mode, set_variable_mode_value, shadow_effect,
};
use crate::edit::text_runs::{
    TextStyleRunPatch, apply_text_range_style, commit_text_data, create_text_node, set_text_plain,
    splice_text_utf16,
};
use crate::edit::vector_net::{
    BooleanOp, VectorNetwork, boolean_combine, decode_vector_network, encode_vector_network,
    network_to_fill_paths,
};
use crate::edit::{Dirty, GuidAlloc, History, Txn, fractional};
use crate::{Document, FigError, Guid, Obj, Val, intern};

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

/// Executes `ops` atomically against `doc` and `history` (or simulates without mutating when
/// `dry_run` is `true`).
pub fn execute_edit_ops(
    doc: &mut Document,
    history: &mut History,
    ops: &[EditOp],
    label: Option<&str>,
    coalesce_key: Option<&str>,
    dry_run: bool,
) -> Result<(EditBatchReport, Dirty), FigError> {
    if ops.is_empty() {
        return Ok((
            EditBatchReport {
                applied: !dry_run,
                revision: doc.revision,
                undo_depth: history.undo_len(),
                redo_depth: history.redo_len(),
                ..Default::default()
            },
            Dirty::default(),
        ));
    }

    // Stage on a lightweight clone so multi-op batches can reference nodes created earlier in
    // the same batch and run the final Auto-Layout derive pass over the post-op state!
    let mut staged = clone_document_shell(doc);
    let mut alloc = GuidAlloc::for_document(&staged);
    let mut batch_txn = Txn::new(label.unwrap_or("Apply edits"));
    if let Some(ck) = coalesce_key {
        batch_txn = batch_txn.with_coalesce(ck);
    }

    let mut created_ids = Vec::new();
    let mut touched_for_layout: Vec<Guid> = Vec::new();

    for op in ops {
        let mut step_txn = Txn::new("step");
        compile_single_op(
            &staged,
            &mut step_txn,
            &mut alloc,
            op,
            &mut created_ids,
            &mut touched_for_layout,
        )?;
        if !step_txn.is_empty() {
            staged.apply_txn(&step_txn)?;
            merge_step_into_batch(&mut batch_txn, step_txn);
        }
    }

    // Automatic Derive Pass: solve Auto-Layout across affected subtrees so HUG/FILL/spacing
    // stay 100% consistent after every edit!
    let mut al_adjustments = 0usize;
    let mut layout_roots: Vec<Option<Guid>> = Vec::new();
    if touched_for_layout.len() > 8 {
        layout_roots.push(None);
    } else {
        for &tid in &touched_for_layout {
            if staged.node(tid).is_some() && !layout_roots.contains(&Some(tid)) {
                layout_roots.push(Some(tid));
            }
        }
    }
    for scope in layout_roots {
        let mut al_txn = Txn::new("autolayout");
        al_adjustments += compute_auto_layout(&staged, scope).len();
        append_auto_layout_patches(&staged, scope, &mut al_txn);
        if !al_txn.is_empty() {
            staged.apply_txn(&al_txn)?;
            merge_step_into_batch(&mut batch_txn, al_txn);
        }
    }

    if dry_run {
        let mut dirty_pages: Vec<Guid> = batch_txn
            .patches
            .iter()
            .filter_map(|(id, _)| staged.page_of(*id).or_else(|| doc.page_of(*id)))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        dirty_pages.sort();
        let mut touched: Vec<Guid> = batch_txn
            .patches
            .iter()
            .map(|(id, _)| *id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        touched.sort();
        return Ok((
            EditBatchReport {
                applied: false,
                revision: doc.revision,
                created_ids,
                touched_nodes: touched,
                dirty_pages,
                auto_layout_adjustments: al_adjustments,
                undo_depth: history.undo_len(),
                redo_depth: history.redo_len(),
            },
            Dirty::default(),
        ));
    }

    let dirty = history.execute(doc, batch_txn)?;
    let mut touched_nodes: Vec<Guid> = dirty.nodes.iter().copied().collect();
    touched_nodes.sort();
    let mut dirty_pages: Vec<Guid> = dirty.pages.iter().copied().collect();
    dirty_pages.sort();

    Ok((
        EditBatchReport {
            applied: true,
            revision: doc.revision,
            created_ids,
            touched_nodes,
            dirty_pages,
            auto_layout_adjustments: al_adjustments,
            undo_depth: history.undo_len(),
            redo_depth: history.redo_len(),
        },
        dirty,
    ))
}

/// Imports an image into `doc` via `history` and returns `(sha1_hex, Dirty)`.
pub fn execute_import_image(
    doc: &mut Document,
    history: &mut History,
    bytes: Vec<u8>,
) -> Result<(String, Dirty), FigError> {
    let mut txn = Txn::new("Import image");
    let hash = import_image_bytes(&mut txn, bytes)?;
    let dirty = history.execute(doc, txn)?;
    Ok((hash, dirty))
}

/// Pastes Figma HTML clipboard (or base64 kiwi) into `doc` via `history`.
pub fn execute_paste_html(
    doc: &mut Document,
    history: &mut History,
    html: &str,
    target_parent: Guid,
    offset: (f64, f64),
) -> Result<(Vec<Guid>, Dirty), FigError> {
    let payload = decode_clipboard_html(html)?;
    let (mut txn, roots) = paste_kiwi_into(doc, &payload.kiwi_bytes, target_parent, offset)?;
    let dirty = history.execute(doc, txn.clone())?;
    txn.patches.clear();
    // Run Auto-Layout if pasted into an Auto-Layout frame!
    let mut al_txn = Txn::new("Paste Auto-Layout");
    append_auto_layout_patches(doc, Some(target_parent), &mut al_txn);
    if !al_txn.is_empty() {
        let _ = history.execute(doc, al_txn)?;
    }
    Ok((roots, dirty))
}

/// Returns concise JSON-schema-like documentation of all supported [`EditOp`] variants for MCP agents.
pub fn describe_edit_ops_markdown() -> &'static str {
    concat!(
        "# Supported `apply_edits` Operations (`op` tag)\n\n",
        "- `create_shape`: `{ kind: FRAME|SECTION|GROUP|ROUNDED_RECTANGLE|ELLIPSE|LINE|REGULAR_POLYGON|STAR, parent, name?, x, y, width, height, corner_radius?, fill_hex?, count?, star_inner_scale? }`\n",
        "- `create_text`: `{ parent, x, y, characters, font_family?, font_style?, font_size?, fill_hex? }`\n",
        "- `create_page`: `{ name }`\n",
        "- `delete_node`: `{ target }`\n",
        "- `duplicate_node`: `{ target, dx?, dy? }`\n",
        "- `rename`: `{ target, name }`\n",
        "- `set_layer_props`: `{ target, visible?, locked?, opacity?, blend_mode?, clip_content? }`\n",
        "- `move`: `{ target, x, y, relative? }`\n",
        "- `resize`: `{ target, width, height }` (propagates constraints & rebuilds geometry)\n",
        "- `rotate`: `{ target, degrees }`\n",
        "- `reparent`: `{ target, new_parent, index? }`\n",
        "- `set_corner_radii`: `{ target, radii: [tl, tr, br, bl], smoothing? }`\n",
        "- `set_constraints`: `{ target, horizontal?: MIN|CENTER|MAX|STRETCH|SCALE, vertical? }`\n",
        "- `set_auto_layout`: `{ target, mode: NONE|HORIZONTAL|VERTICAL|GRID, spacing?, counter_spacing?, padding?: [top,right,bottom,left], primary_sizing?: FIXED|HUG, counter_sizing?: FIXED|HUG, primary_align?: MIN|CENTER|MAX|SPACE_BETWEEN, counter_align?: MIN|CENTER|MAX|BASELINE, wrap? }`\n",
        "- `set_auto_layout_child`: `{ target, primary_fill?, counter_stretch?, positioning?: AUTO|ABSOLUTE, min_size?: [w,h], max_size?: [w,h] }`\n",
        "- `set_solid_fill`: `{ target, hex: \"#RRGGBB\" | null }`\n",
        "- `set_image_fill`: `{ target, sha1_hex, scale_mode?: FILL|FIT|CROP|TILE, opacity? }`\n",
        "- `set_stroke`: `{ target, hex: \"#RRGGBB\" | null, weight?, align?: INSIDE|CENTER|OUTSIDE }`\n",
        "- `set_effects`: `{ target, drop_shadow?: { inner?, rgba:[r,g,b,a], offset:[x,y], radius, spread? }, blur_radius?, background_blur? }`\n",
        "- `set_text`: `{ target, characters }`\n",
        "- `splice_text`: `{ target, utf16_start, utf16_delete_len, insert }`\n",
        "- `style_text_range`: `{ target, utf16_start, utf16_end, style: { font_family?, font_style?, font_size?, fill_hex?, text_decoration?, text_case?, letter_spacing_px?, line_height_px? } }`\n",
        "- `set_typography`: `{ target, font_family?, font_style?, font_size?, align_horizontal?, auto_resize? }`\n",
        "- `set_instance_override`: `{ instance, override_path: [\"1:2\", ...], characters?, visible?, fill_hex?, opacity? }`\n",
        "- `reset_instance_override`: `{ instance, override_path?, field? }`\n",
        "- `set_component_prop`: `{ instance, prop, kind: bool|text|instance_swap, value }`\n",
        "- `switch_variant`: `{ instance, variants: [[\"Size\", \"Large\"]] }`\n",
        "- `swap_instance`: `{ instance, symbol }`\n",
        "- `detach_instance`: `{ instance }`\n",
        "- `create_component`: `{ frame }` / `instantiate_component`: `{ symbol, parent, x, y }`\n",
        "- `bind_style`: `{ target, slot: fill|stroke|text|effect|grid, style: \"1:2\" | null }` / `create_fill_style`: `{ name, hex }`\n",
        "- `bind_paint_variable`: `{ target, field?: fillPaints|strokePaints, paint_index?, variable: \"1:2\" | null }`\n",
        "- `set_variable_mode`: `{ container, collection, mode: \"1:2\" | null }`\n",
        "- `set_variable_color`: `{ variable, mode, hex }`\n",
        "- `set_vector_network`: `{ target, network }` / `boolean_combine`: `{ op: Union|Subtract|Intersect|Exclude, a, b, name? }`\n",
        "- `patch_raw`: `{ target, set: {...}, unset: [...] }`\n"
    )
}

fn compile_single_op(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: &EditOp,
    created_ids: &mut Vec<Guid>,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::CreateShape {
            id,
            kind,
            parent,
            name,
            x,
            y,
            width,
            height,
            corner_radius,
            fill_hex,
            count,
            star_inner_scale,
        } => {
            let gid = id.unwrap_or_else(|| alloc.next_guid());
            create_shape_node(
                doc,
                txn,
                gid,
                &CreateShapeParams {
                    kind: kind.clone(),
                    parent: *parent,
                    name: name.clone(),
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                    corner_radius: *corner_radius,
                    fill_hex: fill_hex.clone(),
                    count: *count,
                    star_inner_scale: *star_inner_scale,
                },
            )?;
            created_ids.push(gid);
            layout_touched.push(*parent);
        }
        EditOp::CreateText {
            id,
            parent,
            x,
            y,
            characters,
            font_family,
            font_style,
            font_size,
            fill_hex,
        } => {
            let gid = id.unwrap_or_else(|| alloc.next_guid());
            create_text_node(
                doc,
                txn,
                gid,
                *parent,
                *x,
                *y,
                characters,
                font_family,
                font_style,
                *font_size,
                fill_hex,
            )?;
            created_ids.push(gid);
            layout_touched.push(*parent);
        }
        EditOp::CreatePage { id, name } => {
            let root = doc
                .root
                .ok_or_else(|| FigError::malformed("document has no root"))?;
            let gid = id.unwrap_or_else(|| alloc.next_guid());
            let last_pos = doc
                .pages
                .last()
                .and_then(|pid| doc.node(*pid))
                .map(|n| n.position.as_str());
            let pos = fractional::between(last_pos, None);
            txn.create_node(
                gid,
                Val::Obj(Obj::from([
                    ("guid", gid.to_val()),
                    ("phase", Val::Enum(intern("CREATED"))),
                    ("type", Val::Enum(intern("CANVAS"))),
                    ("name", Val::Str(name.clone())),
                    ("visible", Val::Bool(true)),
                    (
                        "parentIndex",
                        Val::Obj(Obj::from([
                            ("guid", root.to_val()),
                            ("position", Val::Str(pos)),
                        ])),
                    ),
                ])),
            );
            created_ids.push(gid);
        }
        EditOp::DeleteNode { target } => {
            let Some(node) = doc.node(*target) else {
                return Err(FigError::malformed(format!("missing node {target}")));
            };
            if let Some(p) = node.parent {
                layout_touched.push(p);
            }
            // Delete descendants bottom-up then `target`.
            let mut sub = Vec::new();
            collect_postorder(doc, *target, &mut sub);
            for id in sub {
                txn.delete_node(id);
            }
        }
        EditOp::DuplicateNode { target, dx, dy } => {
            let Some(orig) = doc.node(*target) else {
                return Err(FigError::malformed(format!("missing node {target}")));
            };
            let parent = orig
                .parent
                .ok_or_else(|| FigError::malformed("cannot duplicate root node"))?;
            let kiwi = crate::clipboard::encode_subset_kiwi(doc, &[*target], 0, "dup")?;
            let (paste_txn, roots) = paste_kiwi_into(doc, &kiwi, parent, (*dx, *dy))?;
            created_ids.extend_from_slice(&roots);
            merge_step_into_batch(txn, paste_txn);
            layout_touched.push(parent);
        }
        EditOp::Rename { target, name } => {
            ensure_exists(doc, *target)?;
            txn.set_field(*target, "name", Val::Str(name.clone()));
        }
        EditOp::SetLayerProps {
            target,
            visible,
            locked,
            opacity,
            blend_mode,
            clip_content,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            if let Some(v) = visible {
                set.insert("visible", Val::Bool(*v));
                if let Some(p) = node.parent {
                    layout_touched.push(p);
                }
            }
            if let Some(l) = locked {
                set.insert("locked", Val::Bool(*l));
            }
            if let Some(op) = opacity {
                set.insert("opacity", Val::Float(op.clamp(0.0, 1.0)));
            }
            if let Some(bm) = blend_mode {
                set.insert("blendMode", Val::Enum(intern(bm)));
            }
            if let Some(clip) = clip_content {
                set.insert("frameMaskDisabled", Val::Bool(!*clip));
            }
            txn.modify(*target, set, Vec::new());
        }
        EditOp::Move {
            target,
            x,
            y,
            relative,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut tf = node
                .data
                .get("transform")
                .cloned()
                .unwrap_or_else(|| matrix_2x3(0.0, 0.0, 0.0))
                .into_owned();
            let (nx, ny) = if *relative {
                let cx = tf.num("m02").map_or(0.0, f64::from);
                let cy = tf.num("m12").map_or(0.0, f64::from);
                (cx + *x, cy + *y)
            } else {
                (*x, *y)
            };
            tf.set("m02", Val::Float(nx));
            tf.set("m12", Val::Float(ny));
            txn.set_field(*target, "transform", tf);
        }
        EditOp::Resize {
            target,
            width,
            height,
        } => {
            resize_node_with_constraints(doc, txn, *target, *width, *height)?;
            layout_touched.push(*target);
        }
        EditOp::Rotate { target, degrees } => {
            let node = ensure_exists(doc, *target)?;
            let cx = node
                .data
                .path(&["transform", "m02"])
                .and_then(Val::as_f32)
                .map_or(0.0, f64::from);
            let cy = node
                .data
                .path(&["transform", "m12"])
                .and_then(Val::as_f32)
                .map_or(0.0, f64::from);
            txn.set_field(*target, "transform", matrix_2x3(cx, cy, *degrees));
        }
        EditOp::Reparent {
            target,
            new_parent,
            index,
        } => {
            let node = ensure_exists(doc, *target)?;
            ensure_exists(doc, *new_parent)?;
            if let Some(old_p) = node.parent {
                layout_touched.push(old_p);
            }
            layout_touched.push(*new_parent);
            let siblings: Vec<Guid> = doc
                .children(*new_parent)
                .iter()
                .copied()
                .filter(|&id| id != *target)
                .collect();
            let idx = index.unwrap_or(siblings.len()).min(siblings.len());
            let prev = idx
                .checked_sub(1)
                .and_then(|i| siblings.get(i))
                .and_then(|id| doc.node(*id))
                .map(|n| n.position.as_str());
            let next = siblings
                .get(idx)
                .and_then(|id| doc.node(*id))
                .map(|n| n.position.as_str());
            let pos = fractional::between(prev, next);
            txn.set_field(
                *target,
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", new_parent.to_val()),
                    ("position", Val::Str(pos)),
                ])),
            );
        }
        EditOp::SetCornerRadii {
            target,
            radii,
            smoothing,
        } => {
            let node = ensure_exists(doc, *target)?;
            let uniform = (radii[0] - radii[1]).abs() < 1e-4
                && (radii[1] - radii[2]).abs() < 1e-4
                && (radii[2] - radii[3]).abs() < 1e-4;
            let mut set = Obj::new();
            set.insert("cornerRadius", Val::Float(f64::from(radii[0].max(0.0))));
            set.insert("rectangleCornerRadiiIndependent", Val::Bool(!uniform));
            set.insert(
                "rectangleTopLeftCornerRadius",
                Val::Float(f64::from(radii[0].max(0.0))),
            );
            set.insert(
                "rectangleTopRightCornerRadius",
                Val::Float(f64::from(radii[1].max(0.0))),
            );
            set.insert(
                "rectangleBottomRightCornerRadius",
                Val::Float(f64::from(radii[2].max(0.0))),
            );
            set.insert(
                "rectangleBottomLeftCornerRadius",
                Val::Float(f64::from(radii[3].max(0.0))),
            );
            if let Some(sm) = smoothing {
                set.insert("cornerSmoothing", Val::Float(f64::from(sm.clamp(0.0, 1.0))));
            }
            let mut preview = node.data.clone().into_owned();
            for (k, v) in set.iter() {
                preview.set(k, v.clone());
            }
            if node.data.get("fillGeometry").is_some()
                && let Some(cmds) = rebuild_shape_geometry(&node.kind, &preview)
            {
                let blob_idx = doc.blobs.len() + txn.add_blobs.len();
                txn.add_blobs.push(encode_path_commands(&cmds));
                set.insert(
                    "fillGeometry",
                    Val::Arr(vec![make_path_entry(blob_idx, false)]),
                );
            }
            txn.modify(*target, set, Vec::new());
        }
        EditOp::SetConstraints {
            target,
            horizontal,
            vertical,
        } => {
            ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            if let Some(h) = horizontal {
                set.insert("horizontalConstraint", Val::Enum(intern(h)));
            }
            if let Some(v) = vertical {
                set.insert("verticalConstraint", Val::Enum(intern(v)));
            }
            txn.modify(*target, set, Vec::new());
        }
        EditOp::SetAutoLayout {
            target,
            mode,
            spacing,
            counter_spacing,
            padding,
            primary_sizing,
            counter_sizing,
            primary_align,
            counter_align,
            wrap,
        } => {
            ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            set.insert("stackMode", Val::Enum(intern(mode)));
            if let Some(sp) = spacing {
                set.insert("stackSpacing", Val::Float(*sp));
            }
            if let Some(csp) = counter_spacing {
                set.insert("stackCounterSpacing", Val::Float(*csp));
            }
            if let Some([top, right, bottom, left]) = padding {
                set.insert("stackVerticalPadding", Val::Float(*top));
                set.insert("stackPaddingRight", Val::Float(*right));
                set.insert("stackPaddingBottom", Val::Float(*bottom));
                set.insert("stackHorizontalPadding", Val::Float(*left));
            }
            fn map_sizing(s: &str) -> &str {
                match s {
                    "HUG" => "RESIZE_TO_FIT_WITH_IMPLICIT_SIZE",
                    other => other,
                }
            }
            if let Some(ps) = primary_sizing {
                set.insert("stackPrimarySizing", Val::Enum(intern(map_sizing(ps))));
            }
            if let Some(cs) = counter_sizing {
                set.insert("stackCounterSizing", Val::Enum(intern(map_sizing(cs))));
            }
            if let Some(pa) = primary_align {
                set.insert("stackPrimaryAlignItems", Val::Enum(intern(pa)));
            }
            if let Some(ca) = counter_align {
                set.insert("stackCounterAlignItems", Val::Enum(intern(ca)));
            }
            if let Some(w) = wrap {
                set.insert(
                    "stackWrap",
                    Val::Enum(intern(if *w { "WRAP" } else { "NO_WRAP" })),
                );
            }
            txn.modify(*target, set, Vec::new());
            layout_touched.push(*target);
        }
        EditOp::SetAutoLayoutChild {
            target,
            primary_fill,
            counter_stretch,
            positioning,
            min_size,
            max_size,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            let mut unset = Vec::new();
            if let Some(pf) = primary_fill {
                set.insert(
                    "stackChildPrimaryGrow",
                    Val::Float(if *pf { 1.0 } else { 0.0 }),
                );
            }
            if let Some(cs) = counter_stretch {
                set.insert(
                    "stackChildAlignSelf",
                    Val::Enum(intern(if *cs { "STRETCH" } else { "AUTO" })),
                );
            }
            if let Some(pos) = positioning {
                set.insert("stackPositioning", Val::Enum(intern(pos)));
            }
            if let Some([mw, mh]) = min_size {
                set.insert(
                    "minSize",
                    Val::Obj(Obj::from([(
                        "value",
                        Val::Obj(Obj::from([("x", Val::Float(*mw)), ("y", Val::Float(*mh))])),
                    )])),
                );
            }
            if let Some([xw, xh]) = max_size {
                set.insert(
                    "maxSize",
                    Val::Obj(Obj::from([(
                        "value",
                        Val::Obj(Obj::from([("x", Val::Float(*xw)), ("y", Val::Float(*xh))])),
                    )])),
                );
            } else if max_size.is_none() {
                let _ = &mut unset;
            }
            txn.modify(*target, set, unset);
            if let Some(p) = node.parent {
                layout_touched.push(p);
            }
        }
        EditOp::SetSolidFill { target, hex } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            let mut unset = Vec::new();
            match hex {
                Some(h) => set.insert("fillPaints", Val::Arr(vec![hex_paint(h)?])),
                None => set.insert("fillPaints", Val::Arr(Vec::new())),
            };
            if node.data.get("styleIdForFill").is_some() {
                unset.push("styleIdForFill".into());
            }
            if node.data.get("inheritFillStyleID").is_some() {
                unset.push("inheritFillStyleID".into());
            }
            txn.modify(*target, set, unset);
        }
        EditOp::SetImageFill {
            target,
            sha1_hex,
            scale_mode,
            opacity,
        } => {
            ensure_exists(doc, *target)?;
            let paint = image_paint(sha1_hex, scale_mode, *opacity)?;
            txn.set_field(*target, "fillPaints", Val::Arr(vec![paint]));
        }
        EditOp::SetStroke {
            target,
            hex,
            weight,
            align,
        } => {
            ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            match hex {
                Some(h) => {
                    set.insert("strokePaints", Val::Arr(vec![hex_paint(h)?]));
                }
                None => {
                    set.insert("strokePaints", Val::Arr(Vec::new()));
                }
            }
            if let Some(w) = weight {
                set.insert("strokeWeight", Val::Float(w.max(0.0)));
            }
            if let Some(a) = align {
                set.insert("strokeAlign", Val::Enum(intern(a)));
            }
            txn.modify(*target, set, Vec::new());
        }
        EditOp::SetEffects {
            target,
            drop_shadow,
            blur_radius,
            background_blur,
        } => {
            ensure_exists(doc, *target)?;
            let mut effs = Vec::new();
            if let Some(sh) = drop_shadow {
                effs.push(shadow_effect(
                    sh.inner,
                    sh.rgba,
                    (sh.offset[0], sh.offset[1]),
                    sh.radius,
                    sh.spread,
                ));
            }
            if let Some(r) = blur_radius {
                effs.push(blur_effect(*background_blur, *r));
            }
            txn.set_field(*target, "effects", Val::Arr(effs));
        }
        EditOp::SetText { target, characters } => {
            set_text_plain(doc, txn, *target, characters)?;
            layout_touched.push(*target);
        }
        EditOp::SpliceText {
            target,
            utf16_start,
            utf16_delete_len,
            insert,
        } => {
            let node = ensure_exists(doc, *target)?;
            let base = node
                .data
                .get("textData")
                .cloned()
                .unwrap_or_else(|| Val::Obj(Obj::new()));
            let next = splice_text_utf16(&base, *utf16_start, *utf16_delete_len, insert)?;
            commit_text_data(doc, txn, *target, next)?;
            layout_touched.push(*target);
        }
        EditOp::StyleTextRange {
            target,
            utf16_start,
            utf16_end,
            style,
        } => {
            let node = ensure_exists(doc, *target)?;
            let base = node
                .data
                .get("textData")
                .cloned()
                .unwrap_or_else(|| Val::Obj(Obj::new()));
            let next = apply_text_range_style(&base, *utf16_start, *utf16_end, style)?;
            commit_text_data(doc, txn, *target, next)?;
            layout_touched.push(*target);
        }
        EditOp::SetTypography {
            target,
            font_family,
            font_style,
            font_size,
            align_horizontal,
            auto_resize,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            if font_family.is_some() || font_style.is_some() {
                let fam = font_family
                    .clone()
                    .or_else(|| {
                        node.data
                            .path(&["fontName", "family"])
                            .and_then(Val::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| "Inter".into());
                let sty = font_style
                    .clone()
                    .or_else(|| {
                        node.data
                            .path(&["fontName", "style"])
                            .and_then(Val::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| "Regular".into());
                set.insert(
                    "fontName",
                    Val::Obj(Obj::from([
                        ("family", Val::Str(fam)),
                        ("style", Val::Str(sty)),
                        ("postscript", Val::Str(String::new())),
                    ])),
                );
            }
            if let Some(sz) = font_size {
                set.insert("fontSize", Val::Float(sz.max(1.0)));
            }
            if let Some(ah) = align_horizontal {
                set.insert("textAlignHorizontal", Val::Enum(intern(ah)));
            }
            if let Some(ar) = auto_resize {
                set.insert("textAutoResize", Val::Enum(intern(ar)));
            }
            let unset = if node.data.get("derivedTextData").is_some() {
                vec!["derivedTextData".to_string()]
            } else {
                Vec::new()
            };
            txn.modify(*target, set, unset);
            layout_touched.push(*target);
        }
        EditOp::SetInstanceOverride {
            instance,
            override_path,
            characters,
            visible,
            fill_hex,
            opacity,
        } => {
            let mut fields = Obj::new();
            let mut unset = Vec::new();
            if let Some(chars) = characters {
                fields.insert(
                    "textData",
                    Val::Obj(Obj::from([("characters", Val::Str(chars.clone()))])),
                );
                unset.push("derivedTextData".into());
            }
            if let Some(v) = visible {
                fields.insert("visible", Val::Bool(*v));
            }
            if let Some(h) = fill_hex {
                fields.insert("fillPaints", Val::Arr(vec![hex_paint(h)?]));
            }
            if let Some(op) = opacity {
                fields.insert("opacity", Val::Float(op.clamp(0.0, 1.0)));
            }
            set_instance_override(doc, txn, *instance, override_path, fields, &unset)?;
            layout_touched.push(*instance);
        }
        EditOp::ResetInstanceOverride {
            instance,
            override_path,
            field,
        } => {
            reset_instance_override(
                doc,
                txn,
                *instance,
                override_path.as_deref(),
                field.as_deref(),
            )?;
            layout_touched.push(*instance);
        }
        EditOp::SetComponentProp {
            instance,
            prop,
            val,
        } => {
            set_component_prop_assignment(doc, txn, *instance, prop, val.clone())?;
            layout_touched.push(*instance);
        }
        EditOp::SwitchVariant { instance, variants } => {
            switch_instance_variant(doc, txn, *instance, variants)?;
            layout_touched.push(*instance);
        }
        EditOp::SwapInstance { instance, symbol } => {
            swap_instance_symbol(doc, txn, *instance, *symbol)?;
            layout_touched.push(*instance);
        }
        EditOp::DetachInstance { instance } => {
            let new_frame = detach_instance(doc, txn, alloc, *instance)?;
            created_ids.push(new_frame);
            layout_touched.push(new_frame);
        }
        EditOp::CreateComponent { frame } => {
            convert_frame_to_component(doc, txn, *frame)?;
        }
        EditOp::InstantiateComponent {
            symbol,
            parent,
            x,
            y,
        } => {
            let inst_id = instantiate_component(doc, txn, alloc, *symbol, *parent, *x, *y)?;
            created_ids.push(inst_id);
            layout_touched.push(*parent);
        }
        EditOp::BindStyle {
            target,
            slot,
            style,
        } => {
            bind_style(doc, txn, *target, slot, *style)?;
        }
        EditOp::CreateFillStyle { name, hex } => {
            let paint = hex_paint(hex)?;
            let sid = create_local_style(
                doc,
                txn,
                alloc,
                "FILL",
                name,
                Obj::from([("fillPaints", Val::Arr(vec![paint]))]),
            )?;
            created_ids.push(sid);
        }
        EditOp::BindPaintVariable {
            target,
            field,
            paint_index,
            variable,
        } => {
            bind_paint_color_variable(doc, txn, *target, field, *paint_index, *variable)?;
        }
        EditOp::SetVariableMode {
            container,
            collection,
            mode,
        } => {
            set_explicit_variable_mode(doc, txn, *container, *collection, *mode)?;
        }
        EditOp::SetVariableColor {
            variable,
            mode,
            hex,
        } => {
            let paint = hex_paint(hex)?;
            let color = paint.get("color").cloned().unwrap();
            set_variable_mode_value(
                doc,
                txn,
                *variable,
                *mode,
                Val::Obj(Obj::from([("colorValue", color)])),
                "COLOR",
                "COLOR",
            )?;
        }
        EditOp::SetVectorNetwork { target, network } => {
            ensure_exists(doc, *target)?;
            let net_blob_idx = doc.blobs.len() + txn.add_blobs.len();
            txn.add_blobs.push(encode_vector_network(network));
            let mut fill_entries = Vec::new();
            for (cmds, even_odd) in network_to_fill_paths(network) {
                let fidx = doc.blobs.len() + txn.add_blobs.len();
                txn.add_blobs.push(encode_path_commands(&cmds));
                fill_entries.push(make_path_entry(fidx, even_odd));
            }
            let mut set = Obj::new();
            set.insert(
                "vectorData",
                Val::Obj(Obj::from([(
                    "vectorNetworkBlob",
                    Val::Int(net_blob_idx as i64),
                )])),
            );
            set.insert("fillGeometry", Val::Arr(fill_entries));
            txn.modify(*target, set, Vec::new());
        }
        EditOp::BooleanCombine {
            operation,
            a,
            b,
            name,
        } => {
            let na = ensure_exists(doc, *a)?;
            let nb = ensure_exists(doc, *b)?;
            let parent = na
                .parent
                .ok_or_else(|| FigError::malformed("operand A has no parent"))?;
            let cmds_a = extract_world_or_local_cmds(doc, na);
            let cmds_b = extract_world_or_local_cmds(doc, nb);
            let combined = boolean_combine(*operation, &cmds_a, &cmds_b);
            let blob_idx = doc.blobs.len() + txn.add_blobs.len();
            txn.add_blobs.push(encode_path_commands(&combined));

            let bool_id = alloc.next_guid();
            let pos = na.position.clone();
            let mut obj = Obj::new();
            obj.insert("guid", bool_id.to_val());
            obj.insert("phase", Val::Enum(intern("CREATED")));
            obj.insert("type", Val::Enum(intern("BOOLEAN_OPERATION")));
            obj.insert(
                "name",
                Val::Str(
                    name.clone()
                        .unwrap_or_else(|| operation.figma_name().into()),
                ),
            );
            obj.insert("visible", Val::Bool(true));
            obj.insert("opacity", Val::Float(1.0));
            obj.insert(
                "booleanOperation",
                Val::Enum(intern(operation.figma_name())),
            );
            obj.insert("transform", matrix_2x3(0.0, 0.0, 0.0));
            obj.insert(
                "size",
                na.data.get("size").cloned().unwrap_or_else(|| {
                    Val::Obj(Obj::from([
                        ("x", Val::Float(100.0)),
                        ("y", Val::Float(100.0)),
                    ]))
                }),
            );
            obj.insert(
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", parent.to_val()),
                    ("position", Val::Str(pos)),
                ])),
            );
            if let Some(fp) = na.data.get("fillPaints") {
                obj.insert("fillPaints", fp.clone());
            }
            obj.insert(
                "fillGeometry",
                Val::Arr(vec![make_path_entry(blob_idx, false)]),
            );
            txn.create_node(bool_id, Val::Obj(obj));
            let child_pos = fractional::n_between(None, None, 2);
            for (idx, &cid) in [*a, *b].iter().enumerate() {
                txn.set_field(
                    cid,
                    "parentIndex",
                    Val::Obj(Obj::from([
                        ("guid", bool_id.to_val()),
                        ("position", Val::Str(child_pos[idx].clone())),
                    ])),
                );
            }
            created_ids.push(bool_id);
            layout_touched.push(parent);
        }
        EditOp::PatchRaw { target, set, unset } => {
            ensure_exists(doc, *target)?;
            let mut obj = Obj::new();
            for (k, jv) in set {
                let v: Val = serde_json::from_value(jv.clone())
                    .map_err(|e| FigError::malformed(format!("invalid Val for {k}: {e}")))?;
                obj.insert(k, v);
            }
            txn.modify(*target, obj, unset.clone());
            layout_touched.push(*target);
        }
    }
    Ok(())
}

fn ensure_exists(doc: &Document, id: Guid) -> Result<&crate::Node, FigError> {
    doc.node(id)
        .ok_or_else(|| FigError::malformed(format!("node {id} does not exist")))
}

fn collect_postorder(doc: &Document, id: Guid, out: &mut Vec<Guid>) {
    for &c in doc.children(id) {
        collect_postorder(doc, c, out);
    }
    out.push(id);
}

fn extract_world_or_local_cmds(doc: &Document, node: &crate::Node) -> Vec<PathCmd> {
    let dx = node
        .data
        .path(&["transform", "m02"])
        .and_then(Val::as_f32)
        .unwrap_or(0.0);
    let dy = node
        .data
        .path(&["transform", "m12"])
        .and_then(Val::as_f32)
        .unwrap_or(0.0);
    let mut cmds = node
        .data
        .get("fillGeometry")
        .and_then(|fg| fg.items().first())
        .and_then(|p| p.get("commandsBlob"))
        .and_then(Val::as_u32)
        .and_then(|idx| doc.blobs.get(idx as usize))
        .and_then(|b| crate::edit::geometry::decode_path_commands(b))
        .or_else(|| {
            node.data
                .path(&["vectorData", "vectorNetworkBlob"])
                .and_then(Val::as_u32)
                .and_then(|idx| doc.blobs.get(idx as usize))
                .and_then(|b| decode_vector_network(b))
                .and_then(|net| {
                    network_to_fill_paths(&net)
                        .into_iter()
                        .next()
                        .map(|(c, _)| c)
                })
        })
        .or_else(|| rebuild_shape_geometry(&node.kind, &node.data))
        .unwrap_or_default();

    if dx != 0.0 || dy != 0.0 {
        for c in &mut cmds {
            match c {
                PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => {
                    *x += dx;
                    *y += dy;
                }
                PathCmd::QuadTo(cx, cy, x, y) => {
                    *cx += dx;
                    *cy += dy;
                    *x += dx;
                    *y += dy;
                }
                PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                    *c1x += dx;
                    *c1y += dy;
                    *c2x += dx;
                    *c2y += dy;
                    *x += dx;
                    *y += dy;
                }
                PathCmd::Close => {}
            }
        }
    }
    cmds
}

fn merge_step_into_batch(batch: &mut Txn, step: Txn) {
    batch.patches.extend(step.patches);
    batch.add_blobs.extend(step.add_blobs);
    for (h, b) in step.add_images {
        if !batch.add_images.iter().any(|(eh, _)| eh == &h) {
            batch.add_images.push((h, b));
        }
    }
}

fn clone_document_shell(doc: &Document) -> Document {
    Document {
        instance: doc.instance,
        image_instance: doc.image_instance,
        revision: doc.revision,
        prelude: doc.prelude.clone(),
        format_version: doc.format_version,
        file_name: doc.file_name.clone(),
        nodes: doc.nodes.clone(),
        index: doc.index.clone(),
        children: doc.children.clone(),
        root: doc.root,
        pages: doc.pages.clone(),
        internal_pages: doc.internal_pages.clone(),
        blobs: doc.blobs.clone(),
        images: Default::default(),
        lazy_images: Default::default(),
        thumbnail: None,
        source: doc.source.clone(),
    }
}
