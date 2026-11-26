//! Variables: collections with modes, variables of the four basic types, values per mode, and
//! binding variables to layer fields.
//!
//! A collection is a `VARIABLE_SET` node (`variableSetModes`), a variable a `VARIABLE` node
//! (`variableSetID`, `variableResolvedType`, `variableDataValues`), both on the internal canvas.
//! A layer binds a field through `variableConsumptionMap` and also stores the resolved value, so
//! readers that do not resolve variables (the renderer, for most fields) draw it right; a value
//! change rewrites the layers that use it.

use serde::{Deserialize, Serialize};

use crate::edit::styles_vars::{ensure_internal_canvas, resolve_effective_mode};
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::tokens::{Tokens, Value};
use crate::{Document, FigError, Guid, Node, Obj, Val, intern};

/// A variable's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarType {
    Color,
    Number,
    String,
    Bool,
}

impl VarType {
    fn stored(self) -> &'static str {
        match self {
            VarType::Color => "COLOR",
            VarType::Number => "FLOAT",
            VarType::String => "STRING",
            VarType::Bool => "BOOLEAN",
        }
    }

    fn of(node: &Node) -> Option<VarType> {
        match node.data.text("variableResolvedType")? {
            "COLOR" => Some(VarType::Color),
            "FLOAT" => Some(VarType::Number),
            "STRING" => Some(VarType::String),
            "BOOLEAN" => Some(VarType::Bool),
            _ => None,
        }
    }
}

/// A value for one mode: a literal of the variable's type, or another variable of that type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarValue {
    /// `#RRGGBB` or `#RRGGBBAA`.
    Color(String),
    Number(f64),
    String(String),
    Bool(bool),
    Alias(#[serde(with = "crate::edit::guid_wire")] Guid),
}

/// A layer field a variable can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindField {
    CornerRadius,
    Opacity,
    StrokeWeight,
    ItemSpacing,
    PaddingLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    Width,
    Height,
    FontSize,
    ParagraphSpacing,
    Visible,
    Characters,
}

impl BindField {
    /// `(VariableField enum value, node field, value type)`.
    fn spec(self) -> (&'static str, &'static str, VarType) {
        use BindField::*;
        match self {
            CornerRadius => ("CORNER_RADIUS", "cornerRadius", VarType::Number),
            Opacity => ("OPACITY", "opacity", VarType::Number),
            StrokeWeight => ("STROKE_WEIGHT", "strokeWeight", VarType::Number),
            ItemSpacing => ("STACK_SPACING", "stackSpacing", VarType::Number),
            PaddingLeft => (
                "STACK_PADDING_LEFT",
                "stackHorizontalPadding",
                VarType::Number,
            ),
            PaddingTop => ("STACK_PADDING_TOP", "stackVerticalPadding", VarType::Number),
            PaddingRight => ("STACK_PADDING_RIGHT", "stackPaddingRight", VarType::Number),
            PaddingBottom => (
                "STACK_PADDING_BOTTOM",
                "stackPaddingBottom",
                VarType::Number,
            ),
            Width => ("WIDTH", "size", VarType::Number),
            Height => ("HEIGHT", "size", VarType::Number),
            FontSize => ("FONT_SIZE", "fontSize", VarType::Number),
            ParagraphSpacing => ("PARAGRAPH_SPACING", "paragraphSpacing", VarType::Number),
            Visible => ("VISIBLE", "visible", VarType::Bool),
            Characters => ("TEXT_DATA", "textData", VarType::String),
        }
    }

    fn from_stored(s: &str) -> Option<BindField> {
        use BindField::*;
        [
            CornerRadius,
            Opacity,
            StrokeWeight,
            ItemSpacing,
            PaddingLeft,
            PaddingTop,
            PaddingRight,
            PaddingBottom,
            Width,
            Height,
            FontSize,
            ParagraphSpacing,
            Visible,
            Characters,
        ]
        .into_iter()
        .find(|f| f.spec().0 == s)
    }
}

fn data_val(t: VarType, value: &VarValue, doc: &Document) -> Result<Val, FigError> {
    let (data_type, v) = match (t, value) {
        (VarType::Color, VarValue::Color(hex)) => (
            "COLOR",
            Obj::from([("colorValue", crate::edit::specs::color_val(hex)?)]),
        ),
        (VarType::Number, VarValue::Number(n)) if n.is_finite() => {
            ("FLOAT", Obj::from([("floatValue", Val::Float(*n))]))
        }
        (VarType::String, VarValue::String(s)) => {
            ("STRING", Obj::from([("textValue", Val::Str(s.clone()))]))
        }
        (VarType::Bool, VarValue::Bool(b)) => {
            ("BOOLEAN", Obj::from([("boolValue", Val::Bool(*b))]))
        }
        (_, VarValue::Alias(id)) => {
            let target = doc
                .node(*id)
                .filter(|n| n.kind == "VARIABLE")
                .ok_or_else(|| FigError::malformed(format!("{id} is not a variable")))?;
            if VarType::of(target) != Some(t) {
                return Err(FigError::malformed(format!(
                    "{id} is not a {} variable",
                    t.stored()
                )));
            }
            (
                "ALIAS",
                Obj::from([("alias", Val::Obj(Obj::from([("guid", id.to_val())])))]),
            )
        }
        _ => {
            return Err(FigError::malformed(format!(
                "the value does not fit a {} variable",
                t.stored()
            )));
        }
    };
    Ok(Val::Obj(Obj::from([
        ("value", Val::Obj(v)),
        ("dataType", Val::Enum(intern(data_type))),
        ("resolvedDataType", Val::Enum(intern(t.stored()))),
    ])))
}

fn node_of<'a>(doc: &'a Document, id: Guid, kind: &str) -> Result<&'a Node, FigError> {
    doc.node(id)
        .filter(|n| n.kind == kind)
        .ok_or_else(|| FigError::malformed(format!("{id} is not a {kind}")))
}

fn modes(set: &Node) -> Vec<Val> {
    set.data
        .get("variableSetModes")
        .map(|v| v.items().iter().map(|m| m.clone().into_owned()).collect())
        .unwrap_or_default()
}

fn on_internal_canvas(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    obj: &mut Obj,
) -> Result<(), FigError> {
    let ic = ensure_internal_canvas(doc, txn, alloc)?;
    let last = doc
        .children(ic)
        .last()
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", ic.to_val()),
            ("position", Val::Str(fractional::between(last, None))),
        ])),
    );
    obj.insert("phase", Val::Enum(intern("CREATED")));
    obj.insert("visible", Val::Bool(true));
    Ok(())
}

fn mode_val(id: Guid, name: &str, sort: String) -> Val {
    Val::Obj(Obj::from([
        ("id", id.to_val()),
        ("name", Val::Str(name.to_string())),
        ("sortPosition", Val::Str(sort)),
    ]))
}

/// Creates a collection with `modes` (at least one) and returns its id, then the mode ids.
pub fn create_collection(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    name: &str,
    mode_names: &[String],
) -> Result<Vec<Guid>, FigError> {
    if name.trim().is_empty() {
        return Err(FigError::malformed("a collection needs a name"));
    }
    let names: Vec<String> = if mode_names.is_empty() {
        vec!["Mode 1".to_string()]
    } else {
        mode_names.to_vec()
    };
    let id = alloc.next_guid();
    let mut out = vec![id];
    let sorts = fractional::n_between(None, None, names.len());
    let list: Vec<Val> = names
        .iter()
        .zip(sorts)
        .map(|(n, s)| {
            let m = alloc.next_guid();
            out.push(m);
            mode_val(m, n, s)
        })
        .collect();
    let mut obj = Obj::new();
    obj.insert("guid", id.to_val());
    obj.insert("type", Val::Enum(intern("VARIABLE_SET")));
    obj.insert("name", Val::Str(name.trim().to_string()));
    obj.insert("variableSetModes", Val::Arr(list));
    on_internal_canvas(doc, txn, alloc, &mut obj)?;
    txn.create_node(id, Val::Obj(obj));
    Ok(out)
}

/// Adds a mode, copying each variable's value from the collection's first mode.
pub fn add_mode(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    collection: Guid,
    name: &str,
) -> Result<Guid, FigError> {
    let set = node_of(doc, collection, "VARIABLE_SET")?;
    let mut list = modes(set);
    let first = list
        .first()
        .and_then(|m| m.get("id").and_then(Val::as_guid));
    let last = list.iter().filter_map(|m| m.text("sortPosition")).max();
    let id = alloc.next_guid();
    list.push(mode_val(id, name, fractional::between(last, None)));
    txn.set_field(collection, "variableSetModes", Val::Arr(list));
    for v in variables_of(doc, collection) {
        let mut entries = entries(v);
        if let Some(src) = entries
            .iter()
            .find(|e| e.get("modeID").and_then(Val::as_guid) == first)
            .and_then(|e| e.get("variableData"))
        {
            let copy = src.clone().into_owned();
            entries.push(Val::Obj(Obj::from([
                ("modeID", id.to_val()),
                ("variableData", copy),
            ])));
            write_entries(txn, v.id, entries);
        }
    }
    Ok(id)
}

/// Renames a mode.
pub fn rename_mode(
    doc: &Document,
    txn: &mut Txn,
    collection: Guid,
    mode: Guid,
    name: &str,
) -> Result<(), FigError> {
    let set = node_of(doc, collection, "VARIABLE_SET")?;
    let mut list = modes(set);
    let m = list
        .iter_mut()
        .find(|m| m.get("id").and_then(Val::as_guid) == Some(mode))
        .ok_or_else(|| FigError::malformed(format!("{collection} has no mode {mode}")))?;
    m.set("name", Val::Str(name.to_string()));
    txn.set_field(collection, "variableSetModes", Val::Arr(list));
    Ok(())
}

/// Removes a mode (not the last) and every variable's value for it.
pub fn remove_mode(
    doc: &Document,
    txn: &mut Txn,
    collection: Guid,
    mode: Guid,
) -> Result<(), FigError> {
    let set = node_of(doc, collection, "VARIABLE_SET")?;
    let mut list = modes(set);
    let before = list.len();
    list.retain(|m| m.get("id").and_then(Val::as_guid) != Some(mode));
    if list.len() == before {
        return Err(FigError::malformed(format!(
            "{collection} has no mode {mode}"
        )));
    }
    if list.is_empty() {
        return Err(FigError::malformed("a collection keeps at least one mode"));
    }
    txn.set_field(collection, "variableSetModes", Val::Arr(list));
    for v in variables_of(doc, collection) {
        let mut e = entries(v);
        e.retain(|e| e.get("modeID").and_then(Val::as_guid) != Some(mode));
        write_entries(txn, v.id, e);
    }
    Ok(())
}

fn set_id_of(v: &Node) -> Option<Guid> {
    v.data
        .get("variableSetID")
        .and_then(|s| s.get("guid").and_then(Val::as_guid).or_else(|| s.as_guid()))
}

fn variables_of(doc: &Document, collection: Guid) -> Vec<&Node> {
    doc.nodes
        .iter()
        .filter(|n| n.kind == "VARIABLE" && set_id_of(n) == Some(collection))
        .collect()
}

fn entries(v: &Node) -> Vec<Val> {
    v.data
        .path(&["variableDataValues", "entries"])
        .map(|e| e.items().iter().map(|x| x.clone().into_owned()).collect())
        .unwrap_or_default()
}

fn write_entries(txn: &mut Txn, id: Guid, entries: Vec<Val>) {
    txn.set_field(
        id,
        "variableDataValues",
        Val::Obj(Obj::from([("entries", Val::Arr(entries))])),
    );
}

/// Creates a variable with `value` in every mode of `collection`; returns its id.
pub fn create_variable(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    collection: Guid,
    name: &str,
    t: VarType,
    value: &VarValue,
) -> Result<Guid, FigError> {
    if name.trim().is_empty() {
        return Err(FigError::malformed("a variable needs a name"));
    }
    let set = node_of(doc, collection, "VARIABLE_SET")?;
    let data = data_val(t, value, doc)?;
    let entries: Vec<Val> = modes(set)
        .iter()
        .filter_map(|m| m.get("id").map(|i| i.clone().into_owned()))
        .map(|mid| Val::Obj(Obj::from([("modeID", mid), ("variableData", data.clone())])))
        .collect();
    let id = alloc.next_guid();
    let mut obj = Obj::new();
    obj.insert("guid", id.to_val());
    obj.insert("type", Val::Enum(intern("VARIABLE")));
    obj.insert("name", Val::Str(name.trim().to_string()));
    obj.insert(
        "variableSetID",
        Val::Obj(Obj::from([("guid", collection.to_val())])),
    );
    obj.insert("variableResolvedType", Val::Enum(intern(t.stored())));
    obj.insert(
        "variableDataValues",
        Val::Obj(Obj::from([("entries", Val::Arr(entries))])),
    );
    on_internal_canvas(doc, txn, alloc, &mut obj)?;
    txn.create_node(id, Val::Obj(obj));
    Ok(id)
}

/// Sets a variable's value for `mode`. The caller then refreshes the layers using it.
pub fn set_value(
    doc: &Document,
    txn: &mut Txn,
    variable: Guid,
    mode: Guid,
    value: &VarValue,
) -> Result<(), FigError> {
    let v = node_of(doc, variable, "VARIABLE")?;
    let t = VarType::of(v).ok_or_else(|| {
        FigError::malformed(format!(
            "{variable} is not a color, number, string or bool variable"
        ))
    })?;
    if *value == VarValue::Alias(variable) {
        return Err(FigError::malformed("a variable cannot alias itself"));
    }
    let set = set_id_of(v)
        .and_then(|s| doc.node(s))
        .ok_or_else(|| FigError::malformed(format!("{variable} has no local collection")))?;
    if !modes(set)
        .iter()
        .any(|m| m.get("id").and_then(Val::as_guid) == Some(mode))
    {
        return Err(FigError::malformed(format!(
            "{} has no mode {mode}",
            set.id
        )));
    }
    let data = data_val(t, value, doc)?;
    let mut e = entries(v);
    match e
        .iter_mut()
        .find(|x| x.get("modeID").and_then(Val::as_guid) == Some(mode))
    {
        Some(x) => x.set("variableData", data),
        None => e.push(Val::Obj(Obj::from([
            ("modeID", mode.to_val()),
            ("variableData", data),
        ]))),
    }
    write_entries(txn, variable, e);
    Ok(())
}

/// Renames a variable.
pub fn rename_variable(
    doc: &Document,
    txn: &mut Txn,
    variable: Guid,
    name: &str,
) -> Result<(), FigError> {
    node_of(doc, variable, "VARIABLE")?;
    if name.trim().is_empty() {
        return Err(FigError::malformed("a variable needs a name"));
    }
    txn.set_field(variable, "name", Val::Str(name.trim().to_string()));
    Ok(())
}

/// Deletes a variable; layers and paints using it keep their current values, unbound.
pub fn delete_variable(doc: &Document, txn: &mut Txn, variable: Guid) -> Result<(), FigError> {
    node_of(doc, variable, "VARIABLE")?;
    let aliases =
        |v: &Val| v.path(&["value", "alias", "guid"]).and_then(Val::as_guid) == Some(variable);
    for n in &doc.nodes {
        if n.id == variable {
            continue;
        }
        let mut set = Obj::new();
        if let Some(map) = n.data.get("variableConsumptionMap") {
            let mut map = map.clone().into_owned();
            if let Some(Val::Arr(es)) = map.get_mut("entries") {
                let before = es.len();
                es.retain(|e| !e.get("variableData").is_some_and(aliases));
                if es.len() != before {
                    set.insert("variableConsumptionMap", map);
                }
            }
        }
        for field in ["fillPaints", "strokePaints"] {
            let Some(list) = n.data.get(field) else {
                continue;
            };
            if list
                .items()
                .iter()
                .any(|p| p.get("colorVar").is_some_and(aliases))
            {
                let kept: Vec<Val> = list
                    .items()
                    .iter()
                    .map(|p| {
                        let mut p = p.clone().into_owned();
                        if p.get("colorVar").is_some_and(aliases)
                            && let Val::Obj(o) = &mut p
                        {
                            o.remove("colorVar");
                        }
                        p
                    })
                    .collect();
                set.insert(field, Val::Arr(kept));
            }
        }
        if n.kind == "VARIABLE" {
            // Another variable aliasing this one keeps the value it resolved to.
            let es = entries(n);
            if es
                .iter()
                .any(|e| e.get("variableData").is_some_and(aliases))
            {
                let tokens = Tokens::new(doc);
                let fixed: Vec<Val> = es
                    .into_iter()
                    .map(|mut e| {
                        let mode = e.get("modeID").and_then(Val::as_guid);
                        if let Some(d) = e.get("variableData").filter(|d| aliases(d))
                            && let (Some(val), Some(t)) = (tokens.resolve(d, mode), VarType::of(n))
                            && let Some(literal) = literal(t, &val)
                        {
                            e.set("variableData", literal);
                        }
                        e
                    })
                    .collect();
                set.insert(
                    "variableDataValues",
                    Val::Obj(Obj::from([("entries", Val::Arr(fixed))])),
                );
            }
        }
        if !set.is_empty() {
            txn.modify(n.id, set, Vec::new());
        }
    }
    txn.delete_node(variable);
    Ok(())
}

fn literal(t: VarType, v: &Value) -> Option<Val> {
    let (dt, value) = match (t, v) {
        (VarType::Color, Value::Color(c)) => ("COLOR", Obj::from([("colorValue", c.clone())])),
        (VarType::Number, Value::Float(f)) => {
            ("FLOAT", Obj::from([("floatValue", Val::Float(*f))]))
        }
        (VarType::String, Value::String(s)) => {
            ("STRING", Obj::from([("textValue", Val::Str(s.clone()))]))
        }
        (VarType::Bool, Value::Bool(b)) => ("BOOLEAN", Obj::from([("boolValue", Val::Bool(*b))])),
        _ => return None,
    };
    Some(Val::Obj(Obj::from([
        ("value", Val::Obj(value)),
        ("dataType", Val::Enum(intern(dt))),
        ("resolvedDataType", Val::Enum(intern(t.stored()))),
    ])))
}

/// The node fields `field` gets for resolved `value` on `node`.
fn field_values(node: &Node, field: BindField, value: &Value) -> Result<Obj, FigError> {
    let (_, name, _) = field.spec();
    let mut set = Obj::new();
    match (field, value) {
        (BindField::Visible, Value::Bool(b)) => {
            set.insert(name, Val::Bool(*b));
        }
        (BindField::Characters, Value::String(s)) => {
            let mut td = node
                .data
                .get("textData")
                .cloned()
                .unwrap_or_else(|| Val::Obj(Obj::new()))
                .into_owned();
            if let Val::Obj(o) = &mut td {
                crate::edit::text_runs::replace_characters(o, s);
            }
            set.insert("textData", td);
        }
        (BindField::Width | BindField::Height, Value::Float(f)) => {
            let (w, h) = crate::edit::space::size(&node.data);
            let (w, h) = if field == BindField::Width {
                (f.max(0.0), h)
            } else {
                (w, f.max(0.0))
            };
            set.insert(
                "size",
                Val::Obj(Obj::from([("x", Val::Float(w)), ("y", Val::Float(h))])),
            );
        }
        (BindField::Opacity, Value::Float(f)) => {
            // Figma's opacity variables hold percentages.
            set.insert(name, Val::Float((f / 100.0).clamp(0.0, 1.0)));
        }
        (_, Value::Float(f)) => {
            set.insert(name, Val::Float(f.max(0.0)));
        }
        _ => {
            return Err(FigError::malformed(format!(
                "the variable's value does not fit {name}"
            )));
        }
    }
    Ok(set)
}

/// The value of `variable` as seen from `node` (its collection's mode there).
fn value_at(doc: &Document, tokens: &Tokens, node: Guid, variable: Guid) -> Option<Value> {
    let v = doc.node(variable)?;
    let mode = set_id_of(v).and_then(|set| resolve_effective_mode(doc, tokens, node, set));
    let alias = Val::Obj(Obj::from([(
        "value",
        Val::Obj(Obj::from([(
            "alias",
            Val::Obj(Obj::from([("guid", variable.to_val())])),
        )])),
    )]));
    tokens.resolve(&alias, mode)
}

/// Binds (or with `None` unbinds, keeping the current value) a variable to a layer field.
pub fn bind(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    field: BindField,
    variable: Option<Guid>,
) -> Result<(), FigError> {
    let node = doc
        .node(target)
        .ok_or_else(|| FigError::malformed(format!("node {target} does not exist")))?;
    let (stored, _, want) = field.spec();
    if matches!(
        field,
        BindField::Characters | BindField::FontSize | BindField::ParagraphSpacing
    ) && node.kind != "TEXT"
    {
        return Err(FigError::malformed(format!("{target} is not a TEXT layer")));
    }
    let mut map = node
        .data
        .get("variableConsumptionMap")
        .cloned()
        .unwrap_or_else(|| Val::Obj(Obj::from([("entries", Val::Arr(Vec::new()))])))
        .into_owned();
    let mut list: Vec<Val> = map
        .get("entries")
        .map(|e| e.items().iter().map(|x| x.clone().into_owned()).collect())
        .unwrap_or_default();
    list.retain(|e| e.text("variableField") != Some(stored));
    let mut set = Obj::new();
    if let Some(var) = variable {
        let v = node_of(doc, var, "VARIABLE")?;
        if VarType::of(v) != Some(want) {
            return Err(FigError::malformed(format!(
                "{var} is not a {} variable",
                want.stored()
            )));
        }
        list.push(Val::Obj(Obj::from([
            ("variableField", Val::Enum(intern(stored))),
            (
                "variableData",
                Val::Obj(Obj::from([
                    (
                        "value",
                        Val::Obj(Obj::from([(
                            "alias",
                            Val::Obj(Obj::from([("guid", var.to_val())])),
                        )])),
                    ),
                    ("dataType", Val::Enum(intern("ALIAS"))),
                    ("resolvedDataType", Val::Enum(intern(want.stored()))),
                ])),
            ),
        ])));
        let tokens = Tokens::new(doc);
        let value = value_at(doc, &tokens, target, var)
            .ok_or_else(|| FigError::malformed(format!("{var} has no value to apply")))?;
        for (k, v) in field_values(node, field, &value)?.iter() {
            set.insert(k, v.clone());
        }
    }
    map.set("entries", Val::Arr(list));
    set.insert("variableConsumptionMap", map);
    txn.modify(target, set, Vec::new());
    Ok(())
}

/// Rewrites every bound field whose variable resolves differently in `doc` (after a value or
/// mode change). Returns the touched layers.
pub fn refresh_bound(doc: &Document, txn: &mut Txn) -> Result<Vec<Guid>, FigError> {
    let tokens = Tokens::new(doc);
    let mut touched = Vec::new();
    for n in &doc.nodes {
        let Some(es) = n.data.path(&["variableConsumptionMap", "entries"]) else {
            continue;
        };
        let mut set = Obj::new();
        for e in es.items() {
            let (Some(field), Some(var)) = (
                e.text("variableField").and_then(BindField::from_stored),
                e.path(&["variableData", "value", "alias", "guid"])
                    .and_then(Val::as_guid),
            ) else {
                continue;
            };
            if doc.node(var).is_none() {
                continue;
            }
            if let Some(value) = value_at(doc, &tokens, n.id, var)
                && let Ok(fields) = field_values(n, field, &value)
            {
                for (k, v) in fields.iter() {
                    if n.data.get(k) != Some(v) {
                        set.insert(k, v.clone());
                    }
                }
            }
        }
        if !set.is_empty() {
            txn.modify(n.id, set, Vec::new());
            touched.push(n.id);
        }
    }
    Ok(touched)
}
