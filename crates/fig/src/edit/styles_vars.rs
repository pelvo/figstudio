//! Paints, Effects, Embedded Image SHA-1 Import, Local Styles (`FILL`, `STROKE`, `TEXT`,
//! `EFFECT`, `GRID`), and Variables & Modes (`VARIABLE_SET`, `VARIABLE`, `colorVar`,
//! `variableConsumptionMap`, `variableModeBySetMap`).

use crate::edit::geometry::parse_hex_solid_paint;
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::tokens::Tokens;
use crate::{Document, FigError, Guid, Obj, Val, intern, sha1};

/// Builds a `SOLID` paint `Val` from normalized `0.0..=1.0` RGBA components.
pub fn solid_paint(r: f64, g: f64, b: f64, a: f64) -> Val {
    Val::Obj(Obj::from([
        ("type", Val::Enum(intern("SOLID"))),
        ("visible", Val::Bool(true)),
        ("opacity", Val::Float(a.clamp(0.0, 1.0))),
        ("blendMode", Val::Enum(intern("NORMAL"))),
        (
            "color",
            Val::Obj(Obj::from([
                ("r", Val::Float(r.clamp(0.0, 1.0))),
                ("g", Val::Float(g.clamp(0.0, 1.0))),
                ("b", Val::Float(b.clamp(0.0, 1.0))),
                ("a", Val::Float(1.0)),
            ])),
        ),
    ]))
}

/// Parses `#RRGGBB` or `#RRGGBBAA` into a `SOLID` paint `Val`.
pub fn hex_paint(hex: &str) -> Result<Val, FigError> {
    parse_hex_solid_paint(hex)
        .ok_or_else(|| FigError::malformed(format!("invalid hex colour {hex:?}")))
}

/// Builds a linear/radial/angular/diamond gradient paint `Val`.
pub fn gradient_paint(
    kind: &str,
    stops: &[(f64, [f64; 4])],
    opacity: f64,
) -> Result<Val, FigError> {
    if !matches!(
        kind,
        "GRADIENT_LINEAR" | "GRADIENT_RADIAL" | "GRADIENT_ANGULAR" | "GRADIENT_DIAMOND"
    ) {
        return Err(FigError::malformed(format!(
            "unsupported gradient type {kind}"
        )));
    }
    let stop_vals: Vec<Val> = stops
        .iter()
        .map(|(pos, [r, g, b, a])| {
            Val::Obj(Obj::from([
                ("position", Val::Float(pos.clamp(0.0, 1.0))),
                (
                    "color",
                    Val::Obj(Obj::from([
                        ("r", Val::Float(r.clamp(0.0, 1.0))),
                        ("g", Val::Float(g.clamp(0.0, 1.0))),
                        ("b", Val::Float(b.clamp(0.0, 1.0))),
                        ("a", Val::Float(a.clamp(0.0, 1.0))),
                    ])),
                ),
            ]))
        })
        .collect();
    Ok(Val::Obj(Obj::from([
        ("type", Val::Enum(intern(kind))),
        ("visible", Val::Bool(true)),
        ("opacity", Val::Float(opacity.clamp(0.0, 1.0))),
        ("blendMode", Val::Enum(intern("NORMAL"))),
        ("stops", Val::Arr(stop_vals)),
        (
            "transform",
            Val::Obj(Obj::from([
                ("m00", Val::Float(1.0)),
                ("m01", Val::Float(0.0)),
                ("m02", Val::Float(0.0)),
                ("m10", Val::Float(0.0)),
                ("m11", Val::Float(1.0)),
                ("m12", Val::Float(0.0)),
            ])),
        ),
    ])))
}

/// Imports raw PNG/JPEG/WebP/GIF bytes into `txn.add_images` addressed by lowercase SHA-1 hex
/// and returns the 40-char hash.
pub fn import_image_bytes(txn: &mut Txn, bytes: Vec<u8>) -> Result<String, FigError> {
    if bytes.is_empty() {
        return Err(FigError::malformed("cannot import empty image bytes"));
    }
    let hash = sha1::sha1_hex(&bytes);
    if !txn.add_images.iter().any(|(h, _)| h == &hash) {
        txn.add_images.push((hash.clone(), bytes));
    }
    Ok(hash)
}

/// Builds an `IMAGE` paint referencing a 40-character lowercase hex SHA-1 hash.
pub fn image_paint(sha1_hex: &str, scale_mode: &str, opacity: f64) -> Result<Val, FigError> {
    let raw = parse_hex_bytes(sha1_hex)
        .ok_or_else(|| FigError::malformed(format!("invalid 40-hex image SHA-1 {sha1_hex:?}")))?;
    Ok(Val::Obj(Obj::from([
        ("type", Val::Enum(intern("IMAGE"))),
        ("visible", Val::Bool(true)),
        ("opacity", Val::Float(opacity.clamp(0.0, 1.0))),
        ("blendMode", Val::Enum(intern("NORMAL"))),
        ("imageScaleMode", Val::Enum(intern(scale_mode))),
        (
            "image",
            Val::Obj(Obj::from([
                ("hash", Val::Bytes(raw)),
                ("name", Val::Str(sha1_hex.to_ascii_lowercase())),
            ])),
        ),
    ])))
}

/// Builds a `DROP_SHADOW` or `INNER_SHADOW` `Effect` `Val`.
pub fn shadow_effect(
    inner: bool,
    rgba: [f64; 4],
    offset: (f64, f64),
    radius: f64,
    spread: f64,
) -> Val {
    let kind = if inner { "INNER_SHADOW" } else { "DROP_SHADOW" };
    Val::Obj(Obj::from([
        ("type", Val::Enum(intern(kind))),
        ("visible", Val::Bool(true)),
        ("blendMode", Val::Enum(intern("NORMAL"))),
        (
            "color",
            Val::Obj(Obj::from([
                ("r", Val::Float(rgba[0].clamp(0.0, 1.0))),
                ("g", Val::Float(rgba[1].clamp(0.0, 1.0))),
                ("b", Val::Float(rgba[2].clamp(0.0, 1.0))),
                ("a", Val::Float(rgba[3].clamp(0.0, 1.0))),
            ])),
        ),
        (
            "offset",
            Val::Obj(Obj::from([
                ("x", Val::Float(offset.0)),
                ("y", Val::Float(offset.1)),
            ])),
        ),
        ("radius", Val::Float(radius.max(0.0))),
        ("spread", Val::Float(spread)),
        ("showShadowBehindNode", Val::Bool(true)),
    ]))
}

/// Builds a layer (`FOREGROUND_BLUR`) or background (`BACKGROUND_BLUR`) blur `Effect` `Val`.
pub fn blur_effect(background: bool, radius: f64) -> Val {
    let kind = if background {
        "BACKGROUND_BLUR"
    } else {
        "FOREGROUND_BLUR"
    };
    Val::Obj(Obj::from([
        ("type", Val::Enum(intern(kind))),
        ("visible", Val::Bool(true)),
        ("radius", Val::Float(radius.max(0.0))),
    ]))
}

/// Ensures an internal-only `CANVAS` exists (creating one in `txn` if needed) and returns its `Guid`.
pub fn ensure_internal_canvas(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
) -> Result<Guid, FigError> {
    if let Some(&ic) = doc.internal_pages.first() {
        return Ok(ic);
    }
    let root = doc
        .root
        .ok_or_else(|| FigError::malformed("document has no DOCUMENT root"))?;
    let ic = alloc.next_guid();
    let last_pos = doc
        .children(root)
        .last()
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    let pos = fractional::between(last_pos, None);
    txn.create_node(
        ic,
        Val::Obj(Obj::from([
            ("guid", ic.to_val()),
            ("phase", Val::Enum(intern("CREATED"))),
            ("type", Val::Enum(intern("CANVAS"))),
            ("name", Val::Str("Internal Only Canvas".into())),
            ("visible", Val::Bool(true)),
            ("internalOnly", Val::Bool(true)),
            (
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", root.to_val()),
                    ("position", Val::Str(pos)),
                ])),
            ),
        ])),
    );
    Ok(ic)
}

/// Creates a local style (`FILL`, `STROKE`, `TEXT`, `EFFECT`, `GRID`) on the internal canvas.
pub fn create_local_style(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    style_type: &str,
    name: &str,
    mut payload: Obj,
) -> Result<Guid, FigError> {
    let ic = ensure_internal_canvas(doc, txn, alloc)?;
    let sid = alloc.next_guid();
    let last_pos = doc
        .children(ic)
        .last()
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    let pos = fractional::between(last_pos, None);

    let node_type = match style_type {
        "TEXT" => "TEXT",
        _ => "ROUNDED_RECTANGLE",
    };
    payload.insert("guid", sid.to_val());
    payload.insert("phase", Val::Enum(intern("CREATED")));
    payload.insert("type", Val::Enum(intern(node_type)));
    payload.insert("name", Val::Str(name.into()));
    payload.insert("visible", Val::Bool(true));
    payload.insert("styleType", Val::Enum(intern(style_type)));
    payload.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", ic.to_val()),
            ("position", Val::Str(pos)),
        ])),
    );
    txn.create_node(sid, Val::Obj(payload));
    Ok(sid)
}

/// Binds or unbinds (`style_id = None`) a local style on `node_id` for `slot`
/// (`"fill"`, `"stroke"`, `"text"`, `"effect"`, `"grid"`), syncing the cached paint/effect list.
pub fn bind_style(
    doc: &Document,
    txn: &mut Txn,
    node_id: Guid,
    slot: &str,
    style_id: Option<Guid>,
) -> Result<(), FigError> {
    if doc.node(node_id).is_none() {
        return Err(FigError::malformed(format!("missing node {node_id}")));
    }
    let (style_field, legacy_field, cache_field) = match slot {
        "fill" => (
            "styleIdForFill",
            "inheritFillStyleID",
            Some(("fillPaints", "fillPaints")),
        ),
        "stroke" => (
            "styleIdForStrokeFill",
            "inheritFillStyleIDForStroke",
            Some(("fillPaints", "strokePaints")),
        ),
        "text" => ("styleIdForText", "inheritTextStyleID", None),
        "effect" => (
            "styleIdForEffect",
            "inheritEffectStyleID",
            Some(("effects", "effects")),
        ),
        "grid" => (
            "styleIdForGrid",
            "inheritGridStyleID",
            Some(("layoutGrids", "layoutGrids")),
        ),
        other => return Err(FigError::malformed(format!("unknown style slot {other:?}"))),
    };

    let mut set = Obj::new();
    let mut unset = Vec::new();
    match style_id {
        Some(sid) => {
            let Some(style_node) = doc.node(sid) else {
                return Err(FigError::malformed(format!("missing style node {sid}")));
            };
            set.insert(style_field, Val::Obj(Obj::from([("guid", sid.to_val())])));
            let schema = doc.decoded_schema()?;
            if schema
                .def("NodeChange")
                .is_some_and(|d| d.field(legacy_field).is_some())
            {
                set.insert(legacy_field, sid.to_val());
            }
            if let Some((src_f, dst_f)) = cache_field
                && let Some(cached) = style_node.data.get(src_f)
            {
                set.insert(dst_f, cached.clone());
            }
        }
        None => {
            unset.push(style_field.into());
            unset.push(legacy_field.into());
        }
    }
    txn.modify(node_id, set, unset);
    Ok(())
}

/// Resolves the effective mode `Guid` for `collection_id` at `node_id` by walking `node_id`
/// and its ancestors' `variableModeBySetMap`, falling back to `collection.default_mode`.
pub fn resolve_effective_mode(
    doc: &Document,
    tokens: &Tokens,
    node_id: Guid,
    collection_id: Guid,
) -> Option<Guid> {
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        let node = doc.node(id)?;
        if let Some(entries) = node.data.path(&["variableModeBySetMap", "entries"]) {
            for e in entries.items() {
                let set_guid = e
                    .get("variableSetID")
                    .and_then(|v| v.as_guid().or_else(|| v.get("guid").and_then(Val::as_guid)));
                if set_guid == Some(collection_id)
                    && let Some(mid) = e.get("variableModeID").and_then(Val::as_guid)
                {
                    return Some(mid);
                }
            }
        }
        cur = node.parent;
    }
    tokens
        .collection(collection_id)
        .and_then(|c| c.default_mode)
}

/// Sets an explicit variable mode override on a container (`FRAME`, `SECTION`, `CANVAS`).
pub fn set_explicit_variable_mode(
    doc: &Document,
    txn: &mut Txn,
    container_id: Guid,
    collection_id: Guid,
    mode_id: Option<Guid>,
) -> Result<(), FigError> {
    let Some(node) = doc.node(container_id) else {
        return Err(FigError::malformed(format!("missing node {container_id}")));
    };
    let mut entries: Vec<Val> = node
        .data
        .path(&["variableModeBySetMap", "entries"])
        .map(|v| v.items().to_vec())
        .unwrap_or_default();
    entries.retain(|e| {
        let set_guid = e
            .get("variableSetID")
            .and_then(|v| v.as_guid().or_else(|| v.get("guid").and_then(Val::as_guid)));
        set_guid != Some(collection_id)
    });
    if let Some(mid) = mode_id {
        entries.push(Val::Obj(Obj::from([
            (
                "variableSetID",
                Val::Obj(Obj::from([("guid", collection_id.to_val())])),
            ),
            ("variableModeID", mid.to_val()),
        ])));
    }
    txn.set_field(
        container_id,
        "variableModeBySetMap",
        Val::Obj(Obj::from([("entries", Val::Arr(entries))])),
    );
    Ok(())
}

/// Binds a `COLOR` variable `var_id` onto paint `paint_index` of `fillPaints` or `strokePaints`.
pub fn bind_paint_color_variable(
    doc: &Document,
    txn: &mut Txn,
    node_id: Guid,
    paint_list_field: &str,
    paint_index: usize,
    var_id: Option<Guid>,
) -> Result<(), FigError> {
    let Some(node) = doc.node(node_id) else {
        return Err(FigError::malformed(format!("missing node {node_id}")));
    };
    let mut paints: Vec<Val> = node
        .data
        .get(paint_list_field)
        .map(|v| v.items().to_vec())
        .unwrap_or_default();
    let Some(paint) = paints.get_mut(paint_index) else {
        return Err(FigError::malformed(format!(
            "node {node_id} {paint_list_field}[{paint_index}] out of range"
        )));
    };
    *paint = paint.clone().into_owned();
    if let Val::Obj(ref mut po) = *paint {
        match var_id {
            Some(vid) => {
                po.insert(
                    "colorVar",
                    Val::Obj(Obj::from([
                        ("dataType", Val::Enum(intern("ALIAS"))),
                        ("resolvedDataType", Val::Enum(intern("COLOR"))),
                        (
                            "value",
                            Val::Obj(Obj::from([(
                                "alias",
                                Val::Obj(Obj::from([("guid", vid.to_val())])),
                            )])),
                        ),
                    ])),
                );
            }
            None => {
                po.remove("colorVar");
            }
        }
    }
    txn.set_field(node_id, paint_list_field, Val::Arr(paints));
    Ok(())
}

/// Updates a `VARIABLE` node's value for `mode_id` (`COLOR` hex, `FLOAT`, `STRING`, `BOOL`, or alias).
pub fn set_variable_mode_value(
    doc: &Document,
    txn: &mut Txn,
    variable_id: Guid,
    mode_id: Guid,
    value_val: Val,
    data_type: &str,
    resolved_type: &str,
) -> Result<(), FigError> {
    let Some(var_node) = doc.node(variable_id) else {
        return Err(FigError::malformed(format!(
            "missing VARIABLE {variable_id}"
        )));
    };
    if var_node.kind != "VARIABLE" {
        return Err(FigError::malformed(format!(
            "node {variable_id} is {}, expected VARIABLE",
            var_node.kind
        )));
    }
    let mut entries: Vec<Val> = var_node
        .data
        .path(&["variableDataValues", "entries"])
        .map(|v| v.items().to_vec())
        .unwrap_or_default();

    let var_data = Val::Obj(Obj::from([
        ("dataType", Val::Enum(intern(data_type))),
        ("resolvedDataType", Val::Enum(intern(resolved_type))),
        ("value", value_val),
    ]));

    if let Some(existing) = entries
        .iter_mut()
        .find(|e| e.get("modeID").and_then(Val::as_guid) == Some(mode_id))
    {
        *existing = existing.clone().into_owned();
        existing.set("variableData", var_data);
    } else {
        entries.push(Val::Obj(Obj::from([
            ("modeID", mode_id.to_val()),
            ("variableData", var_data),
        ])));
    }

    txn.set_field(
        variable_id,
        "variableDataValues",
        Val::Obj(Obj::from([("entries", Val::Arr(entries))])),
    );
    Ok(())
}

fn parse_hex_bytes(hex: &str) -> Option<Vec<u8>> {
    let s = hex.trim();
    if s.len() != 40 {
        return None;
    }
    let mut out = Vec::with_capacity(20);
    for i in (0..40).step_by(2) {
        out.push(u8::from_str_radix(&s[i..i + 2], 16).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_paint_and_image_import_compute_valid_payloads() {
        let p = hex_paint("#33669980").expect("hex paint");
        assert_eq!(p.text("type"), Some("SOLID"));
        assert!((p.num("opacity").unwrap() - (128.0 / 255.0)).abs() < 1e-4);

        let mut txn = Txn::new("Import");
        let h = import_image_bytes(&mut txn, b"abc".to_vec()).expect("import");
        assert_eq!(h, "a9993e364706816aba3e25717850c26c9cd0d89d");
        let ip = image_paint(&h, "FILL", 1.0).expect("image paint");
        assert_eq!(ip.text("type"), Some("IMAGE"));
        assert_eq!(
            ip.path(&["image", "hash"])
                .unwrap()
                .as_bytes()
                .unwrap()
                .len(),
            20
        );
    }
}
