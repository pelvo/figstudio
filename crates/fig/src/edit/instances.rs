//! Components (`SYMBOL`), Variants (`STATE_GROUP`), Component Properties, Instance Overrides
//! (`symbolData.symbolOverrides`), Instance Swap, and Detach Instance.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::edit::geometry::matrix_2x3;
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::resolve::{
    SHAPE_GEOMETRY, STYLED_PAINTS, SWAP_INHERITED, inherit_swapped, resolve_instance,
    swap_inherited_patch, symbol_id,
};
use crate::{Document, FigError, Guid, Node, Obj, Val, intern};

/// Typed component property value for `componentPropAssignments`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ComponentPropVal {
    Bool(bool),
    Text(String),
    InstanceSwap(#[serde(with = "crate::edit::ops::guid_wire")] Guid),
}

/// Sets or merges fields on an inner node inside `outermost_instance` via `symbolOverrides`.
/// `override_path` is the sequence of `Node::override_key()` GUIDs from inside the outermost
/// instance down to the target node (matching [`crate::resolve::Overrides`]).
pub fn set_instance_override(
    doc: &Document,
    txn: &mut Txn,
    outermost_instance: Guid,
    override_path: &[Guid],
    fields: Obj,
    unset: &[String],
) -> Result<(), FigError> {
    if override_path.is_empty() {
        return Err(FigError::malformed("override_path cannot be empty"));
    }
    let Some(inst) = doc.node(outermost_instance) else {
        return Err(FigError::malformed(format!(
            "missing instance {outermost_instance}"
        )));
    };
    if inst.kind != "INSTANCE" {
        return Err(FigError::malformed(format!(
            "node {outermost_instance} is {}, expected INSTANCE",
            inst.kind
        )));
    }

    let mut sym_data = inst
        .data
        .get("symbolData")
        .cloned()
        .unwrap_or_else(|| Val::Obj(Obj::new()))
        .into_owned();
    let mut list: Vec<Val> = sym_data
        .get("symbolOverrides")
        .map(|v| v.items().to_vec())
        .unwrap_or_default();

    let target_guids: Vec<Val> = override_path.iter().map(|g| g.to_val()).collect();
    let existing_pos = list
        .iter()
        .position(|entry| path_guids(entry) == override_path);

    let mut entry_obj = match existing_pos {
        Some(idx) => match list.remove(idx).into_owned() {
            Val::Obj(o) => o,
            _ => Obj::new(),
        },
        None => Obj::new(),
    };
    entry_obj.insert(
        "guidPath",
        Val::Obj(Obj::from([("guids", Val::Arr(target_guids))])),
    );
    for u in unset {
        if u != "guidPath" {
            entry_obj.remove(u);
        }
    }
    for (k, v) in fields.iter() {
        if k == "guidPath" {
            continue;
        }
        entry_obj.insert(k, v.clone());
        for (style_key, paint_key) in STYLED_PAINTS {
            if k == paint_key && fields.get(style_key).is_none() {
                entry_obj.remove(style_key);
            }
        }
    }
    if entry_obj.len() > 1 {
        list.push(Val::Obj(entry_obj));
    }
    sym_data.set("symbolOverrides", Val::Arr(list));

    let mut set = Obj::new();
    set.insert("symbolData", sym_data);

    strip_stale_derived(inst, override_path, &mut set);
    txn.modify(outermost_instance, set, Vec::new());
    Ok(())
}

/// Drops the `derivedSymbolData` entry of `override_path` (stale once its override changes)
/// so layout and ink recompute cleanly; entries of other paths stay.
fn strip_stale_derived(inst: &Node, override_path: &[Guid], set: &mut Obj) {
    if let Some(derived) = inst.data.get("derivedSymbolData") {
        let kept: Vec<Val> = derived
            .items()
            .iter()
            .filter(|e| path_guids(e) != override_path)
            .cloned()
            .collect();
        set.insert("derivedSymbolData", Val::Arr(kept));
    }
}

/// Resets one field, one `override_path`, or all overrides on `outermost_instance`.
pub fn reset_instance_override(
    doc: &Document,
    txn: &mut Txn,
    outermost_instance: Guid,
    override_path: Option<&[Guid]>,
    field: Option<&str>,
) -> Result<(), FigError> {
    let Some(inst) = doc.node(outermost_instance) else {
        return Err(FigError::malformed(format!(
            "missing instance {outermost_instance}"
        )));
    };
    if inst.kind != "INSTANCE" {
        return Err(FigError::malformed(format!(
            "node {outermost_instance} is {}, expected INSTANCE",
            inst.kind
        )));
    }

    let mut set = Obj::new();
    let mut unset = Vec::new();
    let mut sym_data = inst
        .data
        .get("symbolData")
        .cloned()
        .unwrap_or_else(|| Val::Obj(Obj::new()))
        .into_owned();

    match (override_path, field) {
        (None, _) => {
            sym_data.set("symbolOverrides", Val::Arr(Vec::new()));
            set.insert("symbolData", sym_data);
            if let (Some(cur_sym), Some(orig_sym)) = (
                inst.data.get("overriddenSymbolID").and_then(Val::as_guid),
                inst.data
                    .path(&["symbolData", "symbolID"])
                    .and_then(Val::as_guid),
            ) {
                swap_inherited_patch(
                    doc,
                    &inst.data,
                    cur_sym,
                    orig_sym,
                    &AUTO_LAYOUT_CONTAINER,
                    &mut set,
                    &mut unset,
                );
            }
            for k in [
                "overriddenSymbolID",
                "componentPropAssignments",
                "derivedSymbolData",
            ] {
                if inst.data.get(k).is_some() && !unset.iter().any(|u| u == k) {
                    unset.push(k.to_string());
                }
            }
        }
        (Some(path), None) => {
            let kept: Vec<Val> = sym_data
                .get("symbolOverrides")
                .map(Val::items)
                .unwrap_or_default()
                .iter()
                .filter(|e| path_guids(e) != path)
                .cloned()
                .collect();
            sym_data.set("symbolOverrides", Val::Arr(kept));
            set.insert("symbolData", sym_data);
            strip_stale_derived(inst, path, &mut set);
        }
        (Some(path), Some(f)) => {
            let mut out = Vec::new();
            for item in sym_data
                .get("symbolOverrides")
                .map(Val::items)
                .unwrap_or_default()
            {
                if path_guids(item) == path {
                    if let Val::Obj(mut o) = item.clone().into_owned() {
                        o.remove(f);
                        if o.len() > 1 {
                            out.push(Val::Obj(o));
                        }
                    }
                } else {
                    out.push(item.clone());
                }
            }
            sym_data.set("symbolOverrides", Val::Arr(out));
            set.insert("symbolData", sym_data);
            strip_stale_derived(inst, path, &mut set);
        }
    }
    txn.modify(outermost_instance, set, unset);
    Ok(())
}

/// Assigns a `ComponentPropAssignment` (`BOOL`, `TEXT`, or `INSTANCE_SWAP`) on `instance_id`
/// by property `defID` string (`"12:34"`) or human property name.
pub fn set_component_prop_assignment(
    doc: &Document,
    txn: &mut Txn,
    instance_id: Guid,
    prop_name_or_id: &str,
    val: ComponentPropVal,
) -> Result<(), FigError> {
    let Some(inst) = doc.node(instance_id) else {
        return Err(FigError::malformed(format!(
            "missing instance {instance_id}"
        )));
    };
    if inst.kind != "INSTANCE" {
        return Err(FigError::malformed(format!(
            "node {instance_id} is not INSTANCE"
        )));
    }

    let def_id = resolve_prop_def_id(doc, inst, prop_name_or_id).ok_or_else(|| {
        FigError::malformed(format!(
            "component property {prop_name_or_id:?} not found on instance {instance_id}"
        ))
    })?;

    let mut list: Vec<Val> = inst
        .data
        .get("componentPropAssignments")
        .map(|v| v.items().to_vec())
        .unwrap_or_default();
    list.retain(|a| a.get("defID").and_then(Val::as_guid) != Some(def_id));

    let value_obj = match val {
        ComponentPropVal::Bool(b) => Obj::from([("boolValue", Val::Bool(b))]),
        ComponentPropVal::Text(s) => Obj::from([(
            "textValue",
            Val::Obj(Obj::from([("characters", Val::Str(s))])),
        )]),
        ComponentPropVal::InstanceSwap(sym) => Obj::from([("guidValue", sym.to_val())]),
    };

    list.push(Val::Obj(Obj::from([
        ("defID", def_id.to_val()),
        ("value", Val::Obj(value_obj)),
    ])));
    txn.set_field(instance_id, "componentPropAssignments", Val::Arr(list));
    Ok(())
}

/// Switches `instance_id` to a sibling variant matching `variant_pairs` (`("Size", "Large")`)
/// inside its `STATE_GROUP`, migrating overrides by inner layer `(name, kind)` when `overrideKey`
/// differs across variants.
pub fn switch_instance_variant(
    doc: &Document,
    txn: &mut Txn,
    instance_id: Guid,
    variant_pairs: &[(String, String)],
) -> Result<Guid, FigError> {
    let Some(inst) = doc.node(instance_id) else {
        return Err(FigError::malformed(format!(
            "missing instance {instance_id}"
        )));
    };
    let cur_sym_id = crate::resolve::symbol_id(&inst.data)
        .ok_or_else(|| FigError::malformed("instance has no symbolID"))?;
    let Some(cur_sym) = doc.node(cur_sym_id) else {
        return Err(FigError::malformed(format!("missing symbol {cur_sym_id}")));
    };
    let Some(group_id) = cur_sym.parent else {
        return Err(FigError::malformed("symbol has no parent state group"));
    };

    let mut desired = parse_variant_name(&cur_sym.name);
    for (k, v) in variant_pairs {
        desired.insert(k.clone(), v.clone());
    }

    let target_sym = doc
        .children(group_id)
        .iter()
        .filter_map(|&cid| doc.node(cid))
        .filter(|n| n.kind == "SYMBOL")
        .find(|cand| {
            let props = parse_variant_name(&cand.name);
            desired.iter().all(|(k, v)| props.get(k) == Some(v))
        })
        .ok_or_else(|| FigError::malformed("no matching variant SYMBOL found in component set"))?;

    let target_id = target_sym.id;
    let old_key_by_sig = collect_layer_signatures(doc, cur_sym_id);
    let new_key_by_sig = collect_layer_signatures(doc, target_id);
    let mut key_remap: HashMap<Guid, Guid> = HashMap::new();
    for (sig, old_k) in old_key_by_sig {
        if let Some(&new_k) = new_key_by_sig.get(&sig) {
            key_remap.insert(old_k, new_k);
        }
    }

    let mut set = Obj::new();
    let mut unset = if inst.data.get("derivedSymbolData").is_some() {
        vec!["derivedSymbolData".to_string()]
    } else {
        Vec::new()
    };
    swap_inherited_patch(
        doc,
        &inst.data,
        cur_sym_id,
        target_id,
        &AUTO_LAYOUT_CONTAINER,
        &mut set,
        &mut unset,
    );
    set.insert("overriddenSymbolID", target_id.to_val());
    if let Some(mut sym_data) = inst.data.get("symbolData").cloned().map(Val::into_owned) {
        if let Some(Val::Arr(overrides)) = sym_data.get_mut("symbolOverrides") {
            for ov in overrides.iter_mut() {
                if let Some(Val::Arr(guids)) =
                    ov.get_mut("guidPath").and_then(|gp| gp.get_mut("guids"))
                    && guids.len() == 1
                    && let Some(old_g) = guids[0].as_guid()
                    && let Some(&mapped_g) = key_remap.get(&old_g)
                {
                    guids[0] = mapped_g.to_val();
                }
            }
        }
        set.insert("symbolData", sym_data);
    }
    txn.modify(instance_id, set, unset);
    Ok(target_id)
}

/// Swaps `instance_id` to point at `new_symbol_id`.
pub fn swap_instance_symbol(
    doc: &Document,
    txn: &mut Txn,
    instance_id: Guid,
    new_symbol_id: Guid,
) -> Result<(), FigError> {
    let Some(inst) = doc.node(instance_id).filter(|n| n.kind == "INSTANCE") else {
        return Err(FigError::malformed(format!(
            "node {instance_id} is not INSTANCE"
        )));
    };
    if doc.node(new_symbol_id).is_none_or(|n| n.kind != "SYMBOL") {
        return Err(FigError::malformed(format!(
            "node {new_symbol_id} is not SYMBOL"
        )));
    }
    let mut unset = if inst.data.get("derivedSymbolData").is_some() {
        vec!["derivedSymbolData".to_string()]
    } else {
        Vec::new()
    };
    let mut set = Obj::new();
    if let Some(cur_sym_id) = symbol_id(&inst.data) {
        swap_inherited_patch(
            doc,
            &inst.data,
            cur_sym_id,
            new_symbol_id,
            &AUTO_LAYOUT_CONTAINER,
            &mut set,
            &mut unset,
        );
    }
    set.insert("overriddenSymbolID", new_symbol_id.to_val());
    txn.modify(instance_id, set, unset);
    Ok(())
}

/// Detaches `instance_id` into an independent `FRAME` subtree with every override, swap,
/// component property, and uniform scale materialized via [`resolve_instance`].
pub fn detach_instance(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    instance_id: Guid,
) -> Result<Guid, FigError> {
    let Some(inst) = doc.node(instance_id) else {
        return Err(FigError::malformed(format!(
            "missing instance {instance_id}"
        )));
    };
    if inst.kind != "INSTANCE" {
        return Err(FigError::malformed(format!(
            "node {instance_id} is not INSTANCE"
        )));
    }

    let resolved = resolve_instance(doc, instance_id).map_err(|e| {
        FigError::malformed(format!("cannot resolve instance {instance_id}: {e:?}"))
    })?;
    if resolved.truncated {
        // Deleting the instance after copying a partial tree would lose the rest.
        return Err(FigError::malformed(format!(
            "instance {instance_id} is too large to detach"
        )));
    }

    let new_root_id = alloc.next_guid();
    let mut root_data = inst.data.clone().into_owned();
    materialize_instance_as_frame(doc, &inst.data, &mut root_data);
    if let Val::Obj(ref mut o) = root_data {
        o.insert("guid", new_root_id.to_val());
    }
    txn.create_node(new_root_id, root_data);

    // Materialize every resolved descendant in pre-order using a level stack + per-parent
    // sibling counters for fractional positions.
    let mut parent_stack: Vec<Guid> = vec![new_root_id];
    let mut sibling_prev: HashMap<Guid, Option<String>> = HashMap::new();
    for (idx, r) in resolved.nodes.iter().enumerate() {
        let depth = r.level.max(1);
        parent_stack.truncate(depth);
        let parent_guid = *parent_stack.last().unwrap_or(&new_root_id);

        let cid = alloc.next_guid();
        let mut data = r.data.clone().into_owned();
        let has_expanded_kids = resolved
            .nodes
            .get(idx + 1)
            .is_some_and(|next| next.level > r.level);

        if data.text("type") == Some("INSTANCE") && has_expanded_kids {
            let base = doc.node(r.node).map_or(&r.data, |n| &n.data);
            materialize_instance_as_frame(doc, base, &mut data);
        }
        if let Val::Obj(ref mut o) = data {
            o.insert("guid", cid.to_val());
            o.remove("overrideKey");
            let prev_entry = sibling_prev.entry(parent_guid).or_insert(None);
            let pos = fractional::between(prev_entry.as_deref(), None);
            *prev_entry = Some(pos.clone());
            o.insert(
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", parent_guid.to_val()),
                    ("position", Val::Str(pos)),
                ])),
            );
        }
        txn.create_node(cid, data);
        parent_stack.push(cid);
    }

    txn.delete_node(instance_id);
    Ok(new_root_id)
}

/// Converts a `FRAME` node into a reusable `SYMBOL` master component in place.
pub fn convert_frame_to_component(
    doc: &Document,
    txn: &mut Txn,
    node_id: Guid,
) -> Result<Guid, FigError> {
    let Some(node) = doc.node(node_id) else {
        return Err(FigError::malformed(format!("missing node {node_id}")));
    };
    if node.kind != "FRAME" {
        return Err(FigError::malformed(format!(
            "only FRAME can convert to SYMBOL, found {}",
            node.kind
        )));
    }
    txn.set_field(node_id, "type", Val::Enum(intern("SYMBOL")));
    Ok(node_id)
}

/// Instantiates `symbol_id` as a new `INSTANCE` under `parent_id` at `(x, y)`.
pub fn instantiate_component(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    symbol_id: Guid,
    parent_id: Guid,
    x: f64,
    y: f64,
) -> Result<Guid, FigError> {
    let Some(sym) = doc.node(symbol_id) else {
        return Err(FigError::malformed(format!("missing SYMBOL {symbol_id}")));
    };
    if sym.kind != "SYMBOL" {
        return Err(FigError::malformed(format!(
            "node {symbol_id} is {}, expected SYMBOL",
            sym.kind
        )));
    }
    if doc.node(parent_id).is_none() {
        return Err(FigError::malformed(format!("missing parent {parent_id}")));
    }

    let inst_id = alloc.next_guid();
    let last_pos = doc
        .children(parent_id)
        .last()
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    let pos = fractional::between(last_pos, None);
    let size = sym.data.get("size").cloned().unwrap_or_else(|| {
        Val::Obj(Obj::from([
            ("x", Val::Float(100.0)),
            ("y", Val::Float(40.0)),
        ]))
    });

    // An instance draws its own fills, strokes, corners and effects around the component's
    // children, so it starts with the master's (as `inherit_swapped` expects). Pre-baked
    // `SHAPE_GEOMETRY` (`fillGeometry`/`strokeGeometry`) is deliberately omitted so the
    // renderer derives the box from `size` and corner radii through resize, stretch, and swap.
    let mut obj = Obj::new();
    inherit_missing(&mut obj, sym, &SWAP_INHERITED);
    obj.insert("guid", inst_id.to_val());
    obj.insert("phase", Val::Enum(intern("CREATED")));
    obj.insert("type", Val::Enum(intern("INSTANCE")));
    obj.insert("name", Val::Str(sym.name.clone()));
    obj.insert("visible", Val::Bool(true));
    if obj.get("opacity").is_none() {
        obj.insert("opacity", Val::Float(1.0));
    }
    obj.insert("transform", matrix_2x3(x, y, 0.0));
    obj.insert("size", size);
    obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", parent_id.to_val()),
            ("position", Val::Str(pos)),
        ])),
    );
    obj.insert(
        "symbolData",
        Val::Obj(Obj::from([
            ("symbolID", symbol_id.to_val()),
            ("symbolOverrides", Val::Arr(Vec::new())),
            ("uniformScaleFactor", Val::Float(1.0)),
        ])),
    );
    txn.create_node(inst_id, Val::Obj(obj));
    Ok(inst_id)
}

/// Auto-Layout container fields (not the child fields): a detached frame needs them, an
/// instance does not (its children are laid out inside the component).
const AUTO_LAYOUT_CONTAINER: [&str; 24] = [
    "stackMode",
    "stackSpacing",
    "stackCounterSpacing",
    "stackPadding",
    "stackHorizontalPadding",
    "stackVerticalPadding",
    "stackPaddingRight",
    "stackPaddingBottom",
    "stackPrimaryAlignItems",
    "stackCounterAlignItems",
    "stackJustify",
    "stackAlign",
    "stackPrimarySizing",
    "stackCounterSizing",
    "stackWidth",
    "stackHeight",
    "stackWrap",
    "stackReverseZIndex",
    "bordersTakeSpace",
    "gridColumns",
    "gridColumnGap",
    "gridRowGap",
    "minSize",
    "maxSize",
];

/// Converts an `INSTANCE` payload (`data`, with its pre-override `base`) into a standalone
/// `FRAME`, preserving active-master appearance and Auto-Layout container fields without
/// snapping explicitly resized axes back to HUG.
fn materialize_instance_as_frame(doc: &Document, base: &Val, data: &mut Val) {
    inherit_swapped(doc, base, data);
    let old_sym = base
        .path(&["symbolData", "symbolID"])
        .and_then(Val::as_guid);
    let master = symbol_id(data).and_then(|id| doc.node(id));

    if let Val::Obj(ref mut o) = *data {
        if let Some(master) = master {
            if let Some(old_id) = old_sym
                && old_id != master.id
                && let Some(old_master) = doc.node(old_id)
            {
                for &key in &AUTO_LAYOUT_CONTAINER {
                    let own = base.get(key);
                    if own == old_master.data.get(key)
                        && o.get(key) == own
                        && own != master.data.get(key)
                    {
                        match master.data.get(key) {
                            Some(v) => {
                                o.insert(key, v.clone());
                            }
                            None => {
                                o.remove(key);
                            }
                        }
                    }
                }
            }
            inherit_missing(o, master, &SWAP_INHERITED);
            inherit_missing(o, master, &AUTO_LAYOUT_CONTAINER);
            fix_resized_hug_axes(o, master);
        }
        o.insert("type", Val::Enum(intern("FRAME")));
        for strip in [
            "symbolData",
            "derivedSymbolData",
            "derivedSymbolDataLayoutVersion",
            "overriddenSymbolID",
            "componentPropAssignments",
            "overrideKey",
        ] {
            o.remove(strip);
        }
    }
}

/// Switches inherited HUG sizing to `FIXED` on any axis where the detached instance was
/// resized away from its master, so the post-detach Auto-Layout pass keeps the user's size.
fn fix_resized_hug_axes(o: &mut Obj, master: &Node) {
    let horiz = match o.get("stackMode").and_then(Val::as_str) {
        Some("HORIZONTAL" | "GRID") => true,
        Some("VERTICAL") => false,
        _ => return,
    };
    let Some((inst_w, inst_h)) = read_size_obj(o) else {
        return;
    };
    let Some((master_w, master_h)) = read_size_val(&master.data) else {
        return;
    };
    let w_resized = (inst_w - master_w).abs() > 1e-3;
    let h_resized = (inst_h - master_h).abs() > 1e-3;
    if !w_resized && !h_resized {
        return;
    }
    let (primary_resized, counter_resized) = if horiz {
        (w_resized, h_resized)
    } else {
        (h_resized, w_resized)
    };
    if primary_resized {
        fix_hug_field(o, "stackPrimarySizing");
    }
    if counter_resized {
        fix_hug_field(o, "stackCounterSizing");
    }
    if w_resized {
        fix_hug_field(o, "stackWidth");
    }
    if h_resized {
        fix_hug_field(o, "stackHeight");
    }
    for key in SHAPE_GEOMETRY {
        o.remove(key);
    }
}

fn fix_hug_field(o: &mut Obj, key: &'static str) {
    if matches!(
        o.get(key).and_then(Val::as_str),
        Some("RESIZE_TO_FIT" | "RESIZE_TO_FIT_WITH_IMPLICIT_SIZE")
    ) {
        o.insert(key, Val::Enum(intern("FIXED")));
    }
}

fn read_size_val(data: &Val) -> Option<(f64, f64)> {
    let x = f64::from(data.path(&["size", "x"]).and_then(Val::as_f32)?);
    let y = f64::from(data.path(&["size", "y"]).and_then(Val::as_f32)?);
    Some((x, y))
}

fn read_size_obj(o: &Obj) -> Option<(f64, f64)> {
    let size = o.get("size")?;
    let x = f64::from(size.get("x").and_then(Val::as_f32)?);
    let y = f64::from(size.get("y").and_then(Val::as_f32)?);
    Some((x, y))
}

/// Copies each of `keys` that `master` has and `obj` lacks.
fn inherit_missing(obj: &mut Obj, master: &Node, keys: &[&'static str]) {
    for &key in keys {
        if obj.get(key).is_none()
            && let Some(value) = master.data.get(key)
        {
            obj.insert(key, value.clone());
        }
    }
}

/// The value of `field` in `instance`'s override entry for `override_path`, if set.
pub(crate) fn existing_override_field(
    doc: &Document,
    instance: Guid,
    override_path: &[Guid],
    field: &str,
) -> Option<Val> {
    doc.node(instance)?
        .data
        .path(&["symbolData", "symbolOverrides"])?
        .items()
        .iter()
        .find(|e| path_guids(e) == override_path)?
        .get(field)
        .map(|v| v.clone().into_owned())
}

fn path_guids(entry: &Val) -> Vec<Guid> {
    entry
        .path(&["guidPath", "guids"])
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .filter_map(Val::as_guid)
        .collect()
}

fn resolve_prop_def_id(doc: &Document, inst: &Node, query: &str) -> Option<Guid> {
    if let Ok(g) = query.parse::<Guid>() {
        return Some(g);
    }
    let sym_id = crate::resolve::symbol_id(&inst.data)?;
    let sym = doc.node(sym_id)?;
    let mut holders = vec![sym];
    if let Some(p) = sym.parent.and_then(|pid| doc.node(pid)) {
        holders.push(p);
    }
    for h in holders {
        for def in h
            .data
            .get("componentPropDefs")
            .map(Val::items)
            .unwrap_or_default()
        {
            let name = def.text("name").unwrap_or("");
            let base_name = name.split('#').next().unwrap_or(name).trim();
            if name == query || base_name.eq_ignore_ascii_case(query) {
                return def.get("id").and_then(Val::as_guid);
            }
        }
    }
    None
}

pub fn parse_variant_name(name: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in name.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn collect_layer_signatures(doc: &Document, root: Guid) -> HashMap<(String, String), Guid> {
    let mut map = HashMap::new();
    let mut stack: Vec<Guid> = doc.children(root).to_vec();
    while let Some(id) = stack.pop() {
        if let Some(n) = doc.node(id) {
            map.entry((n.kind.clone(), n.name.clone()))
                .or_insert_with(|| n.override_key());
            stack.extend_from_slice(doc.children(id));
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_name_parser_splits_key_value_pairs() {
        let m = parse_variant_name("Size=Large, State=Hover, Icon=True");
        assert_eq!(m.get("Size").map(String::as_str), Some("Large"));
        assert_eq!(m.get("State").map(String::as_str), Some("Hover"));
        assert_eq!(m.get("Icon").map(String::as_str), Some("True"));
    }
}
