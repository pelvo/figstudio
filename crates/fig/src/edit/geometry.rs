//! Parametric shapes, `commandsBlob` outline generation, squircle corner smoothing, and
//! Figma resize constraints (`MIN`, `CENTER`, `MAX`, `STRETCH`, `SCALE`).

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use serde::{Deserialize, Serialize};

use crate::edit::{Txn, fractional};
use crate::{Document, FigError, Guid, Obj, Val, intern};

/// One command in a Figma `commandsBlob` path stream (`0..=4` + little-endian `f32` coords).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    Close,
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
}

/// Encodes `cmds` into Figma's binary `commandsBlob` representation.
pub fn encode_path_commands(cmds: &[PathCmd]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cmds.len() * 12);
    let push = |buf: &mut Vec<u8>, v: f32| buf.extend_from_slice(&v.to_le_bytes());
    for &cmd in cmds {
        match cmd {
            PathCmd::Close => out.push(0),
            PathCmd::MoveTo(x, y) => {
                out.push(1);
                push(&mut out, x);
                push(&mut out, y);
            }
            PathCmd::LineTo(x, y) => {
                out.push(2);
                push(&mut out, x);
                push(&mut out, y);
            }
            PathCmd::QuadTo(cx, cy, x, y) => {
                out.push(3);
                for v in [cx, cy, x, y] {
                    push(&mut out, v);
                }
            }
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                out.push(4);
                for v in [c1x, c1y, c2x, c2y, x, y] {
                    push(&mut out, v);
                }
            }
        }
    }
    out
}

/// Decodes a Figma binary `commandsBlob` into [`PathCmd`]s.
pub fn decode_path_commands(bytes: &[u8]) -> Option<Vec<PathCmd>> {
    let read = |at: usize| -> Option<f32> {
        let b: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(f32::from_le_bytes(b))
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let tag = bytes[i];
        i += 1;
        match tag {
            0 => out.push(PathCmd::Close),
            1 => {
                out.push(PathCmd::MoveTo(read(i)?, read(i + 4)?));
                i += 8;
            }
            2 => {
                out.push(PathCmd::LineTo(read(i)?, read(i + 4)?));
                i += 8;
            }
            3 => {
                out.push(PathCmd::QuadTo(
                    read(i)?,
                    read(i + 4)?,
                    read(i + 8)?,
                    read(i + 12)?,
                ));
                i += 16;
            }
            4 => {
                out.push(PathCmd::CubicTo(
                    read(i)?,
                    read(i + 4)?,
                    read(i + 8)?,
                    read(i + 12)?,
                    read(i + 16)?,
                    read(i + 20)?,
                ));
                i += 24;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Builds a Kiwi `Path` struct entry (`{ windingRule, commandsBlob, styleID }`).
pub fn make_path_entry(blob_index: usize, even_odd: bool) -> Val {
    Val::Obj(Obj::from([
        (
            "windingRule",
            Val::Enum(intern(if even_odd { "ODD" } else { "NONZERO" })),
        ),
        ("commandsBlob", Val::Int(blob_index as i64)),
        ("styleID", Val::Int(0)),
    ]))
}

/// Builds a 2x3 Figma `Matrix` (`m00..m12`) for translation `(x, y)` and rotation `deg`.
pub fn matrix_2x3(x: f64, y: f64, rotation_deg: f64) -> Val {
    let rad = rotation_deg.to_radians();
    let (s, c) = rad.sin_cos();
    Val::Obj(Obj::from([
        ("m00", Val::Float(c)),
        ("m01", Val::Float(-s)),
        ("m02", Val::Float(x)),
        ("m10", Val::Float(s)),
        ("m11", Val::Float(c)),
        ("m12", Val::Float(y)),
    ]))
}

/// Generates rounded-rectangle outline commands with per-corner radii `[tl, tr, br, bl]`
/// and optional squircle `smoothing` (`0.0..=1.0`).
pub fn rounded_rect_commands(w: f32, h: f32, radii: [f32; 4], smoothing: f32) -> Vec<PathCmd> {
    let w = if w.is_finite() { w.max(0.0) } else { 0.0 };
    let h = if h.is_finite() { h.max(0.0) } else { 0.0 };
    let max_r = (w.min(h) * 0.5).max(0.0);
    let [tl, tr, br, bl] = radii.map(|r| {
        if r.is_finite() {
            r.clamp(0.0, max_r)
        } else {
            0.0
        }
    });
    if tl <= 1e-4 && tr <= 1e-4 && br <= 1e-4 && bl <= 1e-4 {
        return vec![
            PathCmd::MoveTo(0.0, 0.0),
            PathCmd::LineTo(w, 0.0),
            PathCmd::LineTo(w, h),
            PathCmd::LineTo(0.0, h),
            PathCmd::Close,
        ];
    }
    // Kappa for circular cubic bezier (~0.55228475); smoothing pulls handles slightly further
    // along the straight tangent for iOS-style continuous curvature.
    let smooth = if smoothing.is_finite() {
        smoothing.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let k = 0.552_284_8 + smooth * 0.12;
    let mut cmds = Vec::with_capacity(10);
    cmds.push(PathCmd::MoveTo(tl, 0.0));
    cmds.push(PathCmd::LineTo(w - tr, 0.0));
    if tr > 0.0 {
        cmds.push(PathCmd::CubicTo(
            w - tr * (1.0 - k),
            0.0,
            w,
            tr * (1.0 - k),
            w,
            tr,
        ));
    }
    cmds.push(PathCmd::LineTo(w, h - br));
    if br > 0.0 {
        cmds.push(PathCmd::CubicTo(
            w,
            h - br * (1.0 - k),
            w - br * (1.0 - k),
            h,
            w - br,
            h,
        ));
    }
    cmds.push(PathCmd::LineTo(bl, h));
    if bl > 0.0 {
        cmds.push(PathCmd::CubicTo(
            bl * (1.0 - k),
            h,
            0.0,
            h - bl * (1.0 - k),
            0.0,
            h - bl,
        ));
    }
    cmds.push(PathCmd::LineTo(0.0, tl));
    if tl > 0.0 {
        cmds.push(PathCmd::CubicTo(
            0.0,
            tl * (1.0 - k),
            tl * (1.0 - k),
            0.0,
            tl,
            0.0,
        ));
    }
    cmds.push(PathCmd::Close);
    cmds
}

/// Generates ellipse / arc / donut outline commands matching Figma's `arcData`.
pub fn ellipse_commands(
    w: f32,
    h: f32,
    start_angle: f32,
    end_angle: f32,
    inner_radius: f32,
) -> Vec<PathCmd> {
    let w = if w.is_finite() { w.max(0.0) } else { 0.0 };
    let h = if h.is_finite() { h.max(0.0) } else { 0.0 };
    let (rx, ry) = (w * 0.5, h * 0.5);
    let (cx, cy) = (rx, ry);
    let angles_valid = start_angle.is_finite() && end_angle.is_finite();
    let raw_delta = if angles_valid {
        (end_angle - start_angle).clamp(-TAU, TAU)
    } else {
        TAU
    };
    let sweep = raw_delta.abs();
    let full = !angles_valid || sweep >= TAU - 1e-3 || sweep <= 1e-5;
    let inner = if inner_radius.is_finite() {
        inner_radius.clamp(0.0, 0.999)
    } else {
        0.0
    };

    if full && inner <= 1e-4 {
        let k = 0.552_284_8;
        return vec![
            PathCmd::MoveTo(cx + rx, cy),
            PathCmd::CubicTo(cx + rx, cy + ry * k, cx + rx * k, cy + ry, cx, cy + ry),
            PathCmd::CubicTo(cx - rx * k, cy + ry, cx - rx, cy + ry * k, cx - rx, cy),
            PathCmd::CubicTo(cx - rx, cy - ry * k, cx - rx * k, cy - ry, cx, cy - ry),
            PathCmd::CubicTo(cx + rx * k, cy - ry, cx + rx, cy - ry * k, cx + rx, cy),
            PathCmd::Close,
        ];
    }

    let a0 = if full {
        0.0
    } else {
        start_angle.rem_euclid(TAU)
    };
    let a1 = if full { TAU } else { a0 + raw_delta };
    let mut cmds = Vec::new();
    append_arc_beziers(&mut cmds, cx, cy, rx, ry, a0, a1, true);
    if full {
        cmds.push(PathCmd::Close);
        if inner > 1e-4 {
            append_arc_beziers(&mut cmds, cx, cy, rx * inner, ry * inner, TAU, 0.0, true);
            cmds.push(PathCmd::Close);
        }
    } else if inner > 1e-4 {
        append_arc_beziers(&mut cmds, cx, cy, rx * inner, ry * inner, a1, a0, false);
        cmds.push(PathCmd::Close);
    } else {
        cmds.push(PathCmd::LineTo(cx, cy));
        cmds.push(PathCmd::Close);
    }
    cmds
}

#[expect(clippy::too_many_arguments)]
fn append_arc_beziers(
    out: &mut Vec<PathCmd>,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    a0: f32,
    a1: f32,
    move_first: bool,
) {
    let total = if (a1 - a0).is_finite() {
        (a1 - a0).clamp(-TAU, TAU)
    } else {
        TAU
    };
    let segs = ((total.abs() / FRAC_PI_2).ceil() as usize).clamp(1, 8);
    let step = total / segs as f32;
    let k = (4.0 / 3.0) * (step * 0.25).tan();
    let (s0, c0) = a0.sin_cos();
    let start_pt = (cx + rx * c0, cy + ry * s0);
    if move_first {
        out.push(PathCmd::MoveTo(start_pt.0, start_pt.1));
    } else {
        out.push(PathCmd::LineTo(start_pt.0, start_pt.1));
    }
    for i in 0..segs {
        let t0 = a0 + step * i as f32;
        let t1 = t0 + step;
        let (sin0, cos0) = t0.sin_cos();
        let (sin1, cos1) = t1.sin_cos();
        let p0 = (cx + rx * cos0, cy + ry * sin0);
        let p3 = (cx + rx * cos1, cy + ry * sin1);
        let c1 = (p0.0 - k * rx * sin0, p0.1 + k * ry * cos0);
        let c2 = (p3.0 + k * rx * sin1, p3.1 - k * ry * cos1);
        out.push(PathCmd::CubicTo(c1.0, c1.1, c2.0, c2.1, p3.0, p3.1));
    }
}

/// Generates regular polygon (`count >= 3`) or star (`inner_scale < 1.0`) outline commands.
/// Vertex `count` is capped at `512` to bound path allocation on malformed inputs.
pub fn polygon_or_star_commands(
    w: f32,
    h: f32,
    count: u32,
    inner_scale: Option<f32>,
) -> Vec<PathCmd> {
    let w = if w.is_finite() { w.max(0.0) } else { 0.0 };
    let h = if h.is_finite() { h.max(0.0) } else { 0.0 };
    let n = count.clamp(3, 512) as usize;
    let (cx, cy) = (w * 0.5, h * 0.5);
    let (rx, ry) = (w * 0.5, h * 0.5);
    let mut cmds = Vec::new();
    if let Some(inner) = inner_scale {
        let ir = if inner.is_finite() {
            inner.clamp(0.01, 1.0)
        } else {
            0.5
        };
        let total = n * 2;
        for i in 0..total {
            let angle = -FRAC_PI_2 + (i as f32 * PI / n as f32);
            let (s, c) = angle.sin_cos();
            let scale = if i % 2 == 0 { 1.0 } else { ir };
            let (px, py) = (cx + rx * scale * c, cy + ry * scale * s);
            if i == 0 {
                cmds.push(PathCmd::MoveTo(px, py));
            } else {
                cmds.push(PathCmd::LineTo(px, py));
            }
        }
    } else {
        for i in 0..n {
            let angle = -FRAC_PI_2 + (i as f32 * TAU / n as f32);
            let (s, c) = angle.sin_cos();
            let (px, py) = (cx + rx * c, cy + ry * s);
            if i == 0 {
                cmds.push(PathCmd::MoveTo(px, py));
            } else {
                cmds.push(PathCmd::LineTo(px, py));
            }
        }
    }
    cmds.push(PathCmd::Close);
    cmds
}

/// Recomputes parametric `fillGeometry` commands for `kind` + `data`, or returns `None` for
/// non-parametric kinds (`VECTOR`, `TEXT`, `INSTANCE`, etc.).
pub fn rebuild_shape_geometry(kind: &str, data: &Val) -> Option<Vec<PathCmd>> {
    let w = data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .unwrap_or(0.0);
    let h = data
        .path(&["size", "y"])
        .and_then(Val::as_f32)
        .unwrap_or(0.0);
    let radii_of = |d: &Val| -> [f32; 4] {
        let base = d.num("cornerRadius").unwrap_or(0.0);
        if d.flag("rectangleCornerRadiiIndependent") == Some(true) {
            [
                d.num("rectangleTopLeftCornerRadius").unwrap_or(base),
                d.num("rectangleTopRightCornerRadius").unwrap_or(base),
                d.num("rectangleBottomRightCornerRadius").unwrap_or(base),
                d.num("rectangleBottomLeftCornerRadius").unwrap_or(base),
            ]
        } else {
            [base; 4]
        }
    };
    let smooth = data.num("cornerSmoothing").unwrap_or(0.0);
    match kind {
        "ROUNDED_RECTANGLE" | "RECTANGLE" | "FRAME" | "SECTION" | "SYMBOL" => {
            Some(rounded_rect_commands(w, h, radii_of(data), smooth))
        }
        "ELLIPSE" => {
            let arc = data.get("arcData");
            let a0 = arc.and_then(|a| a.num("startingAngle")).unwrap_or(0.0);
            let a1 = arc.and_then(|a| a.num("endingAngle")).unwrap_or(TAU);
            let inner = arc.and_then(|a| a.num("innerRadius")).unwrap_or(0.0);
            Some(ellipse_commands(w, h, a0, a1, inner))
        }
        "LINE" => Some(vec![PathCmd::MoveTo(0.0, 0.0), PathCmd::LineTo(w, 0.0)]),
        "REGULAR_POLYGON" => {
            let count = data.get("count").and_then(Val::as_u32).unwrap_or(3);
            Some(polygon_or_star_commands(w, h, count, None))
        }
        "STAR" => {
            let count = data.get("count").and_then(Val::as_u32).unwrap_or(5);
            let inner = data.num("starInnerScale").unwrap_or(0.381_966);
            Some(polygon_or_star_commands(w, h, count, Some(inner)))
        }
        _ => None,
    }
}

/// Parameters for creating a new layer in a [`Document`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateShapeParams {
    pub kind: String,
    pub parent: Guid,
    pub name: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub corner_radius: f32,
    #[serde(default)]
    pub fill_hex: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub star_inner_scale: Option<f32>,
}

/// Appends a newly constructed shape/container node (and its `fillGeometry` blob) to `txn`.
pub fn create_shape_node(
    doc: &Document,
    txn: &mut Txn,
    new_id: Guid,
    params: &CreateShapeParams,
) -> Result<Guid, FigError> {
    if doc.node(params.parent).is_none() {
        return Err(FigError::malformed(format!(
            "parent {} does not exist",
            params.parent
        )));
    }
    let normalized_kind = match params.kind.as_str() {
        "RECTANGLE" => "ROUNDED_RECTANGLE",
        other => other,
    };
    let last_pos = doc
        .children(params.parent)
        .last()
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    let pos = fractional::between(last_pos, None);

    let mut obj = Obj::new();
    obj.insert("guid", new_id.to_val());
    obj.insert("phase", Val::Enum(intern("CREATED")));
    obj.insert("type", Val::Enum(intern(normalized_kind)));
    obj.insert(
        "name",
        Val::Str(
            params
                .name
                .clone()
                .unwrap_or_else(|| default_layer_name(normalized_kind)),
        ),
    );
    obj.insert("visible", Val::Bool(true));
    obj.insert("opacity", Val::Float(1.0));
    obj.insert("transform", matrix_2x3(params.x, params.y, 0.0));
    obj.insert(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(params.width.max(0.0))),
            ("y", Val::Float(params.height.max(0.0))),
        ])),
    );
    obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", params.parent.to_val()),
            ("position", Val::Str(pos)),
        ])),
    );
    obj.insert("horizontalConstraint", Val::Enum(intern("MIN")));
    obj.insert("verticalConstraint", Val::Enum(intern("MIN")));
    obj.insert("strokeWeight", Val::Float(1.0));
    obj.insert("strokeAlign", Val::Enum(intern("INSIDE")));
    obj.insert("strokeJoin", Val::Enum(intern("MITER")));

    if params.corner_radius > 0.0 {
        obj.insert("cornerRadius", Val::Float(f64::from(params.corner_radius)));
    }
    if normalized_kind == "FRAME" {
        obj.insert("frameMaskDisabled", Val::Bool(false));
    }
    if let Some(c) = params.count {
        obj.insert("count", Val::Int(i64::from(c.clamp(3, 512))));
    }
    if let Some(ir) = params.star_inner_scale {
        let clamped_ir = if ir.is_finite() {
            ir.clamp(0.01, 1.0)
        } else {
            0.5
        };
        obj.insert("starInnerScale", Val::Float(f64::from(clamped_ir)));
    }

    let default_hex = match normalized_kind {
        "FRAME" => Some("#FFFFFF"),
        "SECTION" => Some("#F5F5F5"),
        "GROUP" | "LINE" => None,
        _ => Some("#D9D9D9"),
    };
    if let Some(hex_str) = params.fill_hex.as_deref().or(default_hex) {
        let paint = crate::edit::styles_vars::hex_paint(hex_str)?;
        if normalized_kind == "LINE" {
            obj.insert("strokePaints", Val::Arr(vec![paint]));
            obj.insert("strokeAlign", Val::Enum(intern("CENTER")));
        } else {
            obj.insert("fillPaints", Val::Arr(vec![paint]));
        }
    }

    let mut data = Val::Obj(obj);
    if normalized_kind != "LINE"
        && normalized_kind != "GROUP"
        && let Some(cmds) = rebuild_shape_geometry(normalized_kind, &data)
    {
        let blob_idx = doc.blobs.len() + txn.add_blobs.len();
        txn.add_blobs.push(encode_path_commands(&cmds));
        data.set(
            "fillGeometry",
            Val::Arr(vec![make_path_entry(blob_idx, false)]),
        );
    }

    txn.create_node(new_id, data);
    Ok(new_id)
}

/// Resizes `node_id` to `(new_w, new_h)` and propagates Figma constraints (`MIN`, `CENTER`,
/// `MAX`, `STRETCH`, `SCALE`) to non-auto-layout (or `ABSOLUTE`) children.
pub fn resize_node_with_constraints(
    doc: &Document,
    txn: &mut Txn,
    node_id: Guid,
    new_w: f64,
    new_h: f64,
) -> Result<(), FigError> {
    let Some(node) = doc.node(node_id) else {
        return Err(FigError::malformed(format!("missing node {node_id}")));
    };
    let old_w = node
        .data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let old_h = node
        .data
        .path(&["size", "y"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let new_w = new_w.max(0.0);
    let new_h = new_h.max(0.0);

    let mut set = Obj::new();
    let new_size = Val::Obj(Obj::from([
        ("x", Val::Float(new_w)),
        ("y", Val::Float(new_h)),
    ]));
    set.insert("size", new_size.clone());

    // Rebuild fillGeometry if this is a parametric shape that stores one.
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
    txn.modify(node_id, set, Vec::new());

    let is_stack = matches!(
        node.data.text("stackMode"),
        Some("HORIZONTAL" | "VERTICAL" | "GRID")
    );
    for &child_id in doc.children(node_id) {
        let Some(child) = doc.node(child_id) else {
            continue;
        };
        if is_stack && child.data.text("stackPositioning") != Some("ABSOLUTE") {
            continue;
        }
        let cx = child
            .data
            .path(&["transform", "m02"])
            .and_then(Val::as_f32)
            .map_or(0.0, f64::from);
        let cy = child
            .data
            .path(&["transform", "m12"])
            .and_then(Val::as_f32)
            .map_or(0.0, f64::from);
        let cw = child
            .data
            .path(&["size", "x"])
            .and_then(Val::as_f32)
            .map_or(0.0, f64::from);
        let ch = child
            .data
            .path(&["size", "y"])
            .and_then(Val::as_f32)
            .map_or(0.0, f64::from);

        let hc = child.data.text("horizontalConstraint").unwrap_or("MIN");
        let vc = child.data.text("verticalConstraint").unwrap_or("MIN");
        let (nx, nw) = solve_1d_constraint(hc, old_w, new_w, cx, cw);
        let (ny, nh) = solve_1d_constraint(vc, old_h, new_h, cy, ch);

        let pos_changed = (nx - cx).abs() > 1e-4 || (ny - cy).abs() > 1e-4;
        let size_changed = (nw - cw).abs() > 1e-4 || (nh - ch).abs() > 1e-4;
        if pos_changed {
            let mut tf = child
                .data
                .get("transform")
                .cloned()
                .unwrap_or_else(|| matrix_2x3(0.0, 0.0, 0.0))
                .into_owned();
            tf.set("m02", Val::Float(nx));
            tf.set("m12", Val::Float(ny));
            txn.set_field(child_id, "transform", tf);
        }
        if size_changed {
            resize_node_with_constraints(doc, txn, child_id, nw, nh)?;
        }
    }
    Ok(())
}

fn solve_1d_constraint(
    mode: &str,
    old_parent: f64,
    new_parent: f64,
    pos: f64,
    len: f64,
) -> (f64, f64) {
    let delta = new_parent - old_parent;
    match mode {
        "MAX" | "FIXED_MAX" => (pos + delta, len),
        "CENTER" => (pos + delta * 0.5, len),
        "STRETCH" => {
            let right_margin = old_parent - (pos + len);
            let new_len = (new_parent - pos - right_margin).max(0.0);
            (pos, new_len)
        }
        "SCALE" if old_parent > 1e-6 => {
            let ratio = new_parent / old_parent;
            (pos * ratio, (len * ratio).max(0.0))
        }
        _ => (pos, len),
    }
}

fn default_layer_name(kind: &str) -> String {
    match kind {
        "FRAME" => "Frame".into(),
        "SECTION" => "Section".into(),
        "GROUP" => "Group".into(),
        "ROUNDED_RECTANGLE" | "RECTANGLE" => "Rectangle".into(),
        "ELLIPSE" => "Ellipse".into(),
        "LINE" => "Line".into(),
        "REGULAR_POLYGON" => "Polygon".into(),
        "STAR" => "Star".into(),
        "TEXT" => "Text".into(),
        "VECTOR" => "Vector".into(),
        _ => kind.to_string(),
    }
}

pub(crate) fn parse_hex_solid_paint(hex_str: &str) -> Option<Val> {
    let s = hex_str.trim().strip_prefix('#').unwrap_or(hex_str.trim());
    if !s.is_ascii() {
        return None;
    }
    let bytes = hex::decode(s).ok()?;
    let (r, g, b, a) = match bytes.as_slice() {
        [r, g, b] => (*r, *g, *b, 255u8),
        [r, g, b, a] => (*r, *g, *b, *a),
        _ => return None,
    };
    Some(Val::Obj(Obj::from([
        ("type", Val::Enum(intern("SOLID"))),
        ("visible", Val::Bool(true)),
        ("opacity", Val::Float(f64::from(a) / 255.0)),
        ("blendMode", Val::Enum(intern("NORMAL"))),
        (
            "color",
            Val::Obj(Obj::from([
                ("r", Val::Float(f64::from(r) / 255.0)),
                ("g", Val::Float(f64::from(g) / 255.0)),
                ("b", Val::Float(f64::from(b) / 255.0)),
                ("a", Val::Float(1.0)),
            ])),
        ),
    ])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_commands_roundtrip_exact() {
        let cmds = rounded_rect_commands(120.0, 60.0, [8.0, 12.0, 4.0, 0.0], 0.6);
        let bytes = encode_path_commands(&cmds);
        let decoded = decode_path_commands(&bytes).expect("decode");
        assert_eq!(decoded, cmds);
    }

    #[test]
    fn all_five_constraint_modes_behave_as_figma_specifies() {
        assert_eq!(
            solve_1d_constraint("MIN", 200.0, 300.0, 20.0, 50.0),
            (20.0, 50.0)
        );
        assert_eq!(
            solve_1d_constraint("MAX", 200.0, 300.0, 130.0, 50.0),
            (230.0, 50.0)
        );
        assert_eq!(
            solve_1d_constraint("CENTER", 200.0, 300.0, 75.0, 50.0),
            (125.0, 50.0)
        );
        assert_eq!(
            solve_1d_constraint("STRETCH", 200.0, 300.0, 20.0, 160.0),
            (20.0, 260.0)
        );
        assert_eq!(
            solve_1d_constraint("SCALE", 200.0, 400.0, 30.0, 50.0),
            (60.0, 100.0)
        );
    }
}
