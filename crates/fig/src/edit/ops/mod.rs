//! Unified semantic [`EditOp`] types and batch execution engine shared by the GUI editor, CLI,
//! and MCP.
//!
//! Every [`EditOp`] compiles against a staged copy of the document, runs the Auto-Layout &
//! geometry derive pass on affected ancestor containers at the end of the batch, validates all
//! resulting patches against the file's Kiwi schema, and commits as a single atomic [`Txn`] in
//! [`History`].

mod batch;
mod compile_arrange;
mod compile_components;
mod compile_design;
mod compile_props;
mod compile_proto;
mod compile_style;
mod compile_tree;

pub use crate::edit::component_props::PropKind as ComponentPropKind;
pub use crate::edit::guid_wire;
pub use crate::edit::specs::{
    EffectPatch, EffectSpec, GradientKind, ImageScale, PaintPatch, PaintSlot, PaintSpec, StopSpec,
    TextLength, TypographyExtra,
};
pub use crate::edit::styles::StyleKind;
pub use crate::edit::variables::{BindField, VarType, VarValue};
pub(crate) use batch::merge_step_into_batch;
pub use batch::{
    describe_edit_ops_markdown, execute_edit_ops, execute_import_image, execute_paste_html,
};
pub use compile_arrange::{AlignEdge, AlignTo, Axis, Pivot, ZOrder};

use serde::{Deserialize, Serialize};

use crate::Guid;
use crate::edit::component_props::PropKind;
use crate::edit::instances::ComponentPropVal;
use crate::edit::text_runs::TextStyleRunPatch;
use crate::edit::vector_net::{BooleanOp, VectorNetwork};
pub use crate::edit::vector_ops::VectorEdit;
pub use compile_proto::{
    Action, ExportSpec, GridSpec, MaskKind, ReactionSpec, StripeSpec, Trigger,
};

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
        /// Keep the width-to-height ratio when resizing (Figma's aspect lock).
        #[serde(default)]
        proportions_locked: Option<bool>,
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
    /// Sets rotation in degrees about `pivot` (default `top_left`: keeps the `(m02, m12)`
    /// translation; `center`: keeps the layer's centre in place).
    Rotate {
        #[serde(with = "guid_wire")]
        target: Guid,
        degrees: f64,
        #[serde(default)]
        pivot: Pivot,
    },
    /// Wraps sibling layers in a new group (a resize-to-fit frame) at the topmost one's place.
    Group {
        #[serde(with = "guid_wire::vec")]
        targets: Vec<Guid>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Wraps sibling layers in a new frame sized to them.
    FrameSelection {
        #[serde(with = "guid_wire::vec")]
        targets: Vec<Guid>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Moves a group's children to its parent (keeping page positions and order) and deletes it.
    Ungroup {
        #[serde(with = "guid_wire")]
        target: Guid,
    },
    /// Aligns layers on an edge or centre line of their union (or of each one's parent).
    Align {
        #[serde(with = "guid_wire::vec")]
        targets: Vec<Guid>,
        edge: AlignEdge,
        #[serde(default)]
        to: AlignTo,
    },
    /// Spaces layers evenly along an axis (`spacing` = a fixed gap instead).
    Distribute {
        #[serde(with = "guid_wire::vec")]
        targets: Vec<Guid>,
        axis: Axis,
        #[serde(default)]
        spacing: Option<f64>,
    },
    /// Mirrors a layer about its own centre line.
    Flip {
        #[serde(with = "guid_wire")]
        target: Guid,
        axis: Axis,
    },
    /// Moves a layer to the front, back, one step forward or one step backward among its siblings.
    Reorder {
        #[serde(with = "guid_wire")]
        target: Guid,
        to: ZOrder,
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
    /// Replaces the effect list: with `effects` when given, else with the shorthand shadow
    /// and/or blur (no fields clears the list).
    SetEffects {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        drop_shadow: Option<ShadowSpec>,
        #[serde(default)]
        blur_radius: Option<f64>,
        #[serde(default)]
        background_blur: bool,
        #[serde(default)]
        effects: Option<Vec<EffectSpec>>,
    },
    /// Inserts an effect at `index` (default: the end).
    AddEffect {
        #[serde(with = "guid_wire")]
        target: Guid,
        effect: EffectSpec,
        #[serde(default)]
        index: Option<usize>,
    },
    /// Changes fields of the effect at `index`, keeping the others.
    UpdateEffect {
        #[serde(with = "guid_wire")]
        target: Guid,
        index: usize,
        patch: EffectPatch,
    },
    /// Removes the effect at `index`.
    RemoveEffect {
        #[serde(with = "guid_wire")]
        target: Guid,
        index: usize,
    },
    /// Moves the effect at `from` to `to`.
    MoveEffect {
        #[serde(with = "guid_wire")]
        target: Guid,
        from: usize,
        to: usize,
    },
    /// Replaces a fill or stroke paint list (solid, gradient and image paints). Detaches a
    /// bound paint style, as editing the list does in Figma.
    SetPaints {
        #[serde(with = "guid_wire")]
        target: Guid,
        slot: PaintSlot,
        paints: Vec<PaintSpec>,
    },
    /// Inserts a paint at `index` (default: the end, which Figma draws on top).
    AddPaint {
        #[serde(with = "guid_wire")]
        target: Guid,
        slot: PaintSlot,
        paint: PaintSpec,
        #[serde(default)]
        index: Option<usize>,
    },
    /// Changes fields of the paint at `index`, keeping the others.
    UpdatePaint {
        #[serde(with = "guid_wire")]
        target: Guid,
        slot: PaintSlot,
        index: usize,
        patch: PaintPatch,
    },
    /// Removes the paint at `index`.
    RemovePaint {
        #[serde(with = "guid_wire")]
        target: Guid,
        slot: PaintSlot,
        index: usize,
    },
    /// Moves the paint at `from` to `to`.
    MovePaint {
        #[serde(with = "guid_wire")]
        target: Guid,
        slot: PaintSlot,
        from: usize,
        to: usize,
    },
    /// Sets stroke weight, position, caps, joins, miter limit, dashes (empty list = solid) and
    /// per-side weights `[top, right, bottom, left]`.
    SetStrokeStyle {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        weight: Option<f64>,
        #[serde(default)]
        align: Option<String>,
        #[serde(default)]
        cap: Option<String>,
        #[serde(default)]
        join: Option<String>,
        #[serde(default)]
        miter_limit: Option<f64>,
        #[serde(default)]
        dashes: Option<Vec<f64>>,
        #[serde(default)]
        sides: Option<[f64; 4]>,
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
        /// Line height, letter and paragraph spacing, vertical align, decoration, case and
        /// truncation for the whole text (each removes that setting from the character runs).
        #[serde(flatten, default)]
        extra: TypographyExtra,
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
        /// Any other overridable fields, in the `patch_raw` value form (checked against the
        /// file's schema; structural fields such as `transform` or `parentIndex` are refused).
        #[serde(default)]
        fields: Option<serde_json::Map<String, serde_json::Value>>,
    },
    /// Adds a `BOOL`, `TEXT` or `INSTANCE_SWAP` property to a component (or its variant set).
    /// Its id is reported in `created_ids`.
    AddComponentProp {
        #[serde(with = "guid_wire")]
        component: Guid,
        name: String,
        kind: PropKind,
        default: ComponentPropVal,
    },
    /// Renames a component property and/or changes its default (`prop`: name or id).
    EditComponentProp {
        #[serde(with = "guid_wire")]
        component: Guid,
        prop: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        default: Option<ComponentPropVal>,
    },
    /// Removes a component property with its layer bindings and instance assignments.
    RemoveComponentProp {
        #[serde(with = "guid_wire")]
        component: Guid,
        prop: String,
    },
    /// Binds a layer inside a component to one of its properties: a `BOOL` drives visibility,
    /// a `TEXT` the characters (TEXT layers), an `INSTANCE_SWAP` the swapped component
    /// (instances).
    BindComponentProp {
        #[serde(with = "guid_wire")]
        node: Guid,
        prop: String,
    },
    /// Removes a layer's binding of the given kind.
    UnbindComponentProp {
        #[serde(with = "guid_wire")]
        node: Guid,
        kind: PropKind,
    },
    /// Combines sibling components into a variant set. Names like `Size=Large, State=Hover`
    /// become properties; a plain name becomes `Property 1=<name>`.
    CombineAsVariants {
        #[serde(with = "guid_wire::vec")]
        components: Vec<Guid>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Adds a copy of variant `from` to its set with a new value for the first property.
    AddVariant {
        #[serde(with = "guid_wire")]
        from: Guid,
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
    /// Wraps two or more sibling layers in a live `BOOLEAN_OPERATION` (`Union`, `Subtract` =
    /// the bottom layer minus the rest, `Intersect`, `Exclude`). Its result is recomputed
    /// whenever an operand changes; paints come from the bottom operand.
    BooleanCombine {
        operation: BooleanOp,
        #[serde(with = "guid_wire::vec")]
        targets: Vec<Guid>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Turns a rectangle, ellipse, polygon, star or line into an editable vector in place.
    ConvertToVector {
        #[serde(with = "guid_wire")]
        target: Guid,
    },
    /// Merges sibling layers into one vector of their combined fill area (bottom layer's paints).
    Flatten {
        #[serde(with = "guid_wire::vec")]
        targets: Vec<Guid>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Turns a layer's stroke into a filled vector just above it; the layer loses its stroke
    /// (and is removed when nothing visible is left).
    OutlineStroke {
        #[serde(with = "guid_wire")]
        target: Guid,
    },
    /// Edits a vector's network vertex by vertex (coordinates in the network's own space).
    EditVectorNetwork {
        #[serde(with = "guid_wire")]
        target: Guid,
        edits: Vec<VectorEdit>,
    },
    /// Creates a local style: `paint` (fills and strokes) from `paints` or `from_node`'s fills,
    /// `effect` from `effects` or `from_node`, `text` and `grid` from `from_node`. Its id is
    /// reported in `created_ids`.
    CreateStyle {
        kind: StyleKind,
        name: String,
        #[serde(default, with = "guid_wire::opt")]
        from_node: Option<Guid>,
        #[serde(default)]
        paints: Option<Vec<PaintSpec>>,
        #[serde(default)]
        effects: Option<Vec<EffectSpec>>,
    },
    /// Renames and/or restyles a style; every layer using it is updated.
    UpdateStyle {
        #[serde(with = "guid_wire")]
        style: Guid,
        #[serde(default)]
        name: Option<String>,
        #[serde(default, with = "guid_wire::opt")]
        from_node: Option<Guid>,
        #[serde(default)]
        paints: Option<Vec<PaintSpec>>,
        #[serde(default)]
        effects: Option<Vec<EffectSpec>>,
    },
    /// Deletes a style; layers using it keep their look, detached.
    DeleteStyle {
        #[serde(with = "guid_wire")]
        style: Guid,
    },
    /// Creates a variable collection with modes (default one, `Mode 1`). `created_ids` lists
    /// the collection, then its modes.
    CreateVariableCollection {
        name: String,
        #[serde(default)]
        modes: Vec<String>,
    },
    /// Adds a mode (values copied from the first mode); its id is in `created_ids`.
    AddVariableMode {
        #[serde(with = "guid_wire")]
        collection: Guid,
        name: String,
    },
    RenameVariableMode {
        #[serde(with = "guid_wire")]
        collection: Guid,
        #[serde(with = "guid_wire")]
        mode: Guid,
        name: String,
    },
    /// Removes a mode (not the last one) and its values.
    RemoveVariableMode {
        #[serde(with = "guid_wire")]
        collection: Guid,
        #[serde(with = "guid_wire")]
        mode: Guid,
    },
    /// Creates a variable with `value` in every mode; its id is in `created_ids`.
    CreateVariable {
        #[serde(with = "guid_wire")]
        collection: Guid,
        name: String,
        kind: VarType,
        value: VarValue,
    },
    /// Sets a variable's value (or alias) for one mode; layers bound to it are updated.
    SetVariableValue {
        #[serde(with = "guid_wire")]
        variable: Guid,
        #[serde(with = "guid_wire")]
        mode: Guid,
        value: VarValue,
    },
    RenameVariable {
        #[serde(with = "guid_wire")]
        variable: Guid,
        name: String,
    },
    /// Deletes a variable; layers, paints and aliases using it keep their current values.
    DeleteVariable {
        #[serde(with = "guid_wire")]
        variable: Guid,
    },
    /// Binds a number, string or bool variable to a layer field (`variable: null` unbinds and
    /// keeps the value). Colour variables bind to paints with `bind_paint_variable`.
    BindVariable {
        #[serde(with = "guid_wire")]
        target: Guid,
        field: BindField,
        #[serde(default, with = "guid_wire::opt")]
        variable: Option<Guid>,
    },
    /// Makes a layer a mask for the layers above it in its parent (`null` stops masking).
    SetMask {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        mask: Option<MaskKind>,
    },
    /// Replaces a frame's layout grids (detaching a grid style).
    SetLayoutGrids {
        #[serde(with = "guid_wire")]
        target: Guid,
        grids: Vec<GridSpec>,
    },
    /// Replaces a layer's export settings.
    SetExportSettings {
        #[serde(with = "guid_wire")]
        target: Guid,
        settings: Vec<ExportSpec>,
    },
    /// Replaces a layer's prototype reactions.
    SetReactions {
        #[serde(with = "guid_wire")]
        target: Guid,
        reactions: Vec<ReactionSpec>,
    },
    /// Starts a prototype flow at a top-level frame (`name: null` removes the start).
    SetFlowStart {
        #[serde(with = "guid_wire")]
        frame: Guid,
        #[serde(default)]
        name: Option<String>,
    },
    /// Sets how a frame scrolls in a prototype (`overflow`: none, horizontal, vertical, both)
    /// and how a child moves when its parent scrolls (`behavior`: scrolls, fixed, sticky).
    SetScrollBehavior {
        #[serde(with = "guid_wire")]
        target: Guid,
        #[serde(default)]
        overflow: Option<String>,
        #[serde(default)]
        behavior: Option<String>,
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
