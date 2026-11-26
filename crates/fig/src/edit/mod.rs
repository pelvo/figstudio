//! Non-destructive, schema-validated editing engine for Figma `.fig` documents.

pub mod autolayout;
pub mod component_props;
pub mod fractional;
pub mod geometry;
pub mod groups;
pub mod guid;
pub mod guid_wire;
pub mod instances;
pub mod ops;
pub mod outline;
pub mod schema_check;
pub mod space;
pub mod specs;
pub mod styles;
pub mod styles_vars;
pub mod text_runs;
pub mod txn;
pub mod variables;
pub mod vector_net;
pub mod vector_ops;

pub use guid::{EDIT_SESSION_BASE, GuidAlloc};
pub use ops::{
    Action, AlignEdge, AlignTo, Axis, BindField, ComponentPropKind, EditBatchReport, EditOp,
    EffectPatch, EffectSpec, ExportSpec, GradientKind, GridSpec, ImageScale, MaskKind, PaintPatch,
    PaintSlot, PaintSpec, Pivot, ReactionSpec, ShadowSpec, StopSpec, StripeSpec, StyleKind,
    TextLength, Trigger, TypographyExtra, VarType, VarValue, VectorEdit, ZOrder,
    describe_edit_ops_markdown, execute_edit_ops, execute_import_image, execute_paste_html,
};
pub use txn::{Dirty, History, NodePatch, Txn};
