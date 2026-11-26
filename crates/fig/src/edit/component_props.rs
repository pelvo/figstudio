//! Defining components: component properties (definitions, bindings) and variant sets.
//!
//! Figma stores a property definition in the component's `componentPropDefs` (`id`, `name`,
//! `type`, `initialValue`, `sortPosition`), binds a layer field to it with an entry in that
//! layer's `componentPropRefs` (`defID`, `componentPropNodeField`), and sets it per instance in
//! `componentPropAssignments`. A variant set is a `FRAME` with `isStateGroup: true` holding the
//! variant `SYMBOL`s; its `VARIANT` definitions name the properties, and each variant lists its
//! values in `variantPropSpecs` and in its name (`Size=Large, State=Hover`).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::edit::instances::{ComponentPropVal, parse_variant_name};
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::{Document, FigError, Guid, Node, Obj, Val, intern};

/// The kind of a component property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropKind {
    Bool,
    Text,
    InstanceSwap,
}

impl PropKind {
    fn stored(self) -> &'static str {
        match self {
            PropKind::Bool => "BOOL",
            PropKind::Text => "TEXT",
            PropKind::InstanceSwap => "INSTANCE_SWAP",
        }
    }

    /// The layer field a property of this kind binds to.
    fn node_field(self) -> &'static str {
        match self {
            PropKind::Bool => "VISIBLE",
            PropKind::Text => "TEXT_DATA",
            PropKind::InstanceSwap => "OVERRIDDEN_SYMBOL_ID",
        }
    }

    fn of(def: &Val) -> Option<PropKind> {
        match def.text("type")? {
            "BOOL" => Some(PropKind::Bool),
            "TEXT" => Some(PropKind::Text),
            "INSTANCE_SWAP" => Some(PropKind::InstanceSwap),
            _ => None,
        }
    }
}

/// A `ComponentPropValue` for `value`, checked against `kind`.
fn prop_value(kind: PropKind, value: &ComponentPropVal) -> Result<Val, FigError> {
    let v = match (kind, value) {
        (PropKind::Bool, ComponentPropVal::Bool(b)) => Obj::from([("boolValue", Val::Bool(*b))]),
        (PropKind::Text, ComponentPropVal::Text(s)) => Obj::from([(
            "textValue",
            Val::Obj(Obj::from([("characters", Val::Str(s.clone()))])),
        )]),
        (PropKind::InstanceSwap, ComponentPropVal::InstanceSwap(g)) => {
            Obj::from([("guidValue", g.to_val())])
        }
        _ => {
            return Err(FigError::malformed(format!(
                "a {} property needs a {} default",
                kind.stored(),
                kind.stored()
            )));
        }
    };
    Ok(Val::Obj(v))
}

/// The node that owns properties for `component`: the variant set for a variant, else itself.
fn owner(doc: &Document, component: Guid) -> Result<&Node, FigError> {
    let node = doc
        .node(component)
        .ok_or_else(|| FigError::malformed(format!("node {component} does not exist")))?;
    if is_variant_set(node) {
        return Ok(node);
    }
    if node.kind != "SYMBOL" {
        return Err(FigError::malformed(format!(
            "{component} is {}, not a component or variant set",
            node.kind
        )));
    }
    match node.parent.and_then(|p| doc.node(p)) {
        Some(set) if is_variant_set(set) => Ok(set),
        _ => Ok(node),
    }
}

/// Whether `node` is a variant set (Figma's "component set").
pub fn is_variant_set(node: &Node) -> bool {
    node.data.flag("isStateGroup") == Some(true)
}

fn defs(node: &Node) -> Vec<Val> {
    node.data
        .get("componentPropDefs")
        .map(|v| v.items().iter().map(|d| d.clone().into_owned()).collect())
        .unwrap_or_default()
}

/// The definition's display name (Figma appends `#id` to non-variant names).
fn base_name(def: &Val) -> &str {
    let name = def.text("name").unwrap_or("");
    name.split('#').next().unwrap_or(name).trim()
}

/// The index of the definition matching `prop` (an id or a name).
fn find_def(defs: &[Val], prop: &str) -> Option<usize> {
    let by_id = prop.parse::<Guid>().ok();
    defs.iter().position(|d| {
        d.flag("isDeleted") != Some(true)
            && match by_id {
                Some(id) => d.get("id").and_then(Val::as_guid) == Some(id),
                None => base_name(d) == prop,
            }
    })
}

fn next_sort_position(defs: &[Val]) -> String {
    let last = defs.iter().filter_map(|d| d.text("sortPosition")).max();
    fractional::between(last, None)
}

/// Adds a property to `component` and returns its id.
pub fn add_prop(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    component: Guid,
    name: &str,
    kind: PropKind,
    default: &ComponentPropVal,
) -> Result<Guid, FigError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FigError::malformed("a property needs a name"));
    }
    let owner = owner(doc, component)?;
    let mut list = defs(owner);
    if find_def(&list, name).is_some() {
        return Err(FigError::malformed(format!(
            "{} already has a property named {name:?}",
            owner.id
        )));
    }
    let id = alloc.next_guid();
    list.push(Val::Obj(Obj::from([
        ("id", id.to_val()),
        ("name", Val::Str(name.to_string())),
        ("initialValue", prop_value(kind, default)?),
        ("sortPosition", Val::Str(next_sort_position(&list))),
        ("type", Val::Enum(intern(kind.stored()))),
    ])));
    txn.set_field(owner.id, "componentPropDefs", Val::Arr(list));
    Ok(id)
}

/// Renames a property and/or changes its default.
pub fn edit_prop(
    doc: &Document,
    txn: &mut Txn,
    component: Guid,
    prop: &str,
    name: Option<&str>,
    default: Option<&ComponentPropVal>,
) -> Result<(), FigError> {
    let owner = owner(doc, component)?;
    let mut list = defs(owner);
    let i = find_def(&list, prop).ok_or_else(|| missing(owner.id, prop))?;
    if let Some(name) = name {
        let name = name.trim();
        if name.is_empty() {
            return Err(FigError::malformed("a property needs a name"));
        }
        if find_def(&list, name).is_some_and(|j| j != i) {
            return Err(FigError::malformed(format!(
                "{} already has a property named {name:?}",
                owner.id
            )));
        }
        list[i].set("name", Val::Str(name.to_string()));
    }
    if let Some(default) = default {
        let kind = PropKind::of(&list[i]).ok_or_else(|| {
            FigError::malformed("only BOOL, TEXT and INSTANCE_SWAP defaults can be edited")
        })?;
        list[i].set("initialValue", prop_value(kind, default)?);
    }
    txn.set_field(owner.id, "componentPropDefs", Val::Arr(list));
    Ok(())
}

fn missing(owner: Guid, prop: &str) -> FigError {
    FigError::malformed(format!("{owner} has no property {prop:?}"))
}

/// Removes a property, its bindings inside the component and its assignments on instances.
pub fn remove_prop(
    doc: &Document,
    txn: &mut Txn,
    component: Guid,
    prop: &str,
) -> Result<(), FigError> {
    let owner = owner(doc, component)?;
    let mut list = defs(owner);
    let i = find_def(&list, prop).ok_or_else(|| missing(owner.id, prop))?;
    if list[i].text("type") == Some("VARIANT") {
        return Err(FigError::malformed(
            "variant properties are removed by renaming or removing variants",
        ));
    }
    let id = list[i]
        .get("id")
        .and_then(Val::as_guid)
        .ok_or_else(|| missing(owner.id, prop))?;
    list.remove(i);
    txn.set_field(owner.id, "componentPropDefs", Val::Arr(list));
    let without = |field: &str, key: &str, node: &Node| -> Option<Val> {
        let items = node.data.get(field)?.items();
        let kept: Vec<Val> = items
            .iter()
            .filter(|e| e.get(key).and_then(Val::as_guid) != Some(id))
            .map(|e| e.clone().into_owned())
            .collect();
        (kept.len() != items.len()).then_some(Val::Arr(kept))
    };
    for node in &doc.nodes {
        if let Some(refs) = without("componentPropRefs", "defID", node) {
            txn.set_field(node.id, "componentPropRefs", refs);
        }
        if node.kind == "INSTANCE"
            && let Some(a) = without("componentPropAssignments", "defID", node)
        {
            txn.set_field(node.id, "componentPropAssignments", a);
        }
    }
    Ok(())
}

/// Binds `node`'s field for `prop`'s kind to `prop` (or, with `prop: None`, unbinds `field`).
pub fn bind_prop(
    doc: &Document,
    txn: &mut Txn,
    node_id: Guid,
    prop: Option<&str>,
    unbind_kind: Option<PropKind>,
) -> Result<(), FigError> {
    let node = doc
        .node(node_id)
        .ok_or_else(|| FigError::malformed(format!("node {node_id} does not exist")))?;
    let mut refs: Vec<Val> = node
        .data
        .get("componentPropRefs")
        .map(|v| v.items().iter().map(|r| r.clone().into_owned()).collect())
        .unwrap_or_default();
    let (field, new_ref) = match prop {
        Some(prop) => {
            let component = enclosing_component(doc, node_id).ok_or_else(|| {
                FigError::malformed(format!("{node_id} is not inside a component"))
            })?;
            let owner = owner(doc, component)?;
            let list = defs(owner);
            let i = find_def(&list, prop).ok_or_else(|| missing(owner.id, prop))?;
            let kind = PropKind::of(&list[i]).ok_or_else(|| {
                FigError::malformed("only BOOL, TEXT and INSTANCE_SWAP properties bind to layers")
            })?;
            match kind {
                PropKind::Text if node.kind != "TEXT" => {
                    return Err(FigError::malformed(format!(
                        "{node_id} is not a TEXT layer"
                    )));
                }
                PropKind::InstanceSwap if node.kind != "INSTANCE" => {
                    return Err(FigError::malformed(format!("{node_id} is not an instance")));
                }
                _ => {}
            }
            let id = list[i].get("id").cloned().map(Val::into_owned);
            let r = Val::Obj(Obj::from([
                ("defID", id.unwrap_or_else(|| Guid(0, 0).to_val())),
                (
                    "componentPropNodeField",
                    Val::Enum(intern(kind.node_field())),
                ),
            ]));
            (kind.node_field(), Some(r))
        }
        None => (
            unbind_kind
                .ok_or_else(|| FigError::malformed("unbinding needs the field's kind"))?
                .node_field(),
            None,
        ),
    };
    refs.retain(|r| r.text("componentPropNodeField") != Some(field));
    refs.extend(new_ref);
    txn.set_field(node_id, "componentPropRefs", Val::Arr(refs));
    // The newer binding map wins over `componentPropRefs` when both exist: drop its entry.
    if let Some(map) = node.data.get("parameterConsumptionMap") {
        let mut map = map.clone().into_owned();
        if let Some(Val::Arr(entries)) = map.get_mut("entries") {
            entries.retain(|e| e.text("variableField") != Some(field));
        }
        txn.set_field(node_id, "parameterConsumptionMap", map);
    }
    Ok(())
}

/// The nearest component (or variant) above `id`.
fn enclosing_component(doc: &Document, id: Guid) -> Option<Guid> {
    let mut seen = HashSet::new();
    let mut cur = doc.node(id)?.parent;
    while let Some(c) = cur {
        if !seen.insert(c) {
            return None;
        }
        let node = doc.node(c)?;
        if node.kind == "SYMBOL" {
            return Some(c);
        }
        cur = node.parent;
    }
    None
}

/// `Prop=Value` pairs of a variant: from its name, or `Property 1=<name>` for a plain name.
fn variant_pairs(name: &str) -> Vec<(String, String)> {
    let parsed = parse_variant_name(name);
    if parsed.is_empty() {
        return vec![("Property 1".to_string(), name.trim().to_string())];
    }
    // Keep the order the name gives.
    name.split(',')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn variant_name(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether the file's schema stores variant properties (`VARIANT` definitions and
/// `variantPropSpecs`). Older files name variants only, which is all their Figma reads.
fn stores_variant_props(doc: &Document) -> bool {
    doc.decoded_schema().is_ok_and(|s| {
        s.def("ComponentPropType")
            .is_some_and(|d| d.field("VARIANT").is_some())
            && s.def("NodeChange")
                .is_some_and(|d| d.field("variantPropSpecs").is_some())
    })
}

/// Writes `variantPropSpecs` (when the file stores them) and the name of variant `id` for
/// `pairs`, given the set's `VARIANT` definitions.
fn write_variant(txn: &mut Txn, id: Guid, pairs: &[(String, String)], defs: &[Val]) {
    let specs: Vec<Val> = pairs
        .iter()
        .filter_map(|(k, v)| {
            let def = defs
                .iter()
                .find(|d| d.text("type") == Some("VARIANT") && base_name(d) == k)?;
            Some(Val::Obj(Obj::from([
                ("propDefId", def.get("id")?.clone().into_owned()),
                ("value", Val::Str(v.clone())),
            ])))
        })
        .collect();
    let mut set = Obj::new();
    set.insert("name", Val::Str(variant_name(pairs)));
    if !defs.is_empty() {
        set.insert("variantPropSpecs", Val::Arr(specs));
    }
    txn.modify(id, set, Vec::new());
}

/// The `VARIANT` definitions for every property the variants name, existing ones kept.
fn variant_defs(
    mut list: Vec<Val>,
    alloc: &mut GuidAlloc,
    all_pairs: &[Vec<(String, String)>],
) -> Vec<Val> {
    for pairs in all_pairs {
        for (k, v) in pairs {
            let exists = list
                .iter()
                .any(|d| d.text("type") == Some("VARIANT") && base_name(d) == k);
            if !exists {
                let pos = next_sort_position(&list);
                list.push(Val::Obj(Obj::from([
                    ("id", alloc.next_guid().to_val()),
                    ("name", Val::Str(k.clone())),
                    (
                        "initialValue",
                        Val::Obj(Obj::from([(
                            "textValue",
                            Val::Obj(Obj::from([("characters", Val::Str(v.clone()))])),
                        )])),
                    ),
                    ("sortPosition", Val::Str(pos)),
                    ("type", Val::Enum(intern("VARIANT"))),
                ])));
            }
        }
    }
    list
}

/// Turns the frame `set` (just created around the components) into a variant set: marks it,
/// adds the `VARIANT` definitions and writes each variant's values and name.
pub fn make_variant_set(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    set: Guid,
    components: &[Guid],
) -> Result<(), FigError> {
    let mut all = Vec::with_capacity(components.len());
    let mut seen = HashSet::new();
    for &c in components {
        let node = doc
            .node(c)
            .ok_or_else(|| FigError::malformed(format!("node {c} does not exist")))?;
        if node.kind != "SYMBOL" {
            return Err(FigError::malformed(format!(
                "{c} is {}, not a component",
                node.kind
            )));
        }
        if node
            .parent
            .and_then(|p| doc.node(p))
            .is_some_and(is_variant_set)
        {
            return Err(FigError::malformed(format!("{c} is already a variant")));
        }
        let mut pairs = variant_pairs(&node.name);
        // Identical values would make two variants indistinguishable: number the repeats.
        let mut n = 2;
        let base = pairs.last().map(|p| p.1.clone()).unwrap_or_default();
        while !seen.insert(variant_name(&pairs)) {
            if let Some(last) = pairs.last_mut() {
                last.1 = format!("{base} {n}");
            }
            n += 1;
        }
        all.push(pairs);
    }
    let list = if stores_variant_props(doc) {
        variant_defs(Vec::new(), alloc, &all)
    } else {
        Vec::new()
    };
    let mut s = Obj::new();
    s.insert("isStateGroup", Val::Bool(true));
    if !list.is_empty() {
        s.insert("componentPropDefs", Val::Arr(list.clone()));
    }
    // A component set draws no fill of its own.
    s.insert("fillPaints", Val::Arr(Vec::new()));
    txn.modify(set, s, Vec::new());
    for (&c, pairs) in components.iter().zip(&all) {
        write_variant(txn, c, pairs, &list);
    }
    Ok(())
}

/// After `new_id` was pasted as a copy of variant `from` in `set`, gives it a value no
/// variant has yet for the set's first property, and grows the set to show it.
pub fn finish_added_variant(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    set: Guid,
    from: Guid,
    new_id: Guid,
) -> Result<(), FigError> {
    let set_node = doc
        .node(set)
        .ok_or_else(|| FigError::malformed(format!("node {set} does not exist")))?;
    let from_node = doc
        .node(from)
        .ok_or_else(|| FigError::malformed(format!("node {from} does not exist")))?;
    let mut pairs = variant_pairs(&from_node.name);
    let names: HashSet<String> = doc
        .children(set)
        .iter()
        .filter_map(|&c| doc.node(c))
        .map(|n| variant_name(&variant_pairs(&n.name)))
        .collect();
    let base = pairs.first().map(|p| p.1.clone()).unwrap_or_default();
    let mut n = 2;
    loop {
        if let Some(first) = pairs.first_mut() {
            first.1 = format!("{base} {n}");
        }
        if !names.contains(&variant_name(&pairs)) {
            break;
        }
        n += 1;
    }
    let list = if stores_variant_props(doc) {
        let list = variant_defs(defs(set_node), alloc, std::slice::from_ref(&pairs));
        txn.set_field(set, "componentPropDefs", Val::Arr(list.clone()));
        list
    } else {
        Vec::new()
    };
    write_variant(txn, new_id, &pairs, &list);
    Ok(())
}

/// A component property as an editor shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct PropView {
    /// The definition's id (`prop` in the ops); a variant property may have none.
    pub id: Option<Guid>,
    pub name: String,
    pub value: PropViewValue,
}

/// A property's value, and for a variant property the values its set offers.
#[derive(Clone, Debug, PartialEq)]
pub enum PropViewValue {
    Bool(bool),
    Text(String),
    InstanceSwap(Option<Guid>),
    Variant { value: String, options: Vec<String> },
}

fn stored_value(def_type: &str, v: Option<&Val>) -> Option<PropViewValue> {
    Some(match def_type {
        "BOOL" => PropViewValue::Bool(v.and_then(|v| v.flag("boolValue")).unwrap_or(false)),
        "TEXT" => PropViewValue::Text(
            v.and_then(|v| v.path(&["textValue", "characters"]))
                .and_then(Val::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        "INSTANCE_SWAP" => {
            PropViewValue::InstanceSwap(v.and_then(|v| v.get("guidValue")).and_then(Val::as_guid))
        }
        _ => return None,
    })
}

/// The properties of `component` (a component, a variant, or a variant set) in their order:
/// defaults, and for a variant its own variant values (a set shows its first variant's).
pub fn component_props(doc: &Document, component: Guid) -> Vec<PropView> {
    let Ok(owner) = owner(doc, component) else {
        return Vec::new();
    };
    let variant_name = doc
        .node(component)
        .filter(|n| n.kind == "SYMBOL")
        .or_else(|| {
            doc.children(owner.id)
                .iter()
                .filter_map(|c| doc.node(*c))
                .find(|c| c.kind == "SYMBOL")
        })
        .map(|n| n.name.clone())
        .unwrap_or_default();
    props(doc, owner, &variant_name, |_| None)
}

/// The properties of `instance` with its values: its assignments, else the defaults; its
/// variant values come from its main component's name.
pub fn instance_props(doc: &Document, instance: Guid) -> Vec<PropView> {
    let Some(inst) = doc.node(instance).filter(|n| n.kind == "INSTANCE") else {
        return Vec::new();
    };
    let Some(main) = crate::resolve::symbol_id(&inst.data).and_then(|s| doc.node(s)) else {
        return Vec::new();
    };
    let Ok(owner) = owner(doc, main.id) else {
        return Vec::new();
    };
    let assigned: Vec<Val> = inst
        .data
        .get("componentPropAssignments")
        .map(|v| v.items().iter().map(|a| a.clone().into_owned()).collect())
        .unwrap_or_default();
    props(doc, owner, &main.name, |id| {
        assigned
            .iter()
            .find(|a| a.get("defID").and_then(Val::as_guid) == Some(id))
            .and_then(|a| a.get("value"))
            .map(|v| v.clone().into_owned())
    })
}

fn props(
    doc: &Document,
    owner: &Node,
    variant_name: &str,
    assigned: impl Fn(Guid) -> Option<Val>,
) -> Vec<PropView> {
    let mut defs: Vec<Val> = defs(owner)
        .into_iter()
        .filter(|d| d.flag("isDeleted") != Some(true))
        .collect();
    defs.sort_by(|a, b| a.text("sortPosition").cmp(&b.text("sortPosition")));
    let mine = parse_variant_name(variant_name);
    let variants: Vec<std::collections::HashMap<String, String>> = doc
        .children(owner.id)
        .iter()
        .filter_map(|c| doc.node(*c))
        .filter(|c| c.kind == "SYMBOL")
        .map(|c| parse_variant_name(&c.name))
        .collect();
    defs.iter()
        .filter_map(|d| {
            let id = d.get("id").and_then(Val::as_guid);
            let name = base_name(d).to_string();
            let ty = d.text("type").unwrap_or("");
            let value = if ty == "VARIANT" {
                let mut options: Vec<String> = Vec::new();
                for v in variants.iter().filter_map(|m| m.get(&name)) {
                    if !options.contains(v) {
                        options.push(v.clone());
                    }
                }
                PropViewValue::Variant {
                    value: mine.get(&name).cloned().unwrap_or_default(),
                    options,
                }
            } else {
                let stored = id.and_then(&assigned);
                stored_value(ty, stored.as_ref().or(d.get("initialValue")))?
            };
            Some(PropView { id, name, value })
        })
        .collect()
}
