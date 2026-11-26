//! Component, instance, style, variable, vector network, and Boolean [`EditOp`] compilation.

use crate::edit::component_props;
use crate::edit::geometry::{encode_path_commands, make_path_entry};
use crate::edit::instances::{
    convert_frame_to_component, detach_instance, existing_override_field, instantiate_component,
    reset_instance_override, set_component_prop_assignment, set_instance_override,
    swap_instance_symbol, switch_instance_variant,
};
use crate::edit::styles_vars::{
    bind_paint_color_variable, bind_style, create_local_style, hex_paint,
    set_explicit_variable_mode, set_variable_mode_value,
};
use crate::edit::text_runs::replace_characters;
use crate::edit::vector_net::{encode_vector_network, network_to_fill_paths};
use crate::edit::vector_ops;
use crate::edit::{GuidAlloc, Txn};
use crate::{Document, FigError, Guid, Obj, Val};

use super::EditOp;
use super::compile_tree::ensure_exists;

pub(super) fn compile_component_or_vector_op(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: &EditOp,
    created_ids: &mut Vec<Guid>,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::SetInstanceOverride {
            instance,
            override_path,
            characters,
            visible,
            fill_hex,
            opacity,
            fields: extra,
        } => {
            let mut fields = Obj::new();
            let mut unset = Vec::new();
            for (k, jv) in extra.iter().flatten() {
                // Fields that place or identify a layer are not overridable in Figma.
                if matches!(
                    k.as_str(),
                    "guid"
                        | "guidPath"
                        | "parentIndex"
                        | "type"
                        | "phase"
                        | "transform"
                        | "overrideKey"
                        | "symbolData"
                        | "componentPropDefs"
                        | "key"
                ) {
                    return Err(FigError::malformed(format!("{k} cannot be overridden")));
                }
                let v: Val = serde_json::from_value(jv.clone())
                    .map_err(|e| FigError::malformed(format!("invalid Val for {k}: {e}")))?;
                crate::edit::schema_check::validate_node_field(
                    &*doc.decoded_schema()?,
                    k,
                    &v,
                    "override",
                )?;
                fields.insert(k, v);
            }
            if let Some(chars) = characters {
                // Start from the override's current textData so its other fields survive.
                let mut td =
                    match existing_override_field(doc, *instance, override_path, "textData") {
                        Some(Val::Obj(o)) => o,
                        _ => Obj::new(),
                    };
                replace_characters(&mut td, chars);
                fields.insert("textData", Val::Obj(td));
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
            let color = paint
                .get("color")
                .cloned()
                .ok_or_else(|| FigError::malformed("hex paint missing color"))?;
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
            targets,
            name,
        } => {
            let id =
                vector_ops::make_boolean(doc, txn, alloc, *operation, targets, name.as_deref())?;
            created_ids.push(id);
            layout_touched.push(id);
        }
        EditOp::ConvertToVector { target } => {
            vector_ops::convert_to_vector(doc, txn, *target)?;
            layout_touched.push(*target);
        }
        EditOp::Flatten { targets, name } => {
            let id = vector_ops::flatten(doc, txn, alloc, targets, name.as_deref())?;
            created_ids.push(id);
            layout_touched.push(id);
        }
        EditOp::OutlineStroke { target } => {
            let id = vector_ops::outline_stroke(doc, txn, alloc, *target)?;
            created_ids.push(id);
            layout_touched.push(id);
        }
        EditOp::EditVectorNetwork { target, edits } => {
            vector_ops::edit_network(doc, txn, *target, edits)?;
            layout_touched.push(*target);
        }
        EditOp::AddComponentProp {
            component,
            name,
            kind,
            default,
        } => {
            let id = component_props::add_prop(doc, txn, alloc, *component, name, *kind, default)?;
            created_ids.push(id);
            layout_touched.push(*component);
        }
        EditOp::EditComponentProp {
            component,
            prop,
            name,
            default,
        } => {
            component_props::edit_prop(
                doc,
                txn,
                *component,
                prop,
                name.as_deref(),
                default.as_ref(),
            )?;
            layout_touched.push(*component);
        }
        EditOp::RemoveComponentProp { component, prop } => {
            component_props::remove_prop(doc, txn, *component, prop)?;
            layout_touched.push(*component);
        }
        EditOp::BindComponentProp { node, prop } => {
            component_props::bind_prop(doc, txn, *node, Some(prop), None)?;
            layout_touched.push(*node);
        }
        EditOp::UnbindComponentProp { node, kind } => {
            component_props::bind_prop(doc, txn, *node, None, Some(*kind))?;
            layout_touched.push(*node);
        }
        EditOp::CombineAsVariants { components, name } => {
            let set = super::compile_arrange::wrap(
                doc,
                txn,
                alloc,
                components,
                Some(name.as_deref().unwrap_or("Component set")),
                false,
            )?;
            component_props::make_variant_set(doc, txn, alloc, set, components)?;
            created_ids.push(set);
            layout_touched.push(set);
        }
        EditOp::AddVariant { from } => {
            let node = ensure_exists(doc, *from)?;
            let set = node
                .parent
                .filter(|p| doc.node(*p).is_some_and(component_props::is_variant_set))
                .ok_or_else(|| FigError::malformed(format!("{from} is not a variant")))?;
            let (w, _) = crate::edit::space::size(&node.data);
            let kiwi = crate::clipboard::encode_subset_kiwi(doc, &[*from], 0, "variant")?;
            let (paste, roots) =
                crate::clipboard::paste_kiwi_into_with(doc, &kiwi, set, (w + 20.0, 0.0), alloc)?;
            super::merge_step_into_batch(txn, paste);
            let new_id = *roots
                .first()
                .ok_or_else(|| FigError::malformed("the variant copy was empty"))?;
            // A component's library key identifies it: the copy is a new component.
            txn.modify(new_id, Obj::new(), vec!["key".into()]);
            component_props::finish_added_variant(doc, txn, alloc, set, *from, new_id)?;
            grow_to_fit(doc, txn, set, *from, w + 20.0)?;
            created_ids.push(new_id);
            layout_touched.push(set);
        }
        EditOp::CreateStyle { .. }
        | EditOp::UpdateStyle { .. }
        | EditOp::DeleteStyle { .. }
        | EditOp::CreateVariableCollection { .. }
        | EditOp::AddVariableMode { .. }
        | EditOp::RenameVariableMode { .. }
        | EditOp::RemoveVariableMode { .. }
        | EditOp::CreateVariable { .. }
        | EditOp::SetVariableValue { .. }
        | EditOp::RenameVariable { .. }
        | EditOp::DeleteVariable { .. }
        | EditOp::BindVariable { .. } => super::compile_design::compile_design_op(
            doc,
            txn,
            alloc,
            op,
            created_ids,
            layout_touched,
        )?,
        EditOp::SetMask { .. }
        | EditOp::SetLayoutGrids { .. }
        | EditOp::SetExportSettings { .. }
        | EditOp::SetReactions { .. }
        | EditOp::SetFlowStart { .. }
        | EditOp::SetScrollBehavior { .. } => {
            super::compile_proto::compile_proto_op(doc, txn, alloc, op, layout_touched)?;
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
        other => {
            return Err(FigError::malformed(format!(
                "edit op not handled: {other:?}"
            )));
        }
    }
    Ok(())
}

/// Grows the (fixed-size) variant `set` so a copy of `from` moved by `dx` stays inside it.
fn grow_to_fit(
    doc: &Document,
    txn: &mut Txn,
    set: Guid,
    from: Guid,
    dx: f64,
) -> Result<(), FigError> {
    let set_node = ensure_exists(doc, set)?;
    let b = crate::edit::space::bounds_in_parent(&ensure_exists(doc, from)?.data)
        .ok_or_else(|| FigError::malformed(format!("{from} has no bounds")))?;
    let (w, h) = crate::edit::space::size(&set_node.data);
    let need_w = b.x1 + dx + 20.0;
    if need_w > w {
        txn.set_field(
            set,
            "size",
            Val::Obj(Obj::from([("x", Val::Float(need_w)), ("y", Val::Float(h))])),
        );
    }
    Ok(())
}
