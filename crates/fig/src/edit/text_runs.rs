//! UTF-16 code-unit `TextRuns` editing (`characters`, `characterStyleIDs`,
//! `styleOverrideTable` GC/compaction, `derivedTextData` invalidation, and `textAutoResize`
//! intrinsic box estimation).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::edit::geometry::{matrix_2x3, parse_hex_solid_paint};
use crate::edit::{Txn, fractional};
use crate::{Document, FigError, Guid, Obj, Val, intern};

/// Style patch applied to a UTF-16 character range in a `TEXT` layer.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TextStyleRunPatch {
    pub font_family: Option<String>,
    pub font_style: Option<String>,
    pub font_size: Option<f64>,
    pub fill_hex: Option<String>,
    pub text_decoration: Option<String>,
    pub text_case: Option<String>,
    pub letter_spacing_px: Option<f64>,
    pub line_height_px: Option<f64>,
}

impl TextStyleRunPatch {
    pub fn apply_to_override_obj(&self, obj: &mut Obj) {
        if self.font_family.is_some() || self.font_style.is_some() {
            let fam = self
                .font_family
                .clone()
                .or_else(|| {
                    obj.get("fontName")
                        .and_then(|f| f.text("family"))
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "Inter".into());
            let sty = self
                .font_style
                .clone()
                .or_else(|| {
                    obj.get("fontName")
                        .and_then(|f| f.text("style"))
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "Regular".into());
            obj.insert(
                "fontName",
                Val::Obj(Obj::from([
                    ("family", Val::Str(fam)),
                    ("style", Val::Str(sty)),
                    ("postscript", Val::Str(String::new())),
                ])),
            );
        }
        if let Some(sz) = self.font_size {
            obj.insert("fontSize", Val::Float(sz.max(1.0)));
        }
        if let Some(ref hex) = self.fill_hex
            && let Some(paint) = parse_hex_solid_paint(hex)
        {
            obj.insert("fillPaints", Val::Arr(vec![paint]));
        }
        if let Some(ref dec) = self.text_decoration {
            obj.insert("textDecoration", Val::Enum(intern(dec)));
        }
        if let Some(ref tc) = self.text_case {
            obj.insert("textCase", Val::Enum(intern(tc)));
        }
        if let Some(ls) = self.letter_spacing_px {
            obj.insert(
                "letterSpacing",
                Val::Obj(Obj::from([
                    ("value", Val::Float(ls)),
                    ("units", Val::Enum(intern("PIXELS"))),
                ])),
            );
        }
        if let Some(lh) = self.line_height_px {
            obj.insert(
                "lineHeight",
                Val::Obj(Obj::from([
                    ("value", Val::Float(lh.max(1.0))),
                    ("units", Val::Enum(intern("PIXELS"))),
                ])),
            );
        }
    }
}

/// Creates a brand-new `TEXT` node under `parent`.
#[expect(clippy::too_many_arguments)]
pub fn create_text_node(
    doc: &Document,
    txn: &mut Txn,
    new_id: Guid,
    parent: Guid,
    x: f64,
    y: f64,
    characters: &str,
    font_family: &str,
    font_style: &str,
    font_size: f64,
    fill_hex: &str,
) -> Result<Guid, FigError> {
    if doc.node(parent).is_none() {
        return Err(FigError::malformed(format!(
            "parent {parent} does not exist"
        )));
    }
    let last_pos = doc
        .children(parent)
        .last()
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    let pos = fractional::between(last_pos, None);
    let (est_w, est_h) = estimate_text_box(characters, font_size, None, None, "WIDTH_AND_HEIGHT");

    let mut obj = Obj::new();
    obj.insert("guid", new_id.to_val());
    obj.insert("phase", Val::Enum(intern("CREATED")));
    obj.insert("type", Val::Enum(intern("TEXT")));
    obj.insert(
        "name",
        Val::Str(
            characters
                .lines()
                .next()
                .unwrap_or("Text")
                .chars()
                .take(40)
                .collect(),
        ),
    );
    obj.insert("visible", Val::Bool(true));
    obj.insert("opacity", Val::Float(1.0));
    obj.insert("autoRename", Val::Bool(true));
    obj.insert("transform", matrix_2x3(x, y, 0.0));
    obj.insert(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(est_w)),
            ("y", Val::Float(est_h)),
        ])),
    );
    obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", parent.to_val()),
            ("position", Val::Str(pos)),
        ])),
    );
    obj.insert("horizontalConstraint", Val::Enum(intern("MIN")));
    obj.insert("verticalConstraint", Val::Enum(intern("MIN")));
    obj.insert("strokeWeight", Val::Float(1.0));
    obj.insert("strokeAlign", Val::Enum(intern("OUTSIDE")));
    obj.insert("strokeJoin", Val::Enum(intern("MITER")));
    obj.insert(
        "fontName",
        Val::Obj(Obj::from([
            ("family", Val::Str(font_family.into())),
            ("style", Val::Str(font_style.into())),
            ("postscript", Val::Str(String::new())),
        ])),
    );
    obj.insert("fontSize", Val::Float(font_size.max(1.0)));
    obj.insert("textAutoResize", Val::Enum(intern("WIDTH_AND_HEIGHT")));
    obj.insert("textAlignHorizontal", Val::Enum(intern("LEFT")));
    obj.insert("textAlignVertical", Val::Enum(intern("TOP")));
    obj.insert(
        "lineHeight",
        Val::Obj(Obj::from([
            ("value", Val::Float(100.0)),
            ("units", Val::Enum(intern("RAW"))),
        ])),
    );
    obj.insert(
        "letterSpacing",
        Val::Obj(Obj::from([
            ("value", Val::Float(0.0)),
            ("units", Val::Enum(intern("PERCENT"))),
        ])),
    );
    if let Some(paint) = parse_hex_solid_paint(fill_hex) {
        obj.insert("fillPaints", Val::Arr(vec![paint]));
    }
    obj.insert(
        "textData",
        Val::Obj(Obj::from([("characters", Val::Str(characters.into()))])),
    );

    txn.create_node(new_id, Val::Obj(obj));
    Ok(new_id)
}

/// Replaces the full text of `node_id`, dropping stale `derivedTextData` and resizing the box
/// according to `textAutoResize`.
pub fn set_text_plain(
    doc: &Document,
    txn: &mut Txn,
    node_id: Guid,
    new_text: &str,
) -> Result<(), FigError> {
    let Some(node) = doc.node(node_id) else {
        return Err(FigError::malformed(format!("missing TEXT node {node_id}")));
    };
    if node.kind != "TEXT" {
        return Err(FigError::malformed(format!(
            "node {node_id} is {}, expected TEXT",
            node.kind
        )));
    }
    let mut set = Obj::new();
    let mut td = node
        .data
        .get("textData")
        .cloned()
        .unwrap_or_else(|| Val::Obj(Obj::new()))
        .into_owned();
    if let Val::Obj(ref mut tdo) = td {
        tdo.insert("characters", Val::Str(new_text.into()));
        tdo.remove("characterStyleIDs");
        tdo.remove("styleOverrideTable");
        tdo.remove("glyphs");
        tdo.remove("baselines");
        tdo.remove("lines");
        tdo.remove("derivedLines");
    }
    set.insert("textData", td);
    if node.data.flag("autoRename") == Some(true) {
        let auto_name: String = new_text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(48)
            .collect();
        if !auto_name.is_empty() {
            set.insert("name", Val::Str(auto_name));
        }
    }
    update_auto_resize_box(&node.data, new_text, &mut set);
    let unset = if node.data.get("derivedTextData").is_some() {
        vec!["derivedTextData".to_string()]
    } else {
        Vec::new()
    };
    txn.modify(node_id, set, unset);
    Ok(())
}

/// Splices `[utf16_start .. utf16_start + utf16_delete_len]` in `textData` and inserts `insert`,
/// shifting `characterStyleIDs` and compacting unused entries in `styleOverrideTable`.
pub fn splice_text_utf16(
    text_data: &Val,
    utf16_start: usize,
    utf16_delete_len: usize,
    insert: &str,
) -> Result<Val, FigError> {
    let chars_str = text_data.text("characters").unwrap_or("");
    let mut code_units: Vec<u16> = chars_str.encode_utf16().collect();
    if utf16_start > code_units.len()
        || utf16_start.saturating_add(utf16_delete_len) > code_units.len()
    {
        return Err(FigError::malformed("UTF-16 splice range out of bounds"));
    }

    let mut style_ids = expanded_style_ids(text_data, code_units.len());
    let inherited_style = if utf16_start > 0 {
        style_ids[utf16_start - 1]
    } else {
        style_ids.first().copied().unwrap_or(0)
    };

    let ins_units: Vec<u16> = insert.encode_utf16().collect();
    let end = utf16_start + utf16_delete_len;
    code_units.splice(utf16_start..end, ins_units.iter().copied());
    style_ids.splice(
        utf16_start..end,
        std::iter::repeat_n(inherited_style, ins_units.len()),
    );

    let new_str = String::from_utf16(&code_units)
        .map_err(|_| FigError::malformed("splice split a UTF-16 surrogate pair"))?;
    let table: Vec<Val> = text_data
        .get("styleOverrideTable")
        .map(|v| v.items().to_vec())
        .unwrap_or_default();

    Ok(pack_text_data(&new_str, style_ids, table))
}

/// Applies `patch` onto UTF-16 range `[utf16_start .. utf16_end]` of `text_data` and compacts
/// `styleOverrideTable`.
pub fn apply_text_range_style(
    text_data: &Val,
    utf16_start: usize,
    utf16_end: usize,
    patch: &TextStyleRunPatch,
) -> Result<Val, FigError> {
    let chars_str = text_data.text("characters").unwrap_or("");
    let u16_len = chars_str.encode_utf16().count();
    if utf16_start > utf16_end || utf16_end > u16_len {
        return Err(FigError::malformed("UTF-16 style range out of bounds"));
    }
    if utf16_start == utf16_end {
        return Ok(text_data.clone().into_owned());
    }

    let mut style_ids = expanded_style_ids(text_data, u16_len);
    let mut table: Vec<Val> = text_data
        .get("styleOverrideTable")
        .map(|v| v.items().to_vec())
        .unwrap_or_default();

    let mut next_id = table
        .iter()
        .filter_map(|e| e.get("styleID").and_then(Val::as_u32))
        .max()
        .unwrap_or(0)
        + 1;

    // Cache derived style ID per base style ID within the range so contiguous runs share one entry.
    let mut derived_for_base: HashMap<u32, u32> = HashMap::new();
    for slot in &mut style_ids[utf16_start..utf16_end] {
        let base_id = *slot;
        let mapped = *derived_for_base.entry(base_id).or_insert_with(|| {
            let mut obj = table
                .iter()
                .find(|e| e.get("styleID").and_then(Val::as_u32) == Some(base_id))
                .and_then(|e| match e.clone().into_owned() {
                    Val::Obj(o) => Some(o),
                    _ => None,
                })
                .unwrap_or_default();
            let sid = next_id;
            next_id += 1;
            obj.insert("styleID", Val::Int(i64::from(sid)));
            patch.apply_to_override_obj(&mut obj);
            table.push(Val::Obj(obj));
            sid
        });
        *slot = mapped;
    }

    Ok(pack_text_data(chars_str, style_ids, table))
}

/// Applies `new_text_data` onto `node_id` in `txn`, clearing stale glyph caches and resizing.
pub fn commit_text_data(
    doc: &Document,
    txn: &mut Txn,
    node_id: Guid,
    new_text_data: Val,
) -> Result<(), FigError> {
    let Some(node) = doc.node(node_id) else {
        return Err(FigError::malformed(format!("missing TEXT node {node_id}")));
    };
    let chars = new_text_data.text("characters").unwrap_or("").to_string();
    let mut set = Obj::new();
    set.insert("textData", new_text_data);
    update_auto_resize_box(&node.data, &chars, &mut set);
    let unset = if node.data.get("derivedTextData").is_some() {
        vec!["derivedTextData".to_string()]
    } else {
        Vec::new()
    };
    txn.modify(node_id, set, unset);
    Ok(())
}

fn expanded_style_ids(text_data: &Val, len: usize) -> Vec<u32> {
    let mut ids: Vec<u32> = text_data
        .get("characterStyleIDs")
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .map(|v| v.as_u32().unwrap_or(0))
        .collect();
    ids.resize(len, 0);
    ids
}

fn pack_text_data(characters: &str, style_ids: Vec<u32>, raw_table: Vec<Val>) -> Val {
    let mut out = Obj::new();
    out.insert("characters", Val::Str(characters.into()));

    if style_ids.iter().all(|&id| id == 0) {
        return Val::Obj(out);
    }

    // Deduplicate and compact used non-zero style entries to 1..=k!
    let mut old_by_id: HashMap<u32, Obj> = HashMap::new();
    for entry in raw_table {
        if let Val::Obj(mut o) = entry.into_owned()
            && let Some(sid) = o.get("styleID").and_then(Val::as_u32)
            && sid != 0
        {
            o.remove("styleID");
            old_by_id.insert(sid, o);
        }
    }

    let mut canonical: Vec<Obj> = Vec::new();
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut new_ids = Vec::with_capacity(style_ids.len());
    for old_sid in style_ids {
        if old_sid == 0 {
            new_ids.push(0);
            continue;
        }
        let Some(body) = old_by_id.get(&old_sid) else {
            new_ids.push(0);
            continue;
        };
        let mapped = *remap.entry(old_sid).or_insert_with(|| {
            if let Some(pos) = canonical.iter().position(|c| c == body) {
                (pos + 1) as u32
            } else {
                canonical.push(body.clone());
                canonical.len() as u32
            }
        });
        new_ids.push(mapped);
    }

    // Trim trailing zeros like Figma does to save space.
    while new_ids.last() == Some(&0) {
        new_ids.pop();
    }

    if !new_ids.is_empty() && !canonical.is_empty() {
        out.insert(
            "characterStyleIDs",
            Val::Arr(
                new_ids
                    .into_iter()
                    .map(|i| Val::Int(i64::from(i)))
                    .collect(),
            ),
        );
        let table_vals: Vec<Val> = canonical
            .into_iter()
            .enumerate()
            .map(|(idx, mut o)| {
                o.insert("styleID", Val::Int((idx + 1) as i64));
                Val::Obj(o)
            })
            .collect();
        out.insert("styleOverrideTable", Val::Arr(table_vals));
    }
    Val::Obj(out)
}

fn update_auto_resize_box(node_data: &Val, text: &str, set: &mut Obj) {
    let mode = node_data.text("textAutoResize").unwrap_or("NONE");
    if mode == "NONE" {
        return;
    }
    let font_size = f64::from(node_data.num("fontSize").unwrap_or(12.0)).max(1.0);
    let cur_w = node_data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .map(f64::from);
    let line_height_px = match (
        node_data
            .path(&["lineHeight", "units"])
            .and_then(Val::as_str),
        node_data
            .path(&["lineHeight", "value"])
            .and_then(Val::as_f32),
    ) {
        (Some("PIXELS"), Some(v)) if v > 0.0 => Some(f64::from(v)),
        (Some("PERCENT"), Some(v)) if v > 0.0 => Some(f64::from(v) / 100.0 * font_size),
        _ => None,
    };
    let (nw, nh) = estimate_text_box(text, font_size, line_height_px, cur_w, mode);
    set.insert(
        "size",
        Val::Obj(Obj::from([("x", Val::Float(nw)), ("y", Val::Float(nh))])),
    );
}

fn estimate_text_box(
    text: &str,
    font_size: f64,
    line_height_px: Option<f64>,
    fixed_width: Option<f64>,
    mode: &str,
) -> (f64, f64) {
    let lh = line_height_px.unwrap_or(font_size * 1.25);
    let char_adv = |ch: char| -> f64 {
        if ch > '\u{2E7F}' {
            font_size * 1.0
        } else if ch.is_whitespace() {
            font_size * 0.3
        } else {
            font_size * 0.56
        }
    };
    if mode == "WIDTH_AND_HEIGHT" {
        let mut max_w = 0.0_f64;
        let mut line_count = 0usize;
        for line in text.split('\n') {
            line_count += 1;
            let w: f64 = line.chars().map(char_adv).sum();
            max_w = max_w.max(w);
        }
        let lines = line_count.max(1) as f64;
        (max_w.ceil().max(1.0), (lines * lh).ceil().max(lh))
    } else {
        let wrap_w = fixed_width.unwrap_or(200.0).max(font_size);
        let mut total_lines = 0usize;
        for para in text.split('\n') {
            let mut x = 0.0;
            let mut lines_in_para = 1usize;
            for ch in para.chars() {
                let adv = char_adv(ch);
                if x + adv > wrap_w && x > 0.0 {
                    lines_in_para += 1;
                    x = adv;
                } else {
                    x += adv;
                }
            }
            total_lines += lines_in_para;
        }
        (wrap_w, (total_lines.max(1) as f64 * lh).ceil().max(lh))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_splice_and_style_override_compaction_handle_emoji_and_cjk() {
        // "A🦀中B": 'A' (1 u16), '🦀' (2 u16 surrogates), '中' (1 u16), 'B' (1 u16) -> total 5 u16!
        let base = Val::Obj(Obj::from([("characters", Val::Str("A🦀中B".into()))]));
        let styled = apply_text_range_style(
            &base,
            1,
            4,
            &TextStyleRunPatch {
                font_style: Some("Bold".into()),
                font_size: Some(24.0),
                ..Default::default()
            },
        )
        .expect("apply range style");

        assert_eq!(styled.get("styleOverrideTable").unwrap().items().len(), 1);
        let spliced = splice_text_utf16(&styled, 1, 2, "X").expect("replace emoji with X");
        assert_eq!(spliced.text("characters"), Some("AX中B"));
        // Style 1 is still used by 'X' and '中' -> table still has 1 entry.
        assert_eq!(spliced.get("styleOverrideTable").unwrap().items().len(), 1);

        // Delete the entire styled range ("X中") -> styleOverrideTable compacts to empty!
        let cleaned = splice_text_utf16(&spliced, 1, 2, "").expect("delete styled range");
        assert_eq!(cleaned.text("characters"), Some("AB"));
        assert!(cleaned.get("styleOverrideTable").is_none());
    }
}
