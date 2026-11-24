//! Layer outlines as `kurbo` paths, and exact boolean operations on them (`linesweeper`).
//!
//! Used by booleans, flatten and outline stroke. An outline is a filled area in a layer's own
//! coordinates, normalised so the non-zero rule fills it.

use std::collections::HashSet;

use kurbo::{Affine, BezPath, Cap, Join, PathEl, Point, Shape, Stroke, StrokeOpts};
use linesweeper::{BinaryOp, FillRule, binary_op};

use crate::edit::geometry::{PathCmd, decode_path_commands, rebuild_shape_geometry};
use crate::edit::groups::is_group;
use crate::edit::space::{Xform, size};
use crate::edit::vector_net::{
    BooleanOp, VectorNetwork, VectorRegion, VectorSegment, VectorVertex, decode_vector_network,
    network_to_fill_paths,
};
use crate::{Document, FigError, Guid, Node, Val};

/// Nested booleans deeper than this are refused (hostile files).
const MAX_DEPTH: usize = 32;
/// Curve flattening tolerance for the stroker, in layer units.
const TOLERANCE: f64 = 0.01;

/// `kurbo` affine for a Figma transform.
pub fn affine(t: Xform) -> Affine {
    Affine::new([t.m00, t.m10, t.m01, t.m11, t.m02, t.m12])
}

/// A path from Figma path commands.
pub fn to_bez(cmds: &[PathCmd]) -> BezPath {
    let mut p = BezPath::new();
    let pt = |x: f32, y: f32| Point::new(f64::from(x), f64::from(y));
    for c in cmds {
        match *c {
            PathCmd::MoveTo(x, y) => p.move_to(pt(x, y)),
            PathCmd::LineTo(x, y) => p.line_to(pt(x, y)),
            PathCmd::QuadTo(a, b, x, y) => p.quad_to(pt(a, b), pt(x, y)),
            PathCmd::CubicTo(a, b, c, d, x, y) => p.curve_to(pt(a, b), pt(c, d), pt(x, y)),
            PathCmd::Close => p.close_path(),
        }
    }
    p
}

/// Figma path commands from a path.
pub fn from_bez(p: &BezPath) -> Vec<PathCmd> {
    let f = |v: f64| v as f32;
    p.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(a) => PathCmd::MoveTo(f(a.x), f(a.y)),
            PathEl::LineTo(a) => PathCmd::LineTo(f(a.x), f(a.y)),
            PathEl::QuadTo(a, b) => PathCmd::QuadTo(f(a.x), f(a.y), f(b.x), f(b.y)),
            PathEl::CurveTo(a, b, c) => {
                PathCmd::CubicTo(f(a.x), f(a.y), f(b.x), f(b.y), f(c.x), f(c.y))
            }
            PathEl::ClosePath => PathCmd::Close,
        })
        .collect()
}

fn op_error(e: linesweeper::Error) -> FigError {
    FigError::malformed(format!("boolean operation failed: {e}"))
}

/// Runs `op` on two non-zero outlines.
fn binary(a: &BezPath, b: &BezPath, op: BinaryOp) -> Result<BezPath, FigError> {
    if a.elements().is_empty() && b.elements().is_empty() {
        return Ok(BezPath::new());
    }
    let contours = binary_op(a, b, FillRule::NonZero, op).map_err(op_error)?;
    let mut out = BezPath::new();
    for c in contours.contours() {
        out.extend(c.path.iter());
    }
    Ok(out)
}

/// `path` filled with `rule`, as an equivalent non-zero outline of simple closed contours.
pub fn normalize(path: &BezPath, rule: FillRule) -> Result<BezPath, FigError> {
    if path.elements().is_empty() {
        return Ok(BezPath::new());
    }
    let closed = close_open_subpaths(path);
    let contours = binary_op(&closed, &BezPath::new(), rule, BinaryOp::Union).map_err(op_error)?;
    let mut out = BezPath::new();
    for c in contours.contours() {
        out.extend(c.path.iter());
    }
    Ok(out)
}

/// Closes every subpath (a fill closes them implicitly; the sweep needs them closed).
fn close_open_subpaths(p: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in p.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(*el);
    }
    if open {
        out.close_path();
    }
    out
}

/// Combines outlines in order: union, the first minus the rest, intersection, or exclusion.
pub fn combine(op: BooleanOp, outlines: &[BezPath]) -> Result<BezPath, FigError> {
    let Some((first, rest)) = outlines.split_first() else {
        return Ok(BezPath::new());
    };
    let mut acc = first.clone();
    match op {
        BooleanOp::Subtract => {
            let mut cut = BezPath::new();
            for o in rest {
                cut = binary(&cut, o, BinaryOp::Union)?;
            }
            acc = binary(&acc, &cut, BinaryOp::Difference)?;
        }
        _ => {
            let bop = match op {
                BooleanOp::Union => BinaryOp::Union,
                BooleanOp::Intersect => BinaryOp::Intersection,
                _ => BinaryOp::Xor,
            };
            for o in rest {
                acc = binary(&acc, o, bop)?;
            }
        }
    }
    Ok(acc)
}

fn blob<'a>(doc: &'a Document, index: Option<&Val>) -> Option<&'a [u8]> {
    let i = usize::try_from(index?.as_u32()?).ok()?;
    doc.blobs.get(i).map(Vec::as_slice)
}

/// The layer's vector network, in layer units (scaled from `normalizedSize`).
pub fn network(doc: &Document, data: &Val) -> Option<(VectorNetwork, Affine)> {
    let net = decode_vector_network(blob(doc, data.path(&["vectorData", "vectorNetworkBlob"]))?)?;
    let (w, h) = size(data);
    let (nw, nh) = data
        .path(&["vectorData", "normalizedSize"])
        .map(|n| {
            (
                n.num("x").map_or(w, f64::from),
                n.num("y").map_or(h, f64::from),
            )
        })
        .unwrap_or((w, h));
    let scale = if nw > 0.0 && nh > 0.0 {
        Affine::scale_non_uniform(w / nw, h / nh)
    } else {
        Affine::IDENTITY
    };
    Some((net, scale))
}

/// The stored or computed fill paths of a layer with their fill rules, in its own space.
fn raw_fills(doc: &Document, node: &Node) -> Vec<(BezPath, FillRule)> {
    let data = &node.data;
    let mut out = Vec::new();
    for entry in data.get("fillGeometry").map(Val::items).unwrap_or_default() {
        if let Some(cmds) = blob(doc, entry.get("commandsBlob")).and_then(decode_path_commands) {
            let rule = if entry.text("windingRule") == Some("ODD") {
                FillRule::EvenOdd
            } else {
                FillRule::NonZero
            };
            out.push((to_bez(&cmds), rule));
        }
    }
    if out.is_empty()
        && node.kind == "VECTOR"
        && let Some((net, scale)) = network(doc, data)
    {
        for (cmds, even_odd) in network_to_fill_paths(&net) {
            let rule = if even_odd {
                FillRule::EvenOdd
            } else {
                FillRule::NonZero
            };
            out.push((scale * to_bez(&cmds), rule));
        }
    }
    if out.is_empty()
        && let Some(cmds) = rebuild_shape_geometry(&node.kind, data)
    {
        out.push((to_bez(&cmds), FillRule::NonZero));
    }
    out
}

/// The area a layer contributes to a boolean, in its own space: its fill shape; for a group,
/// the union of its children; for a boolean, its live result.
pub fn fill_outline(doc: &Document, id: Guid) -> Result<BezPath, FigError> {
    fill_outline_at(doc, id, 0, &mut HashSet::new())
}

fn fill_outline_at(
    doc: &Document,
    id: Guid,
    depth: usize,
    seen: &mut HashSet<Guid>,
) -> Result<BezPath, FigError> {
    if depth > MAX_DEPTH || !seen.insert(id) {
        return Err(FigError::malformed(format!(
            "{id}: booleans nest too deeply"
        )));
    }
    let node = doc
        .node(id)
        .ok_or_else(|| FigError::malformed(format!("node {id} does not exist")))?;
    let children_in_own_space = |seen: &mut HashSet<Guid>| -> Result<Vec<BezPath>, FigError> {
        let mut v = Vec::new();
        for &c in doc.children(id) {
            let child = doc.node(c);
            if child.is_some_and(|n| n.data.flag("visible") == Some(false)) {
                continue;
            }
            let t = child.map_or(Xform::IDENTITY, |n| Xform::of(&n.data));
            v.push(affine(t) * fill_outline_at(doc, c, depth + 1, seen)?);
        }
        Ok(v)
    };
    let out = if node.kind == "BOOLEAN_OPERATION" {
        let op = node
            .data
            .text("booleanOperation")
            .and_then(BooleanOp::from_figma_name)
            .unwrap_or(BooleanOp::Union);
        combine(op, &children_in_own_space(seen)?)?
    } else if is_group(node) {
        combine(BooleanOp::Union, &children_in_own_space(seen)?)?
    } else if node.kind == "TEXT" {
        return Err(FigError::malformed(format!(
            "{id} is text: outlining text needs its fonts, which the engine does not load"
        )));
    } else {
        let mut acc = BezPath::new();
        for (p, rule) in raw_fills(doc, node) {
            let n = normalize(&p, rule)?;
            acc = binary(&acc, &n, BinaryOp::Union)?;
        }
        acc
    };
    seen.remove(&id);
    Ok(out)
}

/// The path a layer's stroke follows, in its own space: its network's segments for a vector,
/// the segment for a line, else its fill shape.
fn stroke_centerline(doc: &Document, node: &Node) -> Result<BezPath, FigError> {
    if node.kind == "VECTOR"
        && let Some((net, scale)) = network(doc, &node.data)
    {
        let mut p = BezPath::new();
        for s in &net.segments {
            let (Some(a), Some(b)) = (
                net.vertices.get(s.start as usize),
                net.vertices.get(s.end as usize),
            ) else {
                continue;
            };
            let (a, b) = (
                Point::new(f64::from(a.x), f64::from(a.y)),
                Point::new(f64::from(b.x), f64::from(b.y)),
            );
            p.move_to(a);
            if s.tangent_start == (0.0, 0.0) && s.tangent_end == (0.0, 0.0) {
                p.line_to(b);
            } else {
                let t = |v: (f32, f32)| kurbo::Vec2::new(f64::from(v.0), f64::from(v.1));
                p.curve_to(a + t(s.tangent_start), b + t(s.tangent_end), b);
            }
        }
        return Ok(scale * p);
    }
    if node.kind == "LINE" {
        let (w, _) = size(&node.data);
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((w, 0.0));
        return Ok(p);
    }
    let fills = raw_fills(doc, node);
    let mut p = BezPath::new();
    for (f, _) in fills {
        p.extend(f.iter());
    }
    if p.elements().is_empty() {
        return Err(FigError::malformed(format!(
            "{} has no shape to stroke",
            node.id
        )));
    }
    Ok(p)
}

/// The area a layer's stroke covers, in its own space, honouring weight, alignment, caps,
/// joins and dashes.
pub fn stroke_outline(doc: &Document, id: Guid) -> Result<BezPath, FigError> {
    let node = doc
        .node(id)
        .ok_or_else(|| FigError::malformed(format!("node {id} does not exist")))?;
    let d = &node.data;
    let weight = d.num("strokeWeight").map_or(1.0, f64::from);
    if !(weight.is_finite() && weight > 0.0) {
        return Err(FigError::malformed(format!("{id} has no stroke weight")));
    }
    let line = stroke_centerline(doc, node)?;
    let closed = !matches!(node.kind.as_str(), "LINE" | "VECTOR");
    let align = d.text("strokeAlign").unwrap_or("CENTER");
    // Inside and outside strokes are a centred stroke of twice the weight, clipped.
    let width = if closed && align != "CENTER" {
        weight * 2.0
    } else {
        weight
    };
    let join = match d.text("strokeJoin") {
        Some("ROUND") => Join::Round,
        Some("BEVEL") => Join::Bevel,
        _ => Join::Miter,
    };
    let cap = match d.text("strokeCap") {
        Some("ROUND") => Cap::Round,
        Some("SQUARE") => Cap::Square,
        _ => Cap::Butt,
    };
    let miter = d.num("miterLimit").map_or(4.0, f64::from).max(1.0);
    let dashes: Vec<f64> = d
        .get("dashPattern")
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .filter_map(Val::as_f32)
        .map(f64::from)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .collect();
    let mut style = Stroke::new(width)
        .with_join(join)
        .with_caps(cap)
        .with_miter_limit(miter);
    if dashes.iter().any(|d| *d > 0.0) {
        style = style.with_dashes(0.0, dashes);
    }
    let raw = kurbo::stroke(line.iter(), &style, &StrokeOpts::default(), TOLERANCE);
    let band = normalize(&raw, FillRule::NonZero)?;
    if !closed || align == "CENTER" {
        return Ok(band);
    }
    let fill = fill_outline(doc, id)?;
    let op = if align == "INSIDE" {
        BinaryOp::Intersection
    } else {
        BinaryOp::Difference
    };
    binary(&band, &fill, op)
}

/// A vector network tracing `path` (each subpath a loop of one filled region).
pub fn network_from_path(path: &BezPath) -> VectorNetwork {
    let mut net = VectorNetwork::default();
    let mut loops = Vec::new();
    let mut current: Vec<u32> = Vec::new();
    let mut start: Option<u32> = None;
    let mut last: Option<(u32, Point)> = None;
    let vertex = |net: &mut VectorNetwork, p: Point| -> u32 {
        net.vertices.push(VectorVertex {
            style: 0,
            x: p.x as f32,
            y: p.y as f32,
        });
        (net.vertices.len() - 1) as u32
    };
    let push_seg = |net: &mut VectorNetwork,
                    from: (u32, Point),
                    to: u32,
                    to_p: Point,
                    c: Option<(Point, Point)>,
                    current: &mut Vec<u32>| {
        let (ts, te) = match c {
            Some((c1, c2)) => (
                ((c1.x - from.1.x) as f32, (c1.y - from.1.y) as f32),
                ((c2.x - to_p.x) as f32, (c2.y - to_p.y) as f32),
            ),
            None => ((0.0, 0.0), (0.0, 0.0)),
        };
        net.segments.push(VectorSegment {
            style: 0,
            start: from.0,
            tangent_start: ts,
            end: to,
            tangent_end: te,
        });
        current.push((net.segments.len() - 1) as u32);
    };
    let finish = |current: &mut Vec<u32>, loops: &mut Vec<Vec<u32>>| {
        if !current.is_empty() {
            loops.push(std::mem::take(current));
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                finish(&mut current, &mut loops);
                let v = vertex(&mut net, p);
                start = Some(v);
                last = Some((v, p));
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                let Some(from) = last else { continue };
                let ctrl = match *el {
                    PathEl::QuadTo(q, _) => Some((
                        from.1 + (q - from.1) * (2.0 / 3.0),
                        p + (q - p) * (2.0 / 3.0),
                    )),
                    PathEl::CurveTo(a, b, _) => Some((a, b)),
                    _ => None,
                };
                // A segment back to the start closes onto the first vertex.
                let to = match start {
                    Some(s)
                        if net.vertices[s as usize].x == p.x as f32
                            && net.vertices[s as usize].y == p.y as f32 =>
                    {
                        s
                    }
                    _ => vertex(&mut net, p),
                };
                push_seg(&mut net, from, to, p, ctrl, &mut current);
                last = Some((to, p));
            }
            PathEl::ClosePath => {
                if let (Some(from), Some(s)) = (last, start)
                    && from.0 != s
                {
                    let sp = net.vertices[s as usize];
                    let sp = Point::new(f64::from(sp.x), f64::from(sp.y));
                    push_seg(&mut net, from, s, sp, None, &mut current);
                }
                finish(&mut current, &mut loops);
                last = start.map(|s| {
                    let v = net.vertices[s as usize];
                    (s, Point::new(f64::from(v.x), f64::from(v.y)))
                });
            }
        }
    }
    finish(&mut current, &mut loops);
    if !loops.is_empty() {
        net.regions.push(VectorRegion {
            winding_nonzero: true,
            style_id: 0,
            loops,
        });
    }
    net
}

/// Bounds of a path (`None` when empty).
pub fn bounds(p: &BezPath) -> Option<kurbo::Rect> {
    (!p.elements().is_empty()).then(|| p.bounding_box())
}
