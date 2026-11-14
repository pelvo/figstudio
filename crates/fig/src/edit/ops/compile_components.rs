//! Component, instance, style, variable, vector network, and Boolean [`EditOp`] compilation.

use crate::edit::geometry::{
    PathCmd, encode_path_commands, make_path_entry, matrix_2x3, rebuild_shape_geometry,
};
use crate::edit::instances::{
    convert_frame_to_component, detach_instance, instantiate_component, reset_instance_override,
    set_component_prop_assignment, set_instance_override, swap_instance_symbol,
    switch_instance_variant,
};
use crate::edit::styles_vars::{
    bind_paint_color_variable, bind_style, create_local_style, hex_paint,
    set_explicit_variable_mode, set_variable_mode_value,
};
use crate::edit::vector_net::{
    boolean_combine, decode_vector_network, encode_vector_network, network_to_fill_paths,
};
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::{Document, FigError, Guid, Obj, Val, intern};

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
        _ => {}
    }
    Ok(())
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
