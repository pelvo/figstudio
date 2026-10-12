//! Owned token records, preserving stored values without changing the document.
use crate::{Guid, Val};

/// One named variable mode.
#[derive(Clone, Debug)]
pub struct Mode {
    /// Local mode identifier.
    pub id: Guid,
    /// Display name.
    pub name: String,
    /// Schema ordering key; equal or missing keys retain their relative stored order.
    pub sort_position: String,
}
/// A local variable collection.
#[derive(Debug)]
pub struct Collection {
    /// Local collection identifier.
    pub id: Guid,
    /// Display name.
    pub name: String,
    /// Modes ordered by their schema sort position.
    pub modes: Vec<Mode>,
    /// Optional explicit override, otherwise the first sorted mode.
    pub default_mode: Option<Guid>,
}
/// One per-mode stored variable value.
#[derive(Debug)]
pub struct ModeValue {
    /// Mode identifier, absent for legacy entries.
    pub mode: Option<Guid>,
    /// Original variable data payload.
    pub data: Val,
}
/// A variable retains its original per-mode payload without modifying the document.
#[derive(Debug)]
pub struct Variable {
    /// Local variable identifier.
    pub id: Guid,
    /// Display name.
    pub name: String,
    /// Owning local collection, when available.
    pub collection: Option<Guid>,
    /// Stored resolved type, including unknown schema values.
    pub resolved_type: Option<String>,
    /// Values in original storage order.
    pub values: Vec<ModeValue>,
    pub(super) first_value: Option<Val>,
}
/// Final variable value, after bounded alias resolution.
#[derive(Debug, PartialEq)]
pub enum Value {
    /// Stored RGBA message.
    Color(Val),
    /// Numeric value.
    Float(f64),
    /// Text value.
    String(String),
    /// Boolean value.
    Bool(bool),
}
/// Known style kinds, with forward-compatible unknown values.
#[derive(Debug, PartialEq)]
pub enum StyleKind {
    /// Fill paints.
    Fill,
    /// Stroke paints.
    Stroke,
    /// Typography.
    Text,
    /// Effects.
    Effect,
    /// Layout grids.
    Grid,
    /// Unknown schema style kind.
    Other(String),
}
/// Stored style payload, addressed by local GUID or published asset reference.
#[derive(Debug)]
pub struct Style {
    /// Local style identifier.
    pub id: Guid,
    /// Display name.
    pub name: String,
    /// Schema style kind.
    pub kind: StyleKind,
    /// Original paint list.
    pub paints: Option<Val>,
    /// Original node data, including typography and style references.
    pub data: Val,
    /// Original effects list.
    pub effects: Option<Val>,
    /// Original layout grids list.
    pub grids: Option<Val>,
}
