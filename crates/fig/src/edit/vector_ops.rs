//! Vector work: live booleans on any number of layers, convert to vector, flatten, outline
//! stroke, and vertex-level network edits.

use std::collections::HashSet;

use kurbo::{Affine, BezPath};
use serde::{Deserialize, Serialize};

use crate::edit::geometry::{encode_path_commands, make_path_entry};
use crate::edit::outline::{
    affine, bounds, combine, fill_outline, from_bez, network, network_from_path, stroke_outline,
    to_bez,
};
use crate::edit::space::{Bounds, Xform, bounds_in_parent, size};
use crate::edit::vector_net::{
    BooleanOp, VectorNetwork, VectorRegion, VectorSegment, VectorVertex, encode_vector_network,
    network_to_fill_paths,
};
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::{Document, FigError, Guid, Node, Obj, Val, intern};

/// One change to a vector network. Coordinates are in the network's own space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case")]
pub enum VectorEdit {
    MoveVertex {
        index: u32,
        x: f32,
        y: f32,
    },
    /// Adds a vertex at the end of the list (its index is the old vertex count).
    AddVertex {
        x: f32,
        y: f32,
    },
    /// Removes a vertex, every segment touching it and every loop using those segments.
    RemoveVertex {
        index: u32,
    },
    /// Sets a segment's control handles, relative to its start and end vertex.
    SetHandles {
        segment: u32,
        start: [f32; 2],
        end: [f32; 2],
    },
    /// Adds a straight segment (its index is the old segment count).
    AddSegment {
        start: u32,
        end: u32,
    },
    /// Removes a segment and every loop using it.
    RemoveSegment {
        index: u32,
    },
    /// Adds a filled region from loops of segment indices.
    AddRegion {
        loops: Vec<Vec<u32>>,
        #[serde(default)]
        even_odd: bool,
    },
    RemoveRegion {
        index: u32,
    },
}

fn missing(what: &str, i: u32, len: usize) -> FigError {
    FigError::malformed(format!("{what} {i} is out of range (there are {len})"))
}

/// Applies `edits` to `net` in order.
pub fn apply_edits(net: &mut VectorNetwork, edits: &[VectorEdit]) -> Result<(), FigError> {
    let finite = |v: f32| {
        if v.is_finite() {
            Ok(v)
        } else {
            Err(FigError::malformed("coordinates must be finite"))
        }
    };
    for e in edits {
        match e {
            VectorEdit::MoveVertex { index, x, y } => {
                let n = net.vertices.len();
                let v = net
                    .vertices
                    .get_mut(*index as usize)
                    .ok_or_else(|| missing("vertex", *index, n))?;
                v.x = finite(*x)?;
                v.y = finite(*y)?;
            }
            VectorEdit::AddVertex { x, y } => net.vertices.push(VectorVertex {
                style: 0,
                x: finite(*x)?,
                y: finite(*y)?,
            }),
            VectorEdit::RemoveVertex { index } => {
                if *index as usize >= net.vertices.len() {
                    return Err(missing("vertex", *index, net.vertices.len()));
                }
                let touching: Vec<u32> = net
                    .segments
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.start == *index || s.end == *index)
                    .map(|(i, _)| i as u32)
                    .collect();
                for s in touching.into_iter().rev() {
                    remove_segment(net, s);
                }
                net.vertices.remove(*index as usize);
                for s in &mut net.segments {
                    if s.start > *index {
                        s.start -= 1;
                    }
                    if s.end > *index {
                        s.end -= 1;
                    }
                }
            }
            VectorEdit::SetHandles {
                segment,
                start,
                end,
            } => {
                let n = net.segments.len();
                let s = net
                    .segments
                    .get_mut(*segment as usize)
                    .ok_or_else(|| missing("segment", *segment, n))?;
                s.tangent_start = (finite(start[0])?, finite(start[1])?);
                s.tangent_end = (finite(end[0])?, finite(end[1])?);
            }
            VectorEdit::AddSegment { start, end } => {
                let n = net.vertices.len();
                for v in [start, end] {
                    if *v as usize >= n {
                        return Err(missing("vertex", *v, n));
                    }
                }
                net.segments.push(VectorSegment {
                    style: 0,
                    start: *start,
                    tangent_start: (0.0, 0.0),
                    end: *end,
                    tangent_end: (0.0, 0.0),
                });
            }
            VectorEdit::RemoveSegment { index } => {
                if *index as usize >= net.segments.len() {
                    return Err(missing("segment", *index, net.segments.len()));
                }
                remove_segment(net, *index);
            }
            VectorEdit::AddRegion { loops, even_odd } => {
                for &s in loops.iter().flatten() {
                    if s as usize >= net.segments.len() {
                        return Err(missing("segment", s, net.segments.len()));
                    }
                }
                net.regions.push(VectorRegion {
                    winding_nonzero: !even_odd,
                    style_id: 0,
                    loops: loops.clone(),
                });
            }
            VectorEdit::RemoveRegion { index } => {
                if *index as usize >= net.regions.len() {
                    return Err(missing("region", *index, net.regions.len()));
                }
                net.regions.remove(*index as usize);
            }
        }
    }
    // A loop must still close for its region to fill.
    let closed = |net: &VectorNetwork, lp: &[u32]| {
        let probe = VectorNetwork {
            vertices: net.vertices.clone(),
            segments: net.segments.clone(),
            regions: vec![VectorRegion {
                winding_nonzero: true,
                style_id: 0,
                loops: vec![lp.to_vec()],
            }],
        };
        !network_to_fill_paths(&probe).is_empty()
    };
    for (i, r) in net.regions.iter().enumerate() {
        if let Some(lp) = r.loops.iter().find(|lp| !closed(net, lp)) {
            return Err(FigError::malformed(format!(
                "region {i} has a loop that does not close: {lp:?}"
            )));
        }
    }
    Ok(())
}

/// Removes segment `index`, the loops using it (and regions left empty), renumbering the rest.
fn remove_segment(net: &mut VectorNetwork, index: u32) {
    net.segments.remove(index as usize);
    for r in &mut net.regions {
        r.loops.retain(|lp| !lp.contains(&index));
        for lp in &mut r.loops {
            for s in lp.iter_mut() {
                if *s > index {
                    *s -= 1;
                }
            }
        }
    }
    net.regions.retain(|r| !r.loops.is_empty());
}

/// `vectorData` and `fillGeometry` for `net` drawn at `scale` (network to layer units), with the
/// network and fill blobs appended to `txn`.
fn network_fields(
    doc: &Document,
    txn: &mut Txn,
    net: &VectorNetwork,
    vector_data: Option<&Val>,
    normalized: (f64, f64),
    scale: Affine,
) -> Obj {
    let net_idx = doc.blobs.len() + txn.add_blobs.len();
    txn.add_blobs.push(encode_vector_network(net));
    let mut vd = vector_data
        .cloned()
        .map(Val::into_owned)
        .unwrap_or_else(|| Val::Obj(Obj::new()));
    vd.set("vectorNetworkBlob", Val::Int(net_idx as i64));
    vd.set(
        "normalizedSize",
        Val::Obj(Obj::from([
            ("x", Val::Float(normalized.0)),
            ("y", Val::Float(normalized.1)),
        ])),
    );
    let mut fills = Vec::new();
    for (cmds, even_odd) in network_to_fill_paths(net) {
        let i = doc.blobs.len() + txn.add_blobs.len();
        txn.add_blobs
            .push(encode_path_commands(&from_bez(&(scale * to_bez(&cmds)))));
        fills.push(make_path_entry(i, even_odd));
    }
    let mut o = Obj::new();
    o.insert("vectorData", vd);
    o.insert("fillGeometry", Val::Arr(fills));
    o
}

/// Applies `edits` to vector `target`'s network and rebuilds its geometry.
pub fn edit_network(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    edits: &[VectorEdit],
) -> Result<(), FigError> {
    let node = node(doc, target)?;
    if node.kind != "VECTOR" {
        return Err(FigError::malformed(format!("{target} is not a vector")));
    }
    let (mut net, scale) = network(doc, &node.data)
        .ok_or_else(|| FigError::malformed(format!("{target} has no readable vector network")))?;
    apply_edits(&mut net, edits)?;
    let normalized = node
        .data
        .path(&["vectorData", "normalizedSize"])
        .map(|n| {
            (
                n.num("x").map_or(0.0, f64::from),
                n.num("y").map_or(0.0, f64::from),
            )
        })
        .unwrap_or_else(|| size(&node.data));
    // The layer's box follows its points, as in Figma: when they move past it (or away from
    // its edges) the network is re-expressed in layer units from the new top-left, the size
    // fits the curves, and the layer moves so nothing moves on the page.
    let mut set = Obj::new();
    let (normalized, scale) = match refit(&mut net, scale, size(&node.data)) {
        Some((min, (w, h))) => {
            let t = Xform::of(&node.data);
            let at = t.apply(min);
            let mut moved = t;
            moved.m02 = at.0;
            moved.m12 = at.1;
            set.insert("transform", moved.to_val());
            set.insert(
                "size",
                Val::Obj(Obj::from([("x", Val::Float(w)), ("y", Val::Float(h))])),
            );
            ((w, h), Affine::IDENTITY)
        }
        None => (normalized, scale),
    };
    let fields = network_fields(
        doc,
        txn,
        &net,
        node.data.get("vectorData"),
        normalized,
        scale,
    );
    for (k, v) in fields.iter() {
        set.insert(k, v.clone());
    }
    let unset = stale_stroke(node);
    txn.modify(target, set, unset);
    Ok(())
}

/// When `net`'s curves (at `scale`, in layer units) no longer start at the box's top-left or
/// fill its `size`, rewrites `net` in layer units from their top-left and returns that corner
/// and the curves' size.
fn refit(
    net: &mut VectorNetwork,
    scale: Affine,
    size: (f64, f64),
) -> Option<((f64, f64), (f64, f64))> {
    use kurbo::Shape;
    let mut path = BezPath::new();
    let at = |i: u32| {
        net.vertices
            .get(i as usize)
            .map(|v| (f64::from(v.x), f64::from(v.y)))
    };
    for s in &net.segments {
        let (Some(a), Some(b)) = (at(s.start), at(s.end)) else {
            continue;
        };
        let (ts, te) = (s.tangent_start, s.tangent_end);
        path.move_to(a);
        path.curve_to(
            (a.0 + f64::from(ts.0), a.1 + f64::from(ts.1)),
            (b.0 + f64::from(te.0), b.1 + f64::from(te.1)),
            b,
        );
    }
    for v in &net.vertices {
        path.move_to((f64::from(v.x), f64::from(v.y)));
        path.line_to((f64::from(v.x), f64::from(v.y)));
    }
    if net.vertices.is_empty() {
        return None;
    }
    let b = (scale * path).bounding_box();
    let same = |a: f64, b: f64| (a - b).abs() < 1e-3;
    if same(b.x0, 0.0) && same(b.y0, 0.0) && same(b.width(), size.0) && same(b.height(), size.1) {
        return None;
    }
    let to_layer = Affine::translate((-b.x0, -b.y0)) * scale;
    let linear = |x: f32, y: f32| {
        let o = to_layer * kurbo::Point::ZERO;
        let p = to_layer * kurbo::Point::new(f64::from(x), f64::from(y));
        ((p.x - o.x) as f32, (p.y - o.y) as f32)
    };
    for v in &mut net.vertices {
        let p = to_layer * kurbo::Point::new(f64::from(v.x), f64::from(v.y));
        (v.x, v.y) = (p.x as f32, p.y as f32);
    }
    for s in &mut net.segments {
        s.tangent_start = linear(s.tangent_start.0, s.tangent_start.1);
        s.tangent_end = linear(s.tangent_end.0, s.tangent_end.1);
    }
    Some(((b.x0, b.y0), (b.width(), b.height())))
}

fn stale_stroke(node: &Node) -> Vec<String> {
    if node.data.get("strokeGeometry").is_some() {
        vec!["strokeGeometry".to_string()]
    } else {
        Vec::new()
    }
}

fn node(doc: &Document, id: Guid) -> Result<&Node, FigError> {
    doc.node(id)
        .ok_or_else(|| FigError::malformed(format!("node {id} does not exist")))
}

/// A network for a parametric layer: its outline, or for a line its segment.
fn shape_network(n: &Node) -> Result<VectorNetwork, FigError> {
    if n.kind == "LINE" {
        let (w, _) = size(&n.data);
        return Ok(VectorNetwork {
            vertices: vec![
                VectorVertex {
                    style: 0,
                    x: 0.0,
                    y: 0.0,
                },
                VectorVertex {
                    style: 0,
                    x: w as f32,
                    y: 0.0,
                },
            ],
            segments: vec![VectorSegment {
                style: 0,
                start: 0,
                tangent_start: (0.0, 0.0),
                end: 1,
                tangent_end: (0.0, 0.0),
            }],
            regions: Vec::new(),
        });
    }
    // The shape's own outline (not a normalised one), so vertices keep the shape's order:
    // a rectangle's start at its top-left corner and run clockwise, as in Figma.
    let cmds = crate::edit::geometry::rebuild_shape_geometry(&n.kind, &n.data)
        .ok_or_else(|| FigError::malformed(format!("{} has no shape", n.id)))?;
    Ok(network_from_path(&to_bez(&cmds)))
}

/// Turns a shape (rectangle, ellipse, polygon, star, line) into an editable vector in place.
pub fn convert_to_vector(doc: &Document, txn: &mut Txn, target: Guid) -> Result<(), FigError> {
    let n = node(doc, target)?;
    if !matches!(
        n.kind.as_str(),
        "ROUNDED_RECTANGLE" | "RECTANGLE" | "ELLIPSE" | "REGULAR_POLYGON" | "STAR" | "LINE"
    ) {
        return Err(FigError::malformed(format!(
            "{target} is {}, not a shape that converts to a vector",
            n.kind
        )));
    }
    let net = shape_network(n)?;
    let mut set = network_fields(doc, txn, &net, None, size(&n.data), Affine::IDENTITY);
    set.insert("type", Val::Enum(intern("VECTOR")));
    // Parametric fields no longer drive the shape.
    let unset: Vec<String> = [
        "cornerRadius",
        "rectangleTopLeftCornerRadius",
        "rectangleTopRightCornerRadius",
        "rectangleBottomLeftCornerRadius",
        "rectangleBottomRightCornerRadius",
        "rectangleCornerRadiiIndependent",
        "count",
        "starInnerScale",
        "arcData",
        "strokeGeometry",
    ]
    .into_iter()
    .filter(|f| n.data.get(f).is_some())
    .map(str::to_string)
    .collect();
    txn.modify(target, set, unset);
    Ok(())
}

/// Sibling targets in z order (bottom first), checked: non-empty, present, one parent.
fn siblings(doc: &Document, targets: &[Guid]) -> Result<(Guid, Vec<Guid>), FigError> {
    let first = *targets
        .first()
        .ok_or_else(|| FigError::malformed("no targets"))?;
    let parent = node(doc, first)?
        .parent
        .ok_or_else(|| FigError::malformed(format!("{first} has no parent")))?;
    let mut seen = HashSet::new();
    for t in targets {
        if !seen.insert(*t) {
            return Err(FigError::malformed(format!("{t} is listed twice")));
        }
        if node(doc, *t)?.parent != Some(parent) {
            return Err(FigError::malformed("targets must share one parent"));
        }
    }
    let ordered = doc
        .children(parent)
        .iter()
        .copied()
        .filter(|c| targets.contains(c))
        .collect();
    Ok((parent, ordered))
}

/// Base fields for a new layer under `parent` at `position`.
fn layer(id: Guid, kind: &str, name: &str, parent: Guid, position: String) -> Obj {
    let mut o = Obj::new();
    o.insert("guid", id.to_val());
    o.insert("phase", Val::Enum(intern("CREATED")));
    o.insert("type", Val::Enum(intern(kind)));
    o.insert("name", Val::Str(name.to_string()));
    o.insert("visible", Val::Bool(true));
    o.insert("opacity", Val::Float(1.0));
    o.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", parent.to_val()),
            ("position", Val::Str(position)),
        ])),
    );
    o
}

/// Paint and stroke fields copied from `src`.
fn look_of(src: &Node, o: &mut Obj) {
    for f in [
        "fillPaints",
        "strokePaints",
        "strokeWeight",
        "strokeAlign",
        "strokeJoin",
        "strokeCap",
        "dashPattern",
        "effects",
        "blendMode",
    ] {
        if let Some(v) = src.data.get(f) {
            o.insert(f, v.clone().into_owned());
        }
    }
}

/// Wraps sibling `targets` in a live `BOOLEAN_OPERATION` at the topmost one's place. Its
/// result geometry is computed by the derive pass ([`refresh_boolean`]), like every later edit
/// of its operands. Paints come from the bottom operand.
pub fn make_boolean(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: BooleanOp,
    targets: &[Guid],
    name: Option<&str>,
) -> Result<Guid, FigError> {
    let (parent, ordered) = siblings(doc, targets)?;
    if ordered.len() < 2 {
        return Err(FigError::malformed("a boolean needs at least two layers"));
    }
    let union = ordered
        .iter()
        .filter_map(|&t| doc.node(t).and_then(|n| bounds_in_parent(&n.data)))
        .reduce(Bounds::union)
        .ok_or_else(|| FigError::malformed("the layers have no bounds"))?;
    let bottom = node(doc, ordered[0])?;
    let top = *ordered
        .last()
        .ok_or_else(|| FigError::malformed("no targets"))?;
    let id = alloc.next_guid();
    let mut o = layer(
        id,
        "BOOLEAN_OPERATION",
        name.unwrap_or(op.figma_name()),
        parent,
        node(doc, top)?.position.clone(),
    );
    o.insert("booleanOperation", Val::Enum(intern(op.figma_name())));
    o.insert("transform", Xform::translate(union.x0, union.y0).to_val());
    o.insert(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(union.width())),
            ("y", Val::Float(union.height())),
        ])),
    );
    look_of(bottom, &mut o);
    txn.create_node(id, Val::Obj(o));
    let back = Xform::translate(-union.x0, -union.y0);
    for (t, pos) in ordered
        .iter()
        .zip(fractional::n_between(None, None, ordered.len()))
    {
        let n = node(doc, *t)?;
        let mut set = Obj::new();
        set.insert("transform", back.compose(Xform::of(&n.data)).to_val());
        set.insert(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", id.to_val()),
                ("position", Val::Str(pos)),
            ])),
        );
        txn.modify(*t, set, Vec::new());
    }
    Ok(id)
}

/// Writes `boolean`'s result geometry from its current operands. An operand that cannot be
/// outlined (text) drops the stored result, so the renderer draws the operands and counts it.
pub fn refresh_boolean(doc: &Document, txn: &mut Txn, boolean: Guid) {
    let Some(n) = doc.node(boolean) else { return };
    match fill_outline(doc, boolean) {
        Ok(path) => {
            let i = doc.blobs.len() + txn.add_blobs.len();
            txn.add_blobs.push(encode_path_commands(&from_bez(&path)));
            let mut set = Obj::new();
            set.insert(
                "fillGeometry",
                Val::Arr(if path.elements().is_empty() {
                    Vec::new()
                } else {
                    vec![make_path_entry(i, false)]
                }),
            );
            txn.modify(boolean, set, stale_stroke(n));
        }
        Err(_) => {
            let unset: Vec<String> = ["fillGeometry", "strokeGeometry"]
                .into_iter()
                .filter(|f| n.data.get(f).is_some())
                .map(str::to_string)
                .collect();
            if !unset.is_empty() {
                txn.modify(boolean, Obj::new(), unset);
            }
        }
    }
}

/// Where a new layer goes: its parent, sibling position, and the transform from the space its
/// path is given in to the parent.
struct Placement {
    parent: Guid,
    position: String,
    to_parent: Xform,
}

/// Fields of a new vector `id` drawing `path` (given in `at.to_parent`'s source space).
fn vector_for(
    doc: &Document,
    txn: &mut Txn,
    id: Guid,
    name: &str,
    at: Placement,
    path: &BezPath,
) -> Result<Obj, FigError> {
    let Placement {
        parent,
        position,
        to_parent,
    } = at;
    let b = bounds(path).ok_or_else(|| FigError::malformed("the result is empty"))?;
    let local = Affine::translate((-b.x0, -b.y0)) * path.clone();
    let net = network_from_path(&local);
    let mut o = layer(id, "VECTOR", name, parent, position);
    o.insert(
        "transform",
        to_parent.compose(Xform::translate(b.x0, b.y0)).to_val(),
    );
    let (w, h) = (b.width().max(0.01), b.height().max(0.01));
    o.insert(
        "size",
        Val::Obj(Obj::from([("x", Val::Float(w)), ("y", Val::Float(h))])),
    );
    for (k, v) in network_fields(doc, txn, &net, None, (w, h), Affine::IDENTITY).iter() {
        o.insert(k, v.clone());
    }
    Ok(o)
}

/// Merges sibling `targets` into one vector of their combined fill area, at the topmost one's
/// place, with the bottom one's paints; the targets are removed.
pub fn flatten(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    targets: &[Guid],
    name: Option<&str>,
) -> Result<Guid, FigError> {
    let (parent, ordered) = siblings(doc, targets)?;
    let mut outlines = Vec::new();
    for &t in &ordered {
        let n = node(doc, t)?;
        outlines.push(affine(Xform::of(&n.data)) * fill_outline(doc, t)?);
    }
    let path = combine(BooleanOp::Union, &outlines)?;
    let bottom = node(doc, ordered[0])?;
    let top = *ordered
        .last()
        .ok_or_else(|| FigError::malformed("no targets"))?;
    let id = alloc.next_guid();
    let mut o = vector_for(
        doc,
        txn,
        id,
        name.unwrap_or(&bottom.name),
        Placement {
            parent,
            position: node(doc, top)?.position.clone(),
            to_parent: Xform::IDENTITY,
        },
        &path,
    )?;
    look_of(bottom, &mut o);
    txn.create_node(id, Val::Obj(o));
    for &t in &ordered {
        delete_subtree(doc, txn, t);
    }
    Ok(id)
}

fn delete_subtree(doc: &Document, txn: &mut Txn, root: Guid) {
    let mut order = Vec::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if seen.insert(id) {
            order.push(id);
            stack.extend(doc.children(id).iter().copied());
        }
    }
    for id in order.into_iter().rev() {
        txn.delete_node(id);
    }
}

/// Turns `target`'s stroke into a filled vector placed just above it, with the stroke's paints.
/// The layer loses its stroke; a layer with no visible fill left is removed.
pub fn outline_stroke(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    target: Guid,
) -> Result<Guid, FigError> {
    let n = node(doc, target)?;
    let strokes = n
        .data
        .get("strokePaints")
        .map(Val::items)
        .unwrap_or_default();
    if strokes.is_empty() {
        return Err(FigError::malformed(format!("{target} has no stroke")));
    }
    let parent = n
        .parent
        .ok_or_else(|| FigError::malformed(format!("{target} has no parent")))?;
    let path = stroke_outline(doc, target)?;
    let siblings = doc.children(parent);
    let at = siblings.iter().position(|s| *s == target).unwrap_or(0);
    let next = siblings
        .get(at + 1)
        .and_then(|id| doc.node(*id))
        .map(|s| s.position.as_str());
    let position = fractional::try_between(Some(n.position.as_str()), next).ok_or_else(|| {
        FigError::malformed("the sibling positions above the layer are malformed")
    })?;
    let id = alloc.next_guid();
    let mut o = vector_for(
        doc,
        txn,
        id,
        &format!("{} stroke", n.name),
        Placement {
            parent,
            position,
            to_parent: Xform::of(&n.data),
        },
        &path,
    )?;
    o.insert(
        "fillPaints",
        Val::Arr(strokes.iter().map(|p| p.clone().into_owned()).collect()),
    );
    txn.create_node(id, Val::Obj(o));
    let has_fill = n
        .data
        .get("fillPaints")
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .any(|p| p.flag("visible") != Some(false));
    if has_fill || !doc.children(target).is_empty() {
        let mut set = Obj::new();
        set.insert("strokePaints", Val::Arr(Vec::new()));
        txn.modify(target, set, stale_stroke(n));
    } else {
        txn.delete_node(target);
    }
    Ok(id)
}
