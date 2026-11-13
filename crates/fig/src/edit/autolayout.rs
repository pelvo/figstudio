//! Complete Figma Auto-Layout solver (`HORIZONTAL`, `VERTICAL`, `WRAP`, `GRID`).
//!
//! Performs bottom-up `HUG` (`RESIZE_TO_FIT` / `RESIZE_TO_FIT_WITH_IMPLICIT_SIZE`) measurement
//! with `minSize` / `maxSize` clamping, followed by top-down `FILL` (`stackChildPrimaryGrow`
//! and `stackChildAlignSelf == "STRETCH"`) space distribution, alignment (`MIN`, `CENTER`,
//! `MAX`, `SPACE_BETWEEN`, `SPACE_EVENLY`, `BASELINE`), multi-line `WRAP`, and `GRID` placement.

use std::collections::HashMap;

use crate::edit::Txn;
use crate::edit::geometry::{encode_path_commands, make_path_entry, rebuild_shape_geometry};
use crate::{Document, FigError, Guid, Obj, Val};

/// Computed geometry update for one node after solving Auto-Layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutUpdate {
    pub id: Guid,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub pos_changed: bool,
    pub size_changed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AxisMode {
    Horizontal,
    Vertical,
    Grid,
}

#[derive(Clone, Copy)]
struct Padding {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

/// Computes Auto-Layout updates across `doc` (scoped to the highest stack ancestor of
/// `scope_root` when `Some`, or across all pages when `None`).
pub fn compute_auto_layout(doc: &Document, scope_root: Option<Guid>) -> Vec<LayoutUpdate> {
    let mut sizes: HashMap<Guid, (f64, f64)> = HashMap::new();
    let mut positions: HashMap<Guid, (f64, f64)> = HashMap::new();

    for n in &doc.nodes {
        sizes.insert(n.id, read_size(&n.data));
        positions.insert(n.id, read_pos(&n.data));
    }

    let roots: Vec<Guid> = match scope_root {
        Some(start) => vec![highest_stack_ancestor(doc, start)],
        None => doc.pages.clone(),
    };

    let mut touched_order = Vec::new();
    for root in &roots {
        measure_bottom_up(doc, *root, &mut sizes);
        layout_top_down(doc, *root, &mut sizes, &mut positions, &mut touched_order);
    }

    let mut out = Vec::new();
    for id in touched_order {
        let Some(node) = doc.node(id) else { continue };
        let (old_w, old_h) = read_size(&node.data);
        let (old_x, old_y) = read_pos(&node.data);
        let (w, h) = sizes[&id];
        let (x, y) = positions[&id];
        let pos_changed = (x - old_x).abs() > 1e-3 || (y - old_y).abs() > 1e-3;
        let size_changed = (w - old_w).abs() > 1e-3 || (h - old_h).abs() > 1e-3;
        if pos_changed || size_changed {
            out.push(LayoutUpdate {
                id,
                x,
                y,
                width: w,
                height: h,
                pos_changed,
                size_changed,
            });
        }
    }
    out
}

/// Appends minimal Auto-Layout `NodePatch`es onto `txn` for `doc`.
pub fn append_auto_layout_patches(doc: &Document, scope_root: Option<Guid>, txn: &mut Txn) {
    let updates = compute_auto_layout(doc, scope_root);
    for u in updates {
        let Some(node) = doc.node(u.id) else { continue };
        let mut set = Obj::new();
        if u.pos_changed {
            let mut tf = node
                .data
                .get("transform")
                .cloned()
                .unwrap_or_else(|| crate::edit::geometry::matrix_2x3(0.0, 0.0, 0.0))
                .into_owned();
            tf.set("m02", Val::Float(u.x));
            tf.set("m12", Val::Float(u.y));
            set.insert("transform", tf);
        }
        if u.size_changed {
            let new_size = Val::Obj(Obj::from([
                ("x", Val::Float(u.width)),
                ("y", Val::Float(u.height)),
            ]));
            set.insert("size", new_size.clone());
            if node.data.get("fillGeometry").is_some() {
                let mut preview = node.data.clone().into_owned();
                preview.set("size", new_size);
                if let Some(cmds) = rebuild_shape_geometry(&node.kind, &preview) {
                    let blob_idx = doc.blobs.len() + txn.add_blobs.len();
                    txn.add_blobs.push(encode_path_commands(&cmds));
                    set.insert(
                        "fillGeometry",
                        Val::Arr(vec![make_path_entry(blob_idx, false)]),
                    );
                }
            }
        }
        txn.modify(u.id, set, Vec::new());
    }
}

/// Runs Auto-Layout on `doc` and commits the resulting patches immediately.
pub fn apply_auto_layout_to_doc(
    doc: &mut Document,
    scope_root: Option<Guid>,
) -> Result<Txn, FigError> {
    let mut txn = Txn::new("Solve Auto-Layout");
    append_auto_layout_patches(doc, scope_root, &mut txn);
    let (inv, _) = doc.apply_txn(&txn)?;
    Ok(inv)
}

fn highest_stack_ancestor(doc: &Document, start: Guid) -> Guid {
    let mut best = start;
    let mut cur = Some(start);
    while let Some(id) = cur {
        let Some(n) = doc.node(id) else { break };
        if n.kind == "CANVAS" || n.kind == "DOCUMENT" {
            break;
        }
        if stack_mode(&n.data).is_some() {
            best = id;
        }
        cur = n.parent;
    }
    best
}

fn stack_mode(data: &Val) -> Option<AxisMode> {
    match data.text("stackMode") {
        Some("HORIZONTAL") => Some(AxisMode::Horizontal),
        Some("VERTICAL") => Some(AxisMode::Vertical),
        Some("GRID") => Some(AxisMode::Grid),
        _ => None,
    }
}

fn is_hug(val: Option<&str>) -> bool {
    matches!(
        val,
        Some("RESIZE_TO_FIT" | "RESIZE_TO_FIT_WITH_IMPLICIT_SIZE")
    )
}

fn primary_is_hug(data: &Val, mode: AxisMode) -> bool {
    is_hug(data.text("stackPrimarySizing")).or_else_bool(|| match mode {
        AxisMode::Horizontal | AxisMode::Grid => is_hug(data.text("stackWidth")),
        AxisMode::Vertical => is_hug(data.text("stackHeight")),
    })
}

fn counter_is_hug(data: &Val, mode: AxisMode) -> bool {
    is_hug(data.text("stackCounterSizing")).or_else_bool(|| match mode {
        AxisMode::Horizontal | AxisMode::Grid => is_hug(data.text("stackHeight")),
        AxisMode::Vertical => is_hug(data.text("stackWidth")),
    })
}

trait BoolExt {
    fn or_else_bool(self, f: impl FnOnce() -> bool) -> bool;
}
impl BoolExt for bool {
    fn or_else_bool(self, f: impl FnOnce() -> bool) -> bool {
        if self { true } else { f() }
    }
}

fn padding_of(data: &Val) -> Padding {
    let base = f64::from(data.num("stackPadding").unwrap_or(0.0));
    let hp = data.num("stackHorizontalPadding").map_or(base, f64::from);
    let vp = data.num("stackVerticalPadding").map_or(base, f64::from);
    let mut left = hp;
    let mut top = vp;
    let mut right = data.num("stackPaddingRight").map_or(hp, f64::from);
    let mut bottom = data.num("stackPaddingBottom").map_or(vp, f64::from);

    let has_visible_stroke = data
        .get("strokePaints")
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .any(|p| p.flag("visible") != Some(false));
    if data.flag("bordersTakeSpace") == Some(true)
        && has_visible_stroke
        && data.text("strokeAlign").unwrap_or("INSIDE") == "INSIDE"
    {
        let sw = f64::from(data.num("strokeWeight").unwrap_or(0.0));
        if data.flag("borderStrokeWeightsIndependent") == Some(true) {
            left += data.num("borderLeftWeight").map_or(sw, f64::from);
            top += data.num("borderTopWeight").map_or(sw, f64::from);
            right += data.num("borderRightWeight").map_or(sw, f64::from);
            bottom += data.num("borderBottomWeight").map_or(sw, f64::from);
        } else {
            left += sw;
            top += sw;
            right += sw;
            bottom += sw;
        }
    }
    Padding {
        left,
        top,
        right,
        bottom,
    }
}

fn clamp_min_max(data: &Val, w: f64, h: f64) -> (f64, f64) {
    let min_x = data
        .path(&["minSize", "value", "x"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let min_y = data
        .path(&["minSize", "value", "y"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let max_x = data
        .path(&["maxSize", "value", "x"])
        .and_then(Val::as_f32)
        .map_or(f64::INFINITY, f64::from);
    let max_y = data
        .path(&["maxSize", "value", "y"])
        .and_then(Val::as_f32)
        .map_or(f64::INFINITY, f64::from);
    (
        w.clamp(min_x, max_x).max(0.0),
        h.clamp(min_y, max_y).max(0.0),
    )
}

fn flow_children(doc: &Document, parent: Guid) -> Vec<Guid> {
    doc.children(parent)
        .iter()
        .copied()
        .filter(|&cid| {
            doc.node(cid).is_some_and(|cn| {
                cn.data.flag("visible") != Some(false)
                    && cn.data.text("stackPositioning") != Some("ABSOLUTE")
            })
        })
        .collect()
}

fn measure_bottom_up(doc: &Document, id: Guid, sizes: &mut HashMap<Guid, (f64, f64)>) {
    for &cid in doc.children(id) {
        measure_bottom_up(doc, cid, sizes);
    }
    let Some(node) = doc.node(id) else { return };
    let Some(mode) = stack_mode(&node.data) else {
        return;
    };
    let (cur_w, cur_h) = sizes[&id];
    let hug_p = primary_is_hug(&node.data, mode);
    let hug_c = counter_is_hug(&node.data, mode);
    if !hug_p && !hug_c {
        return;
    }

    let pad = padding_of(&node.data);
    let kids = flow_children(doc, id);
    if kids.is_empty() {
        let nw = if (mode == AxisMode::Horizontal && hug_p) || (mode == AxisMode::Vertical && hug_c)
        {
            pad.left + pad.right
        } else {
            cur_w
        };
        let nh = if (mode == AxisMode::Vertical && hug_p) || (mode == AxisMode::Horizontal && hug_c)
        {
            pad.top + pad.bottom
        } else {
            cur_h
        };
        sizes.insert(id, clamp_min_max(&node.data, nw, nh));
        return;
    }

    match mode {
        AxisMode::Horizontal | AxisMode::Vertical => {
            let horiz = mode == AxisMode::Horizontal;
            let gap = f64::from(node.data.num("stackSpacing").unwrap_or(0.0));
            let wrap = node.data.text("stackWrap") == Some("WRAP") && !hug_p;
            let c_gap = f64::from(node.data.num("stackCounterSpacing").unwrap_or(gap as f32));
            let avail_p = if horiz {
                (cur_w - pad.left - pad.right).max(0.0)
            } else {
                (cur_h - pad.top - pad.bottom).max(0.0)
            };

            let lines = split_wrap_lines(&kids, sizes, horiz, gap, wrap, avail_p);
            let mut max_line_p = 0.0_f64;
            let mut total_c = 0.0_f64;
            for (li, line) in lines.iter().enumerate() {
                if li > 0 {
                    total_c += c_gap;
                }
                let mut lp = 0.0;
                let mut lc = 0.0_f64;
                for (i, &cid) in line.iter().enumerate() {
                    if i > 0 {
                        lp += gap;
                    }
                    let (cw, ch) = sizes[&cid];
                    let (cp, cc) = if horiz { (cw, ch) } else { (ch, cw) };
                    lp += cp;
                    lc = lc.max(cc);
                }
                max_line_p = max_line_p.max(lp);
                total_c += lc;
            }

            let mut nw = cur_w;
            let mut nh = cur_h;
            if horiz {
                if hug_p {
                    nw = pad.left + max_line_p + pad.right;
                }
                if hug_c {
                    nh = pad.top + total_c + pad.bottom;
                }
            } else {
                if hug_p {
                    nh = pad.top + max_line_p + pad.bottom;
                }
                if hug_c {
                    nw = pad.left + total_c + pad.right;
                }
            }
            sizes.insert(id, clamp_min_max(&node.data, nw, nh));
        }
        AxisMode::Grid => {
            let cols = grid_col_count(&node.data, kids.len());
            let c_gap = f64::from(node.data.num("gridColumnGap").unwrap_or(0.0));
            let r_gap = f64::from(node.data.num("gridRowGap").unwrap_or(0.0));
            let rows = kids.len().div_ceil(cols);
            let mut col_w = vec![0.0_f64; cols];
            let mut row_h = vec![0.0_f64; rows];
            for (idx, &cid) in kids.iter().enumerate() {
                let (cw, ch) = sizes[&cid];
                col_w[idx % cols] = col_w[idx % cols].max(cw);
                row_h[idx / cols] = row_h[idx / cols].max(ch);
            }
            let total_w = pad.left
                + col_w.iter().sum::<f64>()
                + c_gap * cols.saturating_sub(1) as f64
                + pad.right;
            let total_h = pad.top
                + row_h.iter().sum::<f64>()
                + r_gap * rows.saturating_sub(1) as f64
                + pad.bottom;
            let nw = if hug_p { total_w } else { cur_w };
            let nh = if hug_c { total_h } else { cur_h };
            sizes.insert(id, clamp_min_max(&node.data, nw, nh));
        }
    }
}

fn layout_top_down(
    doc: &Document,
    id: Guid,
    sizes: &mut HashMap<Guid, (f64, f64)>,
    positions: &mut HashMap<Guid, (f64, f64)>,
    touched: &mut Vec<Guid>,
) {
    let Some(node) = doc.node(id) else { return };
    touched.push(id);

    if let Some(mode) = stack_mode(&node.data) {
        let (cont_w, cont_h) = sizes[&id];
        let pad = padding_of(&node.data);
        let kids = flow_children(doc, id);

        match mode {
            AxisMode::Horizontal | AxisMode::Vertical => {
                let horiz = mode == AxisMode::Horizontal;
                let gap = f64::from(node.data.num("stackSpacing").unwrap_or(0.0));
                let wrap = node.data.text("stackWrap") == Some("WRAP");
                let c_gap = f64::from(node.data.num("stackCounterSpacing").unwrap_or(gap as f32));
                let inner_p = if horiz {
                    (cont_w - pad.left - pad.right).max(0.0)
                } else {
                    (cont_h - pad.top - pad.bottom).max(0.0)
                };
                let inner_c = if horiz {
                    (cont_h - pad.top - pad.bottom).max(0.0)
                } else {
                    (cont_w - pad.left - pad.right).max(0.0)
                };

                let lines = split_wrap_lines(&kids, sizes, horiz, gap, wrap, inner_p);
                let justify = node
                    .data
                    .text("stackPrimaryAlignItems")
                    .or_else(|| node.data.text("stackJustify"))
                    .unwrap_or("MIN");
                let counter_align = node
                    .data
                    .text("stackCounterAlignItems")
                    .or_else(|| node.data.text("stackAlign"))
                    .unwrap_or("MIN");

                // Compute each line's counter size after resolving primary FILL sizes.
                let mut line_counter_sizes = Vec::with_capacity(lines.len());
                for line in &lines {
                    distribute_primary_fill(doc, line, sizes, horiz, inner_p, gap);
                    let mut lc = if lines.len() == 1 && !counter_is_hug(&node.data, mode) {
                        inner_c
                    } else {
                        0.0
                    };
                    for &cid in line {
                        let (cw, ch) = sizes[&cid];
                        lc = lc.max(if horiz { ch } else { cw });
                    }
                    line_counter_sizes.push(lc);
                }

                let total_lines_c: f64 = line_counter_sizes.iter().sum::<f64>()
                    + c_gap * lines.len().saturating_sub(1) as f64;
                let mut cur_c = if horiz { pad.top } else { pad.left };
                if lines.len() > 1 {
                    match counter_align {
                        "CENTER" => cur_c += (inner_c - total_lines_c) * 0.5,
                        "MAX" => cur_c += inner_c - total_lines_c,
                        _ => {}
                    }
                }

                for (li, line) in lines.iter().enumerate() {
                    let line_c = line_counter_sizes[li];
                    // Stretch children with `stackChildAlignSelf == "STRETCH"`.
                    for &cid in line {
                        if let Some(cn) = doc.node(cid)
                            && cn.data.text("stackChildAlignSelf") == Some("STRETCH")
                        {
                            let (cw, ch) = sizes[&cid];
                            let stretched = if horiz {
                                clamp_min_max(&cn.data, cw, line_c)
                            } else {
                                clamp_min_max(&cn.data, line_c, ch)
                            };
                            sizes.insert(cid, stretched);
                        }
                    }

                    let sum_items_p: f64 = line
                        .iter()
                        .map(|cid| {
                            let (cw, ch) = sizes[cid];
                            if horiz { cw } else { ch }
                        })
                        .sum();
                    let base_gaps = gap * line.len().saturating_sub(1) as f64;
                    let leftover_p = (inner_p - sum_items_p - base_gaps).max(0.0);

                    let (mut cur_p, eff_gap) = match justify {
                        "CENTER" => (leftover_p * 0.5, gap),
                        "MAX" => (leftover_p, gap),
                        "SPACE_BETWEEN" if line.len() > 1 => (
                            0.0,
                            (inner_p - sum_items_p).max(0.0) / (line.len() - 1) as f64,
                        ),
                        "SPACE_EVENLY" => {
                            let step = (inner_p - sum_items_p).max(0.0) / (line.len() + 1) as f64;
                            (step, step)
                        }
                        _ => (0.0, gap),
                    };
                    cur_p += if horiz { pad.left } else { pad.top };

                    for &cid in line {
                        let (cw, ch) = sizes[&cid];
                        let (cp, cc) = if horiz { (cw, ch) } else { (ch, cw) };
                        let self_align = doc
                            .node(cid)
                            .and_then(|cn| cn.data.text("stackChildAlignSelf"))
                            .filter(|&a| a != "AUTO")
                            .unwrap_or(counter_align);
                        let offset_c = match self_align {
                            "CENTER" => (line_c - cc) * 0.5,
                            "MAX" | "BASELINE" => line_c - cc,
                            _ => 0.0,
                        };
                        let pos = if horiz {
                            (cur_p, cur_c + offset_c)
                        } else {
                            (cur_c + offset_c, cur_p)
                        };
                        positions.insert(cid, pos);
                        cur_p += cp + eff_gap;
                    }
                    cur_c += line_c + c_gap;
                }
            }
            AxisMode::Grid => {
                let cols = grid_col_count(&node.data, kids.len());
                let rows = kids.len().div_ceil(cols).max(1);
                let c_gap = f64::from(node.data.num("gridColumnGap").unwrap_or(0.0));
                let r_gap = f64::from(node.data.num("gridRowGap").unwrap_or(0.0));
                let mut col_w = vec![0.0_f64; cols];
                let mut row_h = vec![0.0_f64; rows];
                for (idx, &cid) in kids.iter().enumerate() {
                    let (cw, ch) = sizes[&cid];
                    col_w[idx % cols] = col_w[idx % cols].max(cw);
                    row_h[idx / cols] = row_h[idx / cols].max(ch);
                }
                let mut col_x = vec![pad.left; cols];
                for c in 1..cols {
                    col_x[c] = col_x[c - 1] + col_w[c - 1] + c_gap;
                }
                let mut row_y = vec![pad.top; rows];
                for r in 1..rows {
                    row_y[r] = row_y[r - 1] + row_h[r - 1] + r_gap;
                }
                for (idx, &cid) in kids.iter().enumerate() {
                    positions.insert(cid, (col_x[idx % cols], row_y[idx / cols]));
                }
            }
        }
    }

    for &cid in doc.children(id) {
        layout_top_down(doc, cid, sizes, positions, touched);
    }
}

fn split_wrap_lines(
    kids: &[Guid],
    sizes: &HashMap<Guid, (f64, f64)>,
    horiz: bool,
    gap: f64,
    wrap: bool,
    avail_p: f64,
) -> Vec<Vec<Guid>> {
    if !wrap || kids.is_empty() || avail_p <= 0.0 {
        return vec![kids.to_vec()];
    }
    let mut lines: Vec<Vec<Guid>> = Vec::new();
    let mut cur: Vec<Guid> = Vec::new();
    let mut cur_len = 0.0;
    for &cid in kids {
        let (cw, ch) = sizes[&cid];
        let cp = if horiz { cw } else { ch };
        let next_len = if cur.is_empty() {
            cp
        } else {
            cur_len + gap + cp
        };
        if !cur.is_empty() && next_len > avail_p + 1e-3 {
            lines.push(std::mem::take(&mut cur));
            cur.push(cid);
            cur_len = cp;
        } else {
            cur.push(cid);
            cur_len = next_len;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn distribute_primary_fill(
    doc: &Document,
    line: &[Guid],
    sizes: &mut HashMap<Guid, (f64, f64)>,
    horiz: bool,
    inner_p: f64,
    gap: f64,
) {
    let mut grow_ids: Vec<(Guid, f64)> = Vec::new();
    let mut fixed_sum = gap * line.len().saturating_sub(1) as f64;
    for &cid in line {
        let grow = doc
            .node(cid)
            .and_then(|n| n.data.num("stackChildPrimaryGrow"))
            .map_or(0.0, f64::from);
        if grow > 0.0 {
            grow_ids.push((cid, grow));
        } else {
            let (cw, ch) = sizes[&cid];
            fixed_sum += if horiz { cw } else { ch };
        }
    }
    if grow_ids.is_empty() {
        return;
    }

    // Iterative water-filling distribution respecting minSize / maxSize bounds!
    let mut remaining = (inner_p - fixed_sum).max(0.0);
    let mut active: Vec<(Guid, f64)> = grow_ids;
    loop {
        let total_grow: f64 = active.iter().map(|(_, g)| *g).sum();
        if total_grow <= 0.0 || active.is_empty() {
            break;
        }
        let mut clamped_any = false;
        let mut next_active = Vec::with_capacity(active.len());
        for &(cid, g) in &active {
            let share = remaining * (g / total_grow);
            let (cw, ch) = sizes[&cid];
            let cn = doc.node(cid).unwrap();
            let (clamped_w, clamped_h) = if horiz {
                clamp_min_max(&cn.data, share, ch)
            } else {
                clamp_min_max(&cn.data, cw, share)
            };
            let clamped_p = if horiz { clamped_w } else { clamped_h };
            if (clamped_p - share).abs() > 1e-4 {
                sizes.insert(cid, (clamped_w, clamped_h));
                remaining = (remaining - clamped_p).max(0.0);
                clamped_any = true;
            } else {
                next_active.push((cid, g));
            }
        }
        if !clamped_any {
            for (cid, g) in next_active {
                let share = remaining * (g / total_grow);
                let (cw, ch) = sizes[&cid];
                sizes.insert(cid, if horiz { (share, ch) } else { (cw, share) });
            }
            break;
        }
        active = next_active;
    }
}

fn grid_col_count(data: &Val, n_items: usize) -> usize {
    if let Some(entries) = data.path(&["gridColumns", "entries"])
        && !entries.items().is_empty()
    {
        return entries.items().len().max(1);
    }
    (n_items as f64).sqrt().ceil().max(1.0) as usize
}

fn read_size(data: &Val) -> (f64, f64) {
    let x = data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let y = data
        .path(&["size", "y"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    (x.max(0.0), y.max(0.0))
}

fn read_pos(data: &Val) -> (f64, f64) {
    let x = data
        .path(&["transform", "m02"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let y = data
        .path(&["transform", "m12"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Node;
    use crate::edit::geometry::matrix_2x3;
    use crate::intern;

    fn make_node(
        id: Guid,
        parent: Option<Guid>,
        pos: &str,
        kind: &str,
        w: f64,
        h: f64,
        extra: Obj,
    ) -> Node {
        let mut o = extra;
        o.insert("guid", id.to_val());
        o.insert("type", Val::Enum(intern(kind)));
        o.insert("visible", Val::Bool(true));
        o.insert("transform", matrix_2x3(0.0, 0.0, 0.0));
        o.insert(
            "size",
            Val::Obj(Obj::from([("x", Val::Float(w)), ("y", Val::Float(h))])),
        );
        if let Some(p) = parent {
            o.insert(
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", p.to_val()),
                    ("position", Val::Str(pos.into())),
                ])),
            );
        }
        Node::from_data(Val::Obj(o)).unwrap()
    }

    #[test]
    fn horizontal_fill_with_min_max_clamping_and_vertical_hug() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/stacks_wrap.fig");
        let mut doc = Document::open(path).unwrap();
        let page = doc.pages[0];

        let stack_id = Guid(999, 1);
        let c1 = Guid(999, 2);
        let c2 = Guid(999, 3);
        let stack = make_node(
            stack_id,
            Some(page),
            "~1",
            "FRAME",
            320.0,
            10.0,
            Obj::from([
                ("stackMode", Val::Enum(intern("HORIZONTAL"))),
                ("stackPrimarySizing", Val::Enum(intern("FIXED"))),
                ("stackCounterSizing", Val::Enum(intern("RESIZE_TO_FIT"))),
                ("stackHorizontalPadding", Val::Float(10.0)),
                ("stackVerticalPadding", Val::Float(12.0)),
                ("stackSpacing", Val::Float(20.0)),
            ]),
        );
        // c1 has FILL + maxSize.x = 80; c2 has FILL uncapped -> remaining (300 - 20 = 280) => c1=80, c2=200!
        let n1 = make_node(
            c1,
            Some(stack_id),
            "a",
            "ROUNDED_RECTANGLE",
            40.0,
            44.0,
            Obj::from([
                ("stackChildPrimaryGrow", Val::Float(1.0)),
                (
                    "maxSize",
                    Val::Obj(Obj::from([(
                        "value",
                        Val::Obj(Obj::from([
                            ("x", Val::Float(80.0)),
                            ("y", Val::Float(1000.0)),
                        ])),
                    )])),
                ),
            ]),
        );
        let n2 = make_node(
            c2,
            Some(stack_id),
            "b",
            "ROUNDED_RECTANGLE",
            40.0,
            30.0,
            Obj::from([
                ("stackChildPrimaryGrow", Val::Float(1.0)),
                ("stackChildAlignSelf", Val::Enum(intern("STRETCH"))),
            ]),
        );
        let mut nodes = doc.nodes.clone();
        nodes.extend([stack, n1, n2]);
        doc.set_nodes(nodes).unwrap();

        apply_auto_layout_to_doc(&mut doc, Some(stack_id)).unwrap();
        assert_eq!(read_size(&doc.node(stack_id).unwrap().data), (320.0, 68.0));
        assert_eq!(read_size(&doc.node(c1).unwrap().data), (80.0, 44.0));
        assert_eq!(read_pos(&doc.node(c1).unwrap().data), (10.0, 12.0));
        assert_eq!(read_size(&doc.node(c2).unwrap().data), (200.0, 44.0));
        assert_eq!(read_pos(&doc.node(c2).unwrap().data), (110.0, 12.0));
    }
}
