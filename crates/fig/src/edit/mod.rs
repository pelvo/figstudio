//! Non-destructive, schema-validated editing engine for Figma `.fig` documents.

pub mod autolayout;
pub mod fractional;
pub mod geometry;
pub mod guid;
pub mod guid_wire;
pub mod instances;
pub mod ops;
pub mod schema_check;
pub mod styles_vars;
pub mod text_runs;
pub mod txn;
pub mod vector_net;

pub use guid::{EDIT_SESSION_BASE, GuidAlloc};
pub use ops::{
    EditBatchReport, EditOp, ShadowSpec, describe_edit_ops_markdown, execute_edit_ops,
    execute_import_image, execute_paste_html,
};
pub use txn::{Dirty, History, NodePatch, Txn};
