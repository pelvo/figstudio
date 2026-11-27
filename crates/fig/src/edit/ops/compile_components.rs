//! Component, instance, style, variable, vector network, and Boolean [`EditOp`] compilation.

use std::collections::HashSet;

use crate::edit::geometry::{
    PathCmd, corner_radii_of, cubic_extrema, decode_path_commands, encode_path_commands,
    has_visible_paints, make_path_entry, make_styled_path_entry, matrix_2x3, quad_extrema,
    rebuild_shape_geometry,
};
use crate::edit::instances::{
    convert_frame_to_component, detach_instance, existing_override_field, instantiate_component,
    reset_instance_override, set_component_prop_assignment, set_instance_override,
    swap_instance_symbol, switch_instance_variant,
};
use crate::edit::schema_check::{coerce_json_node_field, require_finite_f32};
use crate::edit::styles_vars::{
    bind_paint_color_variable, bind_style, create_local_style, hex_color, hex_paint,
    set_explicit_variable_mode, set_variable_mode_value,
};
use crate::edit::text_runs::replace_characters;
use crate::edit::vector_net::{
    BooleanOp, BooleanPath, boolean_combine_paths, decode_vector_network, network_bounds,
    network_to_styled_fill_paths, try_encode_vector_network,
};
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::resolve::{InstancePath, resolve_path};
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
                // Start from the override's current textData so its other fields survive;
                // a first override starts from the text as the instance shows it now, so
                // the master's style runs are kept.
                let mut td =
                    match existing_override_field(doc, *instance, override_path, "textData") {
                        Some(Val::Obj(o)) => o,
                        _ => shown_text_data(doc, *instance, override_path),
                    };
                replace_characters(&mut td, chars)?;
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
                let op = require_finite_f32(*op, "set_instance_override opacity")?;
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
            require_finite_f32(*x, "instantiate_component x")?;
            require_finite_f32(*y, "instantiate_component y")?;
            let inst_id = instantiate_component(doc, txn, alloc, *symbol, *parent, *x, *y)?;
            created_ids.push(inst_id);
            layout_touched.push(*parent);
        }
        EditOp::BindStyle {
            target,
            slot,
            style,
        } => {
            let node = ensure_exists(doc, *target)?;
            let borders_take_space = node.data.flag("bordersTakeSpace") == Some(true);
            let is_text = node.kind == "TEXT";
            bind_style(doc, txn, *target, slot, *style)?;
            if (slot == "stroke" && borders_take_space) || (slot == "text" && is_text) {
                layout_touched.push(*target);
            }
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
            let color = hex_color(hex)?;
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
            let node = ensure_exists(doc, *target)?;
            if node.kind != "VECTOR" {
                return Err(FigError::malformed(format!(
                    "node {target} is {}, expected VECTOR",
                    node.kind
                )));
            }
            let encoded_net = try_encode_vector_network(network)?;
            let net_blob_idx = doc.blobs.len() + txn.add_blobs.len();
            txn.add_blobs.push(encoded_net);
            let mut fill_entries = Vec::new();
            for fp in network_to_styled_fill_paths(network) {
                let fidx = doc.blobs.len() + txn.add_blobs.len();
                txn.add_blobs.push(encode_path_commands(&fp.cmds));
                fill_entries.push(make_styled_path_entry(fidx, fp.even_odd, fp.style_id));
            }
            let (new_w, new_h) = network_bounds(network)
                .map_or((0.0, 0.0), |(_, _, max_x, max_y)| {
                    (max_x.max(0.0), max_y.max(0.0))
                });
            let mut vector_data = match node.data.get("vectorData").cloned().map(Val::into_owned) {
                Some(Val::Obj(o)) => o,
                _ => Obj::new(),
            };
            vector_data.insert("vectorNetworkBlob", Val::Int(net_blob_idx as i64));
            if new_w > 0.0 && new_h > 0.0 {
                vector_data.insert(
                    "normalizedSize",
                    Val::Obj(Obj::from([
                        ("x", Val::Float(new_w)),
                        ("y", Val::Float(new_h)),
                    ])),
                );
            } else {
                vector_data.remove("normalizedSize");
            }
            let mut set = Obj::new();
            set.insert("vectorData", Val::Obj(vector_data));
            set.insert("fillGeometry", Val::Arr(fill_entries));
            set.insert(
                "size",
                Val::Obj(Obj::from([
                    ("x", Val::Float(new_w)),
                    ("y", Val::Float(new_h)),
                ])),
            );
            txn.modify(*target, set, Vec::new());
            layout_touched.push(*target);
        }
        EditOp::BooleanCombine {
            operation,
            a,
            b,
            name,
        } => {
            compile_boolean_combine(
                doc,
                txn,
                alloc,
                *operation,
                *a,
                *b,
                name.as_deref(),
                created_ids,
                layout_touched,
            )?;
        }
        EditOp::PatchRaw { target, set, unset } => {
            ensure_exists(doc, *target)?;
            let schema = doc.decoded_schema()?;
            let at = format!("node {target}");
            let mut obj = Obj::new();
            for (k, jv) in set {
                let (field_name, coerced) = coerce_json_node_field(&schema, k, jv, &at)?;
                obj.insert(field_name, coerced);
            }
            txn.modify(*target, obj, unset.clone());
            layout_touched.push(*target);
        }
        _ => {}
    }
    Ok(())
}

/// The `textData` the instance shows at `override_path` (master text with overrides applied).
fn shown_text_data(doc: &Document, instance: Guid, override_path: &[Guid]) -> Obj {
    let guids = std::iter::once(instance)
        .chain(override_path.iter().copied())
        .collect();
    InstancePath::new(guids)
        .and_then(|path| resolve_path(doc, &path).ok())
        .and_then(|shown| shown.data.get("textData").cloned().map(Val::into_owned))
        .and_then(|td| match td {
            Val::Obj(o) => Some(o),
            _ => None,
        })
        .unwrap_or_default()
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal op helper mirrors compile_* signature"
)]
fn compile_boolean_combine(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    operation: BooleanOp,
    a: Guid,
    b: Guid,
    name: Option<&str>,
    created_ids: &mut Vec<Guid>,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    if a == b {
        return Err(FigError::malformed("boolean operands must be distinct"));
    }
    let na = ensure_exists(doc, a)?;
    let nb = ensure_exists(doc, b)?;
    if matches!(na.kind.as_str(), "CANVAS" | "DOCUMENT")
        || matches!(nb.kind.as_str(), "CANVAS" | "DOCUMENT")
    {
        return Err(FigError::malformed(
            "cannot boolean-combine CANVAS or DOCUMENT nodes",
        ));
    }
    let parent = na
        .parent
        .ok_or_else(|| FigError::malformed("operand A has no parent"))?;
    let old_b_parent = nb
        .parent
        .ok_or_else(|| FigError::malformed("operand B has no parent"))?;
    if let (Some(pa), Some(pb)) = (doc.page_of(a), doc.page_of(b))
        && pa != pb
    {
        return Err(FigError::malformed(
            "boolean operands must belong to the same page",
        ));
    }
    if is_ancestor_or_same(doc, b, a) || is_ancestor_or_same(doc, a, b) {
        return Err(FigError::malformed(
            "cannot boolean-combine a node with its own ancestor",
        ));
    }

    let tf_a = Affine2D::from_val(na.data.get("transform"))?;
    let tf_b = if old_b_parent == parent {
        Affine2D::from_val(nb.data.get("transform"))?
    } else {
        let p_world = node_page_transform(doc, parent)?;
        let p_inv = p_world
            .inverse()
            .ok_or_else(|| FigError::malformed("operand A parent transform is not invertible"))?;
        let b_world = node_page_transform(doc, b)?;
        let rel = p_inv.mul(b_world);
        if !rel.is_finite() {
            return Err(FigError::malformed(
                "operand B transform relative to parent is non-finite",
            ));
        }
        rel
    };

    let CombinedOperands {
        cmds: combined,
        fallback_origin,
    } = combine_visible_operands(doc, operation, &[(na, tf_a), (nb, tf_b)])?;

    let (origin, width, height, fill_geometry) =
        if let Some((min_x, min_y, max_x, max_y)) = cmds_bounds(&combined) {
            let local_combined = transform_cmds(&combined, Affine2D::translation(-min_x, -min_y));
            (
                (min_x, min_y),
                (max_x - min_x).max(0.0),
                (max_y - min_y).max(0.0),
                encode_boolean_fill_geometry(doc, txn, &local_combined),
            )
        } else {
            (
                fallback_origin.unwrap_or((0.0, 0.0)),
                0.0,
                0.0,
                Val::Arr(Vec::new()),
            )
        };

    let bool_id = alloc.next_guid();
    let mut obj = Obj::new();
    obj.insert("guid", bool_id.to_val());
    obj.insert("phase", Val::Enum(intern("CREATED")));
    obj.insert("type", Val::Enum(intern("BOOLEAN_OPERATION")));
    obj.insert(
        "name",
        Val::Str(name.unwrap_or(operation.figma_name()).into()),
    );
    obj.insert("visible", Val::Bool(true));
    obj.insert("opacity", Val::Float(1.0));
    obj.insert(
        "booleanOperation",
        Val::Enum(intern(operation.figma_name())),
    );
    obj.insert("transform", matrix_2x3(origin.0, origin.1, 0.0));
    obj.insert(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(width)),
            ("y", Val::Float(height)),
        ])),
    );
    obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", parent.to_val()),
            ("position", Val::Str(na.position.clone())),
        ])),
    );
    if let Some(fp) = na.data.get("fillPaints") {
        obj.insert("fillPaints", fp.clone());
    }
    obj.insert("fillGeometry", fill_geometry);
    txn.create_node(bool_id, Val::Obj(obj));

    let rebase = Affine2D::translation(-origin.0, -origin.1);
    let child_pos = fractional::n_between(None, None, 2);
    for (idx, (cid, tf_in_parent)) in [(a, tf_a), (b, tf_b)].into_iter().enumerate() {
        let mut child_set = Obj::with_capacity(2);
        child_set.insert(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", bool_id.to_val()),
                ("position", Val::Str(child_pos[idx].clone())),
            ])),
        );
        child_set.insert("transform", rebase.mul(tf_in_parent).to_val());
        txn.modify(cid, child_set, Vec::new());
    }
    created_ids.push(bool_id);
    layout_touched.push(parent);
    if old_b_parent != parent {
        layout_touched.push(old_b_parent);
    }
    Ok(())
}

fn encode_boolean_fill_geometry(doc: &Document, txn: &mut Txn, cmds: &[PathCmd]) -> Val {
    let blob_idx = doc.blobs.len() + txn.add_blobs.len();
    txn.add_blobs.push(encode_path_commands(cmds));
    Val::Arr(vec![make_path_entry(blob_idx, false)])
}

/// Recomputes a `BOOLEAN_OPERATION` node's derived `fillGeometry`, `size`, and (when its local
/// origin shifts) affine-safe `transform` + direct-child transform rebasing from its current
/// visible children in sibling order. Fails closed if the operation, any visible operand, or
/// required derived stroke/corner geometry cannot be safely recomputed.
pub(super) fn recompute_boolean_node(
    doc: &Document,
    bool_id: Guid,
    txn: &mut Txn,
) -> Result<(), FigError> {
    let node = ensure_exists(doc, bool_id)?;
    if node.kind != "BOOLEAN_OPERATION" {
        return Ok(());
    }
    let op_name = node.data.text("booleanOperation").ok_or_else(|| {
        FigError::malformed(format!(
            "BOOLEAN_OPERATION {bool_id} is missing booleanOperation"
        ))
    })?;
    let operation = BooleanOp::from_figma_name(op_name).ok_or_else(|| {
        FigError::malformed(format!(
            "BOOLEAN_OPERATION {bool_id} has unsupported booleanOperation {op_name:?}"
        ))
    })?;
    if corner_radii_of(&node.data).iter().any(|&r| r > 1e-4) {
        return Err(FigError::malformed(format!(
            "cannot recompute BOOLEAN_OPERATION {bool_id} with non-zero corner radius"
        )));
    }
    if has_visible_paints(node.data.get("strokePaints")) {
        return Err(FigError::malformed(format!(
            "cannot recompute stroked BOOLEAN_OPERATION {bool_id}: derived strokeGeometry regeneration is not supported"
        )));
    }

    let tf_bool = Affine2D::from_val(node.data.get("transform"))?;
    let children = doc.children(bool_id);
    let mut operands = Vec::with_capacity(children.len());
    for child in children.iter().filter_map(|&cid| doc.node(cid)) {
        let tf_child = Affine2D::from_val(child.data.get("transform"))?;
        operands.push((child, tf_child));
    }
    let combined = combine_visible_operands(doc, operation, &operands)?.cmds;

    let mut set = Obj::new();
    let mut unset = Vec::new();
    if let Some((min_x, min_y, max_x, max_y)) = cmds_bounds(&combined) {
        let local_combined = if min_x.abs() > 1e-6 || min_y.abs() > 1e-6 {
            let rebase = Affine2D::translation(-min_x, -min_y);
            let new_tf_bool = tf_bool.mul(Affine2D::translation(min_x, min_y));
            if !new_tf_bool.is_finite() {
                return Err(FigError::malformed(format!(
                    "rebased transform for BOOLEAN_OPERATION {bool_id} is non-finite"
                )));
            }
            set.insert("transform", new_tf_bool.to_val());
            for child in children.iter().filter_map(|&cid| doc.node(cid)) {
                let new_tf_c = rebase.mul(Affine2D::from_val(child.data.get("transform"))?);
                if !new_tf_c.is_finite() {
                    return Err(FigError::malformed(format!(
                        "rebased transform for boolean child {} is non-finite",
                        child.id
                    )));
                }
                txn.set_field(child.id, "transform", new_tf_c.to_val());
            }
            transform_cmds(&combined, rebase)
        } else {
            combined
        };
        set.insert(
            "fillGeometry",
            encode_boolean_fill_geometry(doc, txn, &local_combined),
        );
        set.insert(
            "size",
            size_val((max_x - min_x).max(0.0), (max_y - min_y).max(0.0)),
        );
    } else {
        set.insert("fillGeometry", Val::Arr(Vec::new()));
        set.insert("size", size_val(0.0, 0.0));
    }
    if node.data.get("strokeGeometry").is_some() {
        unset.push("strokeGeometry".into());
    }
    txn.modify(bool_id, set, unset);
    Ok(())
}

fn size_val(w: f64, h: f64) -> Val {
    Val::Obj(Obj::from([("x", Val::Float(w)), ("y", Val::Float(h))]))
}

#[derive(Clone, Copy, Debug)]
struct Affine2D {
    m00: f64,
    m01: f64,
    m02: f64,
    m10: f64,
    m11: f64,
    m12: f64,
}

impl Affine2D {
    const IDENTITY: Self = Self {
        m00: 1.0,
        m01: 0.0,
        m02: 0.0,
        m10: 0.0,
        m11: 1.0,
        m12: 0.0,
    };

    fn translation(dx: f64, dy: f64) -> Self {
        Self {
            m02: dx,
            m12: dy,
            ..Self::IDENTITY
        }
    }

    fn from_val(value: Option<&Val>) -> Result<Self, FigError> {
        let Some(t) = value else {
            return Ok(Self::IDENTITY);
        };
        let get = |k: &str, default: f64| t.num(k).map_or(default, f64::from);
        let m = Self {
            m00: get("m00", 1.0),
            m01: get("m01", 0.0),
            m02: get("m02", 0.0),
            m10: get("m10", 0.0),
            m11: get("m11", 1.0),
            m12: get("m12", 0.0),
        };
        if !m.is_finite() {
            return Err(FigError::malformed("node has non-finite transform"));
        }
        Ok(m)
    }

    fn is_finite(self) -> bool {
        self.m00.is_finite()
            && self.m01.is_finite()
            && self.m02.is_finite()
            && self.m10.is_finite()
            && self.m11.is_finite()
            && self.m12.is_finite()
    }

    fn mul(self, rhs: Self) -> Self {
        Self {
            m00: self.m00 * rhs.m00 + self.m01 * rhs.m10,
            m01: self.m00 * rhs.m01 + self.m01 * rhs.m11,
            m02: self.m00 * rhs.m02 + self.m01 * rhs.m12 + self.m02,
            m10: self.m10 * rhs.m00 + self.m11 * rhs.m10,
            m11: self.m10 * rhs.m01 + self.m11 * rhs.m11,
            m12: self.m10 * rhs.m02 + self.m11 * rhs.m12 + self.m12,
        }
    }

    fn inverse(self) -> Option<Self> {
        let det = self.m00 * self.m11 - self.m01 * self.m10;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let inv00 = self.m11 / det;
        let inv01 = -self.m01 / det;
        let inv10 = -self.m10 / det;
        let inv11 = self.m00 / det;
        let inv = Self {
            m00: inv00,
            m01: inv01,
            m02: -(inv00 * self.m02 + inv01 * self.m12),
            m10: inv10,
            m11: inv11,
            m12: -(inv10 * self.m02 + inv11 * self.m12),
        };
        inv.is_finite().then_some(inv)
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = (f64::from(x), f64::from(y));
        (
            (self.m00 * x + self.m01 * y + self.m02) as f32,
            (self.m10 * x + self.m11 * y + self.m12) as f32,
        )
    }

    fn to_val(self) -> Val {
        Val::Obj(Obj::from([
            ("m00", Val::Float(self.m00)),
            ("m01", Val::Float(self.m01)),
            ("m02", Val::Float(self.m02)),
            ("m10", Val::Float(self.m10)),
            ("m11", Val::Float(self.m11)),
            ("m12", Val::Float(self.m12)),
        ]))
    }
}

fn is_ancestor_or_same(doc: &Document, candidate: Guid, start: Guid) -> bool {
    let mut cur = Some(start);
    let mut seen = HashSet::new();
    while let Some(id) = cur {
        if id == candidate {
            return true;
        }
        if !seen.insert(id) {
            break;
        }
        cur = doc.node(id).and_then(|n| n.parent);
    }
    false
}

fn node_page_transform(doc: &Document, id: Guid) -> Result<Affine2D, FigError> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = Some(id);
    while let Some(cid) = cur {
        if !seen.insert(cid) {
            break;
        }
        let Some(node) = doc.node(cid) else {
            break;
        };
        if node.kind == "CANVAS" || node.kind == "DOCUMENT" {
            break;
        }
        chain.push(Affine2D::from_val(node.data.get("transform"))?);
        cur = node.parent;
    }
    Ok(chain
        .into_iter()
        .rev()
        .fold(Affine2D::IDENTITY, Affine2D::mul))
}

struct CombinedOperands {
    cmds: Vec<PathCmd>,
    fallback_origin: Option<(f64, f64)>,
}

/// Shared visible-operand fold and strict operand admissibility used by both
/// `EditOp::BooleanCombine` and `recompute_boolean_node`.
fn combine_visible_operands(
    doc: &Document,
    operation: BooleanOp,
    operands: &[(&crate::Node, Affine2D)],
) -> Result<CombinedOperands, FigError> {
    let mut combined_paths: Option<Vec<BooleanPath>> = None;
    let mut first_origin: Option<(f64, f64)> = None;
    for &(node, tf) in operands {
        if node.data.flag("visible") == Some(false) {
            continue;
        }
        let transformed = transform_boolean_paths(&extract_operand_paths(doc, node, 0)?, tf);
        if first_origin.is_none() {
            first_origin = boolean_paths_bounds(&transformed).map(|(x, y, _, _)| (x, y));
        }
        combined_paths = Some(match combined_paths {
            None => transformed,
            Some(acc) => {
                let next = boolean_combine_paths(operation, &acc, &transformed);
                if next.is_empty() {
                    Vec::new()
                } else {
                    vec![BooleanPath {
                        cmds: next,
                        even_odd: false,
                    }]
                }
            }
        });
    }
    let cmds = match combined_paths {
        None => Vec::new(),
        Some(paths) if paths.is_empty() => Vec::new(),
        Some(mut paths) if paths.len() == 1 && !paths[0].even_odd => paths.swap_remove(0).cmds,
        Some(paths) => boolean_combine_paths(BooleanOp::Union, &paths, &[]),
    };
    Ok(CombinedOperands {
        cmds,
        fallback_origin: first_origin,
    })
}

fn extract_operand_paths(
    doc: &Document,
    node: &crate::Node,
    depth: usize,
) -> Result<Vec<BooleanPath>, FigError> {
    if depth > 64 {
        return Err(FigError::malformed(format!(
            "boolean operand {} exceeds maximum GROUP nesting depth",
            node.id
        )));
    }
    if node.kind == "GROUP" {
        let mut all = Vec::new();
        for child in doc
            .children(node.id)
            .iter()
            .filter_map(|&cid| doc.node(cid))
            .filter(|c| c.data.flag("visible") != Some(false))
        {
            let tf_c = Affine2D::from_val(child.data.get("transform"))?;
            all.extend(transform_boolean_paths(
                &extract_operand_paths(doc, child, depth + 1)?,
                tf_c,
            ));
        }
        return Ok(all);
    }
    if matches!(node.kind.as_str(), "FRAME" | "SECTION" | "SYMBOL")
        && !doc.children(node.id).is_empty()
    {
        return Err(FigError::malformed(format!(
            "boolean operand {} ({}) has children and cannot be safely recomputed",
            node.id, node.kind
        )));
    }
    let mut fg_err = None;
    if let Some(fg) = node.data.get("fillGeometry") {
        let Val::Arr(items) = fg else {
            fg_err = Some(FigError::malformed(format!(
                "boolean operand {} has malformed fillGeometry",
                node.id
            )));
            return finish_operand_fallback(doc, node, fg_err);
        };
        let mut all = Vec::with_capacity(items.len());
        for p in items {
            let decoded = p
                .get("commandsBlob")
                .and_then(Val::as_u32)
                .and_then(|idx| doc.blobs.get(idx as usize))
                .and_then(|blob| decode_path_commands(blob));
            let Some(cmds) = decoded else {
                fg_err = Some(FigError::malformed(format!(
                    "boolean operand {} has missing or undecodable fillGeometry commandsBlob",
                    node.id
                )));
                all.clear();
                break;
            };
            if !cmds.is_empty() {
                all.push(BooleanPath {
                    cmds,
                    even_odd: p.text("windingRule") == Some("ODD"),
                });
            }
        }
        if fg_err.is_none() {
            if !all.is_empty() {
                return Ok(all);
            }
            if node.kind == "BOOLEAN_OPERATION"
                || (node.kind == "VECTOR"
                    && node
                        .data
                        .path(&["vectorData", "vectorNetworkBlob"])
                        .is_none())
            {
                return Ok(Vec::new());
            }
        }
    }
    finish_operand_fallback(doc, node, fg_err)
}

fn finish_operand_fallback(
    doc: &Document,
    node: &crate::Node,
    fg_err: Option<FigError>,
) -> Result<Vec<BooleanPath>, FigError> {
    if let Some(blob_val) = node.data.path(&["vectorData", "vectorNetworkBlob"]) {
        if let Some(net) = blob_val
            .as_u32()
            .and_then(|idx| doc.blobs.get(idx as usize))
            .and_then(|blob| decode_vector_network(blob))
        {
            let mut all: Vec<BooleanPath> = network_to_styled_fill_paths(&net)
                .into_iter()
                .map(|fp| BooleanPath {
                    cmds: fp.cmds,
                    even_odd: fp.even_odd,
                })
                .collect();
            if !all.is_empty() {
                if let Some(norm) = node.data.path(&["vectorData", "normalizedSize"])
                    && let (Some(nw), Some(nh), Some(w), Some(h)) = (
                        norm.num("x").map(f64::from),
                        norm.num("y").map(f64::from),
                        node.data
                            .path(&["size", "x"])
                            .and_then(Val::as_f32)
                            .map(f64::from),
                        node.data
                            .path(&["size", "y"])
                            .and_then(Val::as_f32)
                            .map(f64::from),
                    )
                    && nw > 0.0
                    && nh > 0.0
                    && w.is_finite()
                    && h.is_finite()
                {
                    let scale = Affine2D {
                        m00: w / nw,
                        m11: h / nh,
                        ..Affine2D::IDENTITY
                    };
                    for p in &mut all {
                        p.cmds = transform_cmds(&p.cmds, scale);
                    }
                }
                return Ok(all);
            }
            if node.kind == "VECTOR" {
                return Ok(Vec::new());
            }
        } else {
            return Err(FigError::malformed(format!(
                "boolean operand {} has missing or undecodable vectorNetworkBlob",
                node.id
            )));
        }
    }
    if let Some(err) = fg_err {
        return Err(err);
    }
    if let Some(cmds) = rebuild_shape_geometry(&node.kind, &node.data) {
        return Ok(if cmds.is_empty() {
            Vec::new()
        } else {
            vec![BooleanPath {
                cmds,
                even_odd: false,
            }]
        });
    }
    Err(FigError::malformed(format!(
        "boolean operand {} ({}) has no safely recomputable fill geometry",
        node.id, node.kind
    )))
}

fn transform_boolean_paths(paths: &[BooleanPath], tf: Affine2D) -> Vec<BooleanPath> {
    paths
        .iter()
        .map(|p| BooleanPath {
            cmds: transform_cmds(&p.cmds, tf),
            even_odd: p.even_odd,
        })
        .collect()
}

fn boolean_paths_bounds(paths: &[BooleanPath]) -> Option<(f64, f64, f64, f64)> {
    let mut acc: Option<(f64, f64, f64, f64)> = None;
    for p in paths {
        if let Some((x0, y0, x1, y1)) = cmds_bounds(&p.cmds) {
            acc = Some(match acc {
                None => (x0, y0, x1, y1),
                Some((ax0, ay0, ax1, ay1)) => (ax0.min(x0), ay0.min(y0), ax1.max(x1), ay1.max(y1)),
            });
        }
    }
    acc
}

fn transform_cmds(cmds: &[PathCmd], tf: Affine2D) -> Vec<PathCmd> {
    cmds.iter()
        .map(|&c| match c {
            PathCmd::MoveTo(x, y) => {
                let (nx, ny) = tf.apply(x, y);
                PathCmd::MoveTo(nx, ny)
            }
            PathCmd::LineTo(x, y) => {
                let (nx, ny) = tf.apply(x, y);
                PathCmd::LineTo(nx, ny)
            }
            PathCmd::QuadTo(cx, cy, x, y) => {
                let (ncx, ncy) = tf.apply(cx, cy);
                let (nx, ny) = tf.apply(x, y);
                PathCmd::QuadTo(ncx, ncy, nx, ny)
            }
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                let (nc1x, nc1y) = tf.apply(c1x, c1y);
                let (nc2x, nc2y) = tf.apply(c2x, c2y);
                let (nx, ny) = tf.apply(x, y);
                PathCmd::CubicTo(nc1x, nc1y, nc2x, nc2y, nx, ny)
            }
            PathCmd::Close => PathCmd::Close,
        })
        .collect()
}

fn cmds_bounds(cmds: &[PathCmd]) -> Option<(f64, f64, f64, f64)> {
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut cur = (0.0_f64, 0.0_f64);
    let mut start = (0.0_f64, 0.0_f64);
    let mut any = false;
    for &cmd in cmds {
        match cmd {
            PathCmd::MoveTo(x, y) => {
                cur = (f64::from(x), f64::from(y));
                start = cur;
                include_pt(
                    cur, &mut min_x, &mut min_y, &mut max_x, &mut max_y, &mut any,
                );
            }
            PathCmd::LineTo(x, y) => {
                cur = (f64::from(x), f64::from(y));
                include_pt(
                    cur, &mut min_x, &mut min_y, &mut max_x, &mut max_y, &mut any,
                );
            }
            PathCmd::QuadTo(cx, cy, x, y) => {
                let (cx, cy, x, y) = (f64::from(cx), f64::from(cy), f64::from(x), f64::from(y));
                include_pt(
                    (x, y),
                    &mut min_x,
                    &mut min_y,
                    &mut max_x,
                    &mut max_y,
                    &mut any,
                );
                quad_extrema(cur.0, cx, x, &mut min_x, &mut max_x);
                quad_extrema(cur.1, cy, y, &mut min_y, &mut max_y);
                cur = (x, y);
            }
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                let (c1x, c1y, c2x, c2y, x, y) = (
                    f64::from(c1x),
                    f64::from(c1y),
                    f64::from(c2x),
                    f64::from(c2y),
                    f64::from(x),
                    f64::from(y),
                );
                include_pt(
                    (x, y),
                    &mut min_x,
                    &mut min_y,
                    &mut max_x,
                    &mut max_y,
                    &mut any,
                );
                cubic_extrema(cur.0, c1x, c2x, x, &mut min_x, &mut max_x);
                cubic_extrema(cur.1, c1y, c2y, y, &mut min_y, &mut max_y);
                cur = (x, y);
            }
            PathCmd::Close => cur = start,
        }
    }
    (any && min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite())
        .then_some((min_x, min_y, max_x, max_y))
}

fn include_pt(
    (x, y): (f64, f64),
    min_x: &mut f64,
    min_y: &mut f64,
    max_x: &mut f64,
    max_y: &mut f64,
    any: &mut bool,
) {
    if x.is_finite() && y.is_finite() {
        *min_x = min_x.min(x);
        *min_y = min_y.min(y);
        *max_x = max_x.max(x);
        *max_y = max_y.max(y);
        *any = true;
    }
}
