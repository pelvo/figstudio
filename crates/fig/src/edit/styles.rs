//! Local styles of every kind: create, update (propagated to every layer using the style),
//! delete (layers keep the look, detached), and text-style binding.
//!
//! Figma keeps a style as a node on the internal canvas with `styleType` (`FILL` paint styles
//! serve fills and strokes, `TEXT`, `EFFECT`, `GRID`) holding the styled fields. A layer using
//! one stores `styleIdFor…` plus its own copy of those fields, so readers never need the style.

use serde::{Deserialize, Serialize};

use crate::edit::specs::{EffectSpec, PaintSpec};
use crate::edit::styles_vars::create_local_style;
use crate::edit::{GuidAlloc, Txn};
use crate::{Document, FigError, Guid, Node, Obj, Val};

/// The kind of a style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleKind {
    /// Paint style, used for fills and strokes.
    Paint,
    Text,
    Effect,
    Grid,
}

impl StyleKind {
    fn stored(self) -> &'static str {
        match self {
            StyleKind::Paint => "FILL",
            StyleKind::Text => "TEXT",
            StyleKind::Effect => "EFFECT",
            StyleKind::Grid => "GRID",
        }
    }

    fn of(node: &Node) -> Option<StyleKind> {
        match node.data.text("styleType")? {
            "FILL" | "STROKE" => Some(StyleKind::Paint),
            "TEXT" => Some(StyleKind::Text),
            "EFFECT" => Some(StyleKind::Effect),
            "GRID" => Some(StyleKind::Grid),
            _ => None,
        }
    }
}

/// Text fields a text style sets on the layers using it.
pub const TEXT_STYLE_FIELDS: [&str; 8] = [
    "fontName",
    "fontSize",
    "lineHeight",
    "letterSpacing",
    "paragraphSpacing",
    "paragraphIndent",
    "textCase",
    "textDecoration",
];

/// For a style kind: `(style field on the style node, layer style-id field, legacy layer
/// field, layer field that receives the style's value)`.
fn bindings(kind: StyleKind) -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    match kind {
        StyleKind::Paint => vec![
            (
                "fillPaints",
                "styleIdForFill",
                "inheritFillStyleID",
                "fillPaints",
            ),
            (
                "fillPaints",
                "styleIdForStrokeFill",
                "inheritFillStyleIDForStroke",
                "strokePaints",
            ),
        ],
        StyleKind::Effect => vec![(
            "effects",
            "styleIdForEffect",
            "inheritEffectStyleID",
            "effects",
        )],
        StyleKind::Grid => vec![(
            "layoutGrids",
            "styleIdForGrid",
            "inheritGridStyleID",
            "layoutGrids",
        )],
        StyleKind::Text => vec![("", "styleIdForText", "inheritTextStyleID", "")],
    }
}

/// The style a layer's `field` refers to.
fn referenced(data: &Val, id_field: &str, legacy: &str) -> Option<Guid> {
    data.get(id_field)
        .and_then(|v| v.get("guid").and_then(Val::as_guid).or_else(|| v.as_guid()))
        .or_else(|| data.get(legacy).and_then(Val::as_guid))
}

/// Where a style's look comes from: an existing layer, and/or paint or effect specs.
#[derive(Clone, Copy, Debug, Default)]
pub struct StyleSource<'a> {
    pub from_node: Option<Guid>,
    pub paints: Option<&'a [PaintSpec]>,
    pub effects: Option<&'a [EffectSpec]>,
}

impl StyleSource<'_> {
    fn is_empty(&self) -> bool {
        self.from_node.is_none() && self.paints.is_none() && self.effects.is_none()
    }
}

/// The styled fields to store on a style of `kind`, from a node and/or specs.
fn payload(doc: &Document, kind: StyleKind, source: StyleSource<'_>) -> Result<Obj, FigError> {
    let StyleSource {
        from_node,
        paints,
        effects,
    } = source;
    let src = from_node
        .map(|id| {
            doc.node(id)
                .ok_or_else(|| FigError::malformed(format!("node {id} does not exist")))
        })
        .transpose()?;
    let copy = |o: &mut Obj, field: &str| -> Result<(), FigError> {
        let node = src.ok_or_else(|| {
            FigError::malformed(format!("a {} style needs from_node", kind.stored()))
        })?;
        let v = node.data.get(field).ok_or_else(|| {
            FigError::malformed(format!("{} has no {field} to make a style from", node.id))
        })?;
        o.insert(field, v.clone().into_owned());
        Ok(())
    };
    let mut o = Obj::new();
    match kind {
        StyleKind::Paint => match paints {
            Some(p) => {
                let list = p
                    .iter()
                    .map(PaintSpec::to_val)
                    .collect::<Result<Vec<_>, _>>()?;
                o.insert("fillPaints", Val::Arr(list));
            }
            None => copy(&mut o, "fillPaints")?,
        },
        StyleKind::Effect => match effects {
            Some(e) => {
                let list = e
                    .iter()
                    .map(EffectSpec::to_val)
                    .collect::<Result<Vec<_>, _>>()?;
                o.insert("effects", Val::Arr(list));
            }
            None => copy(&mut o, "effects")?,
        },
        StyleKind::Grid => copy(&mut o, "layoutGrids")?,
        StyleKind::Text => {
            let node = src.ok_or_else(|| FigError::malformed("a TEXT style needs from_node"))?;
            if node.kind != "TEXT" {
                return Err(FigError::malformed(format!(
                    "{} is not a TEXT layer",
                    node.id
                )));
            }
            for f in TEXT_STYLE_FIELDS {
                if let Some(v) = node.data.get(f) {
                    o.insert(f, v.clone().into_owned());
                }
            }
            o.insert(
                "textData",
                Val::Obj(Obj::from([("characters", Val::Str("Ag".into()))])),
            );
        }
    }
    Ok(o)
}

/// Creates a style and returns its id.
pub fn create(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    kind: StyleKind,
    name: &str,
    source: StyleSource<'_>,
) -> Result<Guid, FigError> {
    if name.trim().is_empty() {
        return Err(FigError::malformed("a style needs a name"));
    }
    let p = payload(doc, kind, source)?;
    create_local_style(doc, txn, alloc, kind.stored(), name.trim(), p)
}

fn style_node(doc: &Document, style: Guid) -> Result<(&Node, StyleKind), FigError> {
    let node = doc
        .node(style)
        .ok_or_else(|| FigError::malformed(format!("style {style} does not exist")))?;
    let kind = StyleKind::of(node)
        .ok_or_else(|| FigError::malformed(format!("{style} is not a style")))?;
    Ok((node, kind))
}

/// The fields a layer using a style of `kind` gets from `style_data`, for `layer_field`.
fn applied(kind: StyleKind, style_data: &Val, style_field: &str, layer_field: &str) -> Obj {
    let mut set = Obj::new();
    if kind == StyleKind::Text {
        for f in TEXT_STYLE_FIELDS {
            if let Some(v) = style_data.get(f) {
                set.insert(f, v.clone().into_owned());
            }
        }
    } else if let Some(v) = style_data.get(style_field) {
        set.insert(layer_field, v.clone().into_owned());
    }
    set
}

/// Updates a style and every layer using it.
pub fn update(
    doc: &Document,
    txn: &mut Txn,
    style: Guid,
    name: Option<&str>,
    source: StyleSource<'_>,
) -> Result<Vec<Guid>, FigError> {
    let (node, kind) = style_node(doc, style)?;
    let mut set = Obj::new();
    if let Some(n) = name {
        if n.trim().is_empty() {
            return Err(FigError::malformed("a style needs a name"));
        }
        set.insert("name", Val::Str(n.trim().to_string()));
    }
    let restyled = !source.is_empty();
    if restyled {
        for (k, v) in payload(doc, kind, source)?.iter() {
            if k != "textData" {
                set.insert(k, v.clone());
            }
        }
    }
    let mut style_data = node.data.clone().into_owned();
    for (k, v) in set.iter() {
        style_data.set(k, v.clone());
    }
    txn.modify(style, set, Vec::new());
    let mut touched = Vec::new();
    if restyled {
        for n in &doc.nodes {
            if n.id == style {
                continue;
            }
            for (sf, id_field, legacy, lf) in bindings(kind) {
                if referenced(&n.data, id_field, legacy) == Some(style) {
                    let fields = applied(kind, &style_data, sf, lf);
                    let unset =
                        if kind == StyleKind::Text && n.data.get("derivedTextData").is_some() {
                            vec!["derivedTextData".to_string()]
                        } else {
                            Vec::new()
                        };
                    txn.modify(n.id, fields, unset);
                    touched.push(n.id);
                }
            }
        }
    }
    Ok(touched)
}

/// Deletes a style; the layers using it keep their look, detached.
pub fn delete(doc: &Document, txn: &mut Txn, style: Guid) -> Result<(), FigError> {
    let (_, kind) = style_node(doc, style)?;
    for n in &doc.nodes {
        for (_, id_field, legacy, _) in bindings(kind) {
            if referenced(&n.data, id_field, legacy) == Some(style) {
                let unset: Vec<String> = [id_field, legacy]
                    .into_iter()
                    .filter(|f| n.data.get(f).is_some())
                    .map(str::to_string)
                    .collect();
                txn.modify(n.id, Obj::new(), unset);
            }
        }
    }
    txn.delete_node(style);
    Ok(())
}

/// The text fields to copy onto a layer bound to text style `style` (for `bind_style`).
pub fn text_fields(doc: &Document, style: Guid) -> Result<Obj, FigError> {
    let (node, kind) = style_node(doc, style)?;
    if kind != StyleKind::Text {
        return Err(FigError::malformed(format!("{style} is not a text style")));
    }
    Ok(applied(kind, &node.data, "", ""))
}
