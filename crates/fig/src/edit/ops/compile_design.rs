//! Style and variable [`EditOp`]s.

use crate::edit::styles::StyleSource;
use crate::edit::{GuidAlloc, Txn, styles, variables};
use crate::{Document, FigError, Guid};

use super::EditOp;

pub(super) fn compile_design_op(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: &EditOp,
    created_ids: &mut Vec<Guid>,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::CreateStyle {
            kind,
            name,
            from_node,
            paints,
            effects,
        } => created_ids.push(styles::create(
            doc,
            txn,
            alloc,
            *kind,
            name,
            StyleSource {
                from_node: *from_node,
                paints: paints.as_deref(),
                effects: effects.as_deref(),
            },
        )?),
        EditOp::UpdateStyle {
            style,
            name,
            from_node,
            paints,
            effects,
        } => layout_touched.extend(styles::update(
            doc,
            txn,
            *style,
            name.as_deref(),
            StyleSource {
                from_node: *from_node,
                paints: paints.as_deref(),
                effects: effects.as_deref(),
            },
        )?),
        EditOp::DeleteStyle { style } => styles::delete(doc, txn, *style)?,
        EditOp::CreateVariableCollection { name, modes } => {
            created_ids.extend(variables::create_collection(doc, txn, alloc, name, modes)?);
        }
        EditOp::AddVariableMode { collection, name } => {
            created_ids.push(variables::add_mode(doc, txn, alloc, *collection, name)?);
        }
        EditOp::RenameVariableMode {
            collection,
            mode,
            name,
        } => variables::rename_mode(doc, txn, *collection, *mode, name)?,
        EditOp::RemoveVariableMode { collection, mode } => {
            variables::remove_mode(doc, txn, *collection, *mode)?;
        }
        EditOp::CreateVariable {
            collection,
            name,
            kind,
            value,
        } => created_ids.push(variables::create_variable(
            doc,
            txn,
            alloc,
            *collection,
            name,
            *kind,
            value,
        )?),
        EditOp::SetVariableValue {
            variable,
            mode,
            value,
        } => variables::set_value(doc, txn, *variable, *mode, value)?,
        EditOp::RenameVariable { variable, name } => {
            variables::rename_variable(doc, txn, *variable, name)?;
        }
        EditOp::DeleteVariable { variable } => variables::delete_variable(doc, txn, *variable)?,
        EditOp::BindVariable {
            target,
            field,
            variable,
        } => {
            variables::bind(doc, txn, *target, *field, *variable)?;
            layout_touched.push(*target);
        }
        _ => return Err(FigError::malformed("not a style or variable op")),
    }
    Ok(())
}
