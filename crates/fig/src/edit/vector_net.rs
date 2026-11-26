//! Lossless `vectorNetworkBlob` codec and 2D polygon / flattened-Bezier Boolean operations
//! (`UNION`, `SUBTRACT`, `INTERSECT`, `EXCLUDE`).

use serde::{Deserialize, Serialize};

use crate::FigError;
use crate::edit::geometry::{PathCmd, cubic_extrema};

/// One vertex of a Figma `vectorNetworkBlob` (12 bytes: `u32` style, `f32` x, `f32` y).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorVertex {
    pub style: u32,
    pub x: f32,
    pub y: f32,
}

/// One segment of a Figma `vectorNetworkBlob` (28 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorSegment {
    pub style: u32,
    pub start: u32,
    pub tangent_start: (f32, f32),
    pub end: u32,
    pub tangent_end: (f32, f32),
}

/// One filled region of a Figma `vectorNetworkBlob`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorRegion {
    pub winding_nonzero: bool,
    pub style_id: u32,
    pub loops: Vec<Vec<u32>>,
}

/// Complete editable Figma Vector Network.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorNetwork {
    pub vertices: Vec<VectorVertex>,
    pub segments: Vec<VectorSegment>,
    pub regions: Vec<VectorRegion>,
}

/// Decodes a `vectorNetworkBlob` byte slice into a [`VectorNetwork`].
pub fn decode_vector_network(bytes: &[u8]) -> Option<VectorNetwork> {
    struct Cur<'a>(&'a [u8]);
    impl<'a> Cur<'a> {
        fn word(&mut self) -> Option<u32> {
            let (head, tail) = self.0.split_at_checked(4)?;
            self.0 = tail;
            Some(u32::from_le_bytes(head.try_into().ok()?))
        }
        fn float(&mut self) -> Option<f32> {
            let v = f32::from_bits(self.word()?);
            v.is_finite().then_some(v)
        }
        fn rem(&self) -> usize {
            self.0.len()
        }
    }

    let mut r = Cur(bytes);
    let v_count = usize::try_from(r.word()?).ok()?;
    let s_count = usize::try_from(r.word()?).ok()?;
    let r_count = usize::try_from(r.word()?).ok()?;
    // Minimum wire size per entry: vertex = 12 B, segment = 28 B, region header = 8 B (`packed` + `loop_count`).
    if v_count
        .checked_mul(12)?
        .checked_add(s_count.checked_mul(28)?)?
        .checked_add(r_count.checked_mul(8)?)?
        > r.rem()
    {
        return None;
    }

    let mut vertices = Vec::with_capacity(v_count);
    for _ in 0..v_count {
        vertices.push(VectorVertex {
            style: r.word()?,
            x: r.float()?,
            y: r.float()?,
        });
    }

    let mut segments = Vec::with_capacity(s_count);
    for _ in 0..s_count {
        let style = r.word()?;
        let start = r.word()?;
        let ax = r.float()?;
        let ay = r.float()?;
        let end = r.word()?;
        let bx = r.float()?;
        let by = r.float()?;
        if start as usize >= v_count || end as usize >= v_count {
            return None;
        }
        segments.push(VectorSegment {
            style,
            start,
            tangent_start: (ax, ay),
            end,
            tangent_end: (bx, by),
        });
    }

    let mut regions = Vec::with_capacity(r_count);
    for _ in 0..r_count {
        let packed = r.word()?;
        let loop_count = usize::try_from(r.word()?).ok()?;
        if loop_count.checked_mul(4)? > r.rem() {
            return None;
        }
        let mut loops = Vec::with_capacity(loop_count);
        for _ in 0..loop_count {
            let edge_count = usize::try_from(r.word()?).ok()?;
            if edge_count.checked_mul(4)? > r.rem() {
                return None;
            }
            let mut edges = Vec::with_capacity(edge_count);
            for _ in 0..edge_count {
                let idx = r.word()?;
                if idx as usize >= s_count {
                    return None;
                }
                edges.push(idx);
            }
            loops.push(edges);
        }
        regions.push(VectorRegion {
            winding_nonzero: (packed & 1) != 0,
            style_id: packed >> 1,
            loops,
        });
    }
    (r.rem() == 0).then_some(VectorNetwork {
        vertices,
        segments,
        regions,
    })
}

/// Validates `net` so its encoded `vectorNetworkBlob` always decodes cleanly and every region
/// loop forms a closed, finite path.
pub fn validate_vector_network(net: &VectorNetwork) -> Result<(), FigError> {
    u32::try_from(net.vertices.len())
        .map_err(|_| FigError::malformed("vector network has too many vertices"))?;
    u32::try_from(net.segments.len())
        .map_err(|_| FigError::malformed("vector network has too many segments"))?;
    u32::try_from(net.regions.len())
        .map_err(|_| FigError::malformed("vector network has too many regions"))?;

    for (i, v) in net.vertices.iter().enumerate() {
        if !v.x.is_finite() || !v.y.is_finite() {
            return Err(FigError::malformed(format!(
                "vector network vertex {i} has non-finite coordinates"
            )));
        }
    }

    let v_len = net.vertices.len();
    for (i, s) in net.segments.iter().enumerate() {
        let start = usize::try_from(s.start)
            .ok()
            .filter(|&idx| idx < v_len)
            .ok_or_else(|| {
                FigError::malformed(format!(
                    "vector network segment {i} start vertex {} out of range (len {v_len})",
                    s.start
                ))
            })?;
        let end = usize::try_from(s.end)
            .ok()
            .filter(|&idx| idx < v_len)
            .ok_or_else(|| {
                FigError::malformed(format!(
                    "vector network segment {i} end vertex {} out of range (len {v_len})",
                    s.end
                ))
            })?;
        if !s.tangent_start.0.is_finite()
            || !s.tangent_start.1.is_finite()
            || !s.tangent_end.0.is_finite()
            || !s.tangent_end.1.is_finite()
        {
            return Err(FigError::malformed(format!(
                "vector network segment {i} has non-finite tangent"
            )));
        }
        let p = net.vertices[start];
        let q = net.vertices[end];
        if !(p.x + s.tangent_start.0).is_finite()
            || !(p.y + s.tangent_start.1).is_finite()
            || !(q.x + s.tangent_end.0).is_finite()
            || !(q.y + s.tangent_end.1).is_finite()
        {
            return Err(FigError::malformed(format!(
                "vector network segment {i} control point overflows f32"
            )));
        }
    }

    let s_len = net.segments.len();
    for (ri, r) in net.regions.iter().enumerate() {
        if r.style_id > (u32::MAX >> 1) {
            return Err(FigError::malformed(format!(
                "vector network region {ri} style_id {} exceeds 31-bit field",
                r.style_id
            )));
        }
        if r.loops.is_empty() {
            return Err(FigError::malformed(format!(
                "vector network region {ri} has no loops"
            )));
        }
        u32::try_from(r.loops.len()).map_err(|_| {
            FigError::malformed(format!("vector network region {ri} has too many loops"))
        })?;
        for (li, lp) in r.loops.iter().enumerate() {
            u32::try_from(lp.len()).map_err(|_| {
                FigError::malformed(format!(
                    "vector network region {ri} loop {li} has too many edges"
                ))
            })?;
            if lp.is_empty() {
                return Err(FigError::malformed(format!(
                    "vector network region {ri} loop {li} is empty"
                )));
            }
            for &idx in lp {
                if usize::try_from(idx).ok().filter(|&i| i < s_len).is_none() {
                    return Err(FigError::malformed(format!(
                        "vector network region {ri} loop {li} segment {idx} out of range (len {s_len})"
                    )));
                }
            }
            if directed_loop_cmds(net, lp, false)
                .or_else(|| directed_loop_cmds(net, lp, true))
                .is_none()
            {
                return Err(FigError::malformed(format!(
                    "vector network region {ri} loop {li} is not a closed connected cycle"
                )));
            }
        }
    }
    Ok(())
}

/// Validates and encodes a [`VectorNetwork`] into Figma's binary `vectorNetworkBlob` format.
pub fn try_encode_vector_network(net: &VectorNetwork) -> Result<Vec<u8>, FigError> {
    validate_vector_network(net)?;
    Ok(encode_vector_network_unchecked(net))
}

/// Encodes an already-validated [`VectorNetwork`] into Figma's binary `vectorNetworkBlob` format.
fn encode_vector_network_unchecked(net: &VectorNetwork) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + net.vertices.len() * 12 + net.segments.len() * 28);
    let push_u32 = |buf: &mut Vec<u8>, v: u32| buf.extend_from_slice(&v.to_le_bytes());
    let push_f32 = |buf: &mut Vec<u8>, v: f32| buf.extend_from_slice(&v.to_le_bytes());

    push_u32(&mut out, net.vertices.len() as u32);
    push_u32(&mut out, net.segments.len() as u32);
    push_u32(&mut out, net.regions.len() as u32);

    for v in &net.vertices {
        push_u32(&mut out, v.style);
        push_f32(&mut out, v.x);
        push_f32(&mut out, v.y);
    }
    for s in &net.segments {
        push_u32(&mut out, s.style);
        push_u32(&mut out, s.start);
        push_f32(&mut out, s.tangent_start.0);
        push_f32(&mut out, s.tangent_start.1);
        push_u32(&mut out, s.end);
        push_f32(&mut out, s.tangent_end.0);
        push_f32(&mut out, s.tangent_end.1);
    }
    for r in &net.regions {
        let packed = (r.style_id << 1) | u32::from(r.winding_nonzero);
        push_u32(&mut out, packed);
        push_u32(&mut out, r.loops.len() as u32);
        for lp in &r.loops {
            push_u32(&mut out, lp.len() as u32);
            for &idx in lp {
                push_u32(&mut out, idx);
            }
        }
    }
    out
}

/// One filled region outline extracted from a [`VectorNetwork`], keeping its winding rule
/// and `style_id` (index into `vectorData.styleOverrideTable`, `0` = node fill).
#[derive(Clone, Debug, PartialEq)]
pub struct VectorFillPath {
    pub cmds: Vec<PathCmd>,
    pub even_odd: bool,
    pub style_id: u32,
}

/// Converts [`VectorNetwork`] regions into filled outlines preserving winding rule and `style_id`.
pub fn network_to_styled_fill_paths(net: &VectorNetwork) -> Vec<VectorFillPath> {
    let mut out = Vec::new();
    for region in &net.regions {
        let mut cmds = Vec::new();
        let mut ok = true;
        for lp in &region.loops {
            if let Some(loop_cmds) =
                directed_loop_cmds(net, lp, false).or_else(|| directed_loop_cmds(net, lp, true))
            {
                cmds.extend(loop_cmds);
            } else {
                ok = false;
                break;
            }
        }
        if ok && !cmds.is_empty() {
            out.push(VectorFillPath {
                cmds,
                even_odd: !region.winding_nonzero,
                style_id: region.style_id,
            });
        }
    }
    out
}

/// Converts [`VectorNetwork`] regions into `(PathCmds, even_odd)` outlines for `fillGeometry`.
pub fn network_to_fill_paths(net: &VectorNetwork) -> Vec<(Vec<PathCmd>, bool)> {
    network_to_styled_fill_paths(net)
        .into_iter()
        .map(|p| (p.cmds, p.even_odd))
        .collect()
}

/// Computes exact axis-aligned bounds `(min_x, min_y, max_x, max_y)` of `net`, including cubic
/// Bezier extrema of curved segments.
pub fn network_bounds(net: &VectorNetwork) -> Option<(f64, f64, f64, f64)> {
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut any = false;
    for v in &net.vertices {
        let (x, y) = (f64::from(v.x), f64::from(v.y));
        if x.is_finite() && y.is_finite() {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            any = true;
        }
    }
    for s in &net.segments {
        if s.tangent_start == (0.0, 0.0) && s.tangent_end == (0.0, 0.0) {
            continue;
        }
        let Some(p) = net.vertices.get(s.start as usize) else {
            continue;
        };
        let Some(q) = net.vertices.get(s.end as usize) else {
            continue;
        };
        let (px, py) = (f64::from(p.x), f64::from(p.y));
        let (qx, qy) = (f64::from(q.x), f64::from(q.y));
        let (c1x, c1y) = (
            px + f64::from(s.tangent_start.0),
            py + f64::from(s.tangent_start.1),
        );
        let (c2x, c2y) = (
            qx + f64::from(s.tangent_end.0),
            qy + f64::from(s.tangent_end.1),
        );
        cubic_extrema(px, c1x, c2x, qx, &mut min_x, &mut max_x);
        cubic_extrema(py, c1y, c2y, qy, &mut min_y, &mut max_y);
    }
    (any && min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite())
        .then_some((min_x, min_y, max_x, max_y))
}

fn directed_loop_cmds(
    net: &VectorNetwork,
    edges: &[u32],
    reverse_first: bool,
) -> Option<Vec<PathCmd>> {
    let first = net.segments.get(*edges.first()? as usize)?;
    let start_v = if reverse_first {
        first.end
    } else {
        first.start
    };
    let mut cur = start_v;
    let v0 = net.vertices.get(cur as usize)?;
    let mut cmds = vec![PathCmd::MoveTo(v0.x, v0.y)];
    for &eidx in edges {
        let seg = net.segments.get(eidx as usize)?;
        let (next_v, ta, tb) = if seg.start == cur {
            (seg.end, seg.tangent_start, seg.tangent_end)
        } else if seg.end == cur {
            (seg.start, seg.tangent_end, seg.tangent_start)
        } else {
            return None;
        };
        let p = net.vertices.get(cur as usize)?;
        let q = net.vertices.get(next_v as usize)?;
        if ta == (0.0, 0.0) && tb == (0.0, 0.0) {
            cmds.push(PathCmd::LineTo(q.x, q.y));
        } else {
            cmds.push(PathCmd::CubicTo(
                p.x + ta.0,
                p.y + ta.1,
                q.x + tb.0,
                q.y + tb.1,
                q.x,
                q.y,
            ));
        }
        cur = next_v;
    }
    if cur != start_v {
        return None;
    }
    cmds.push(PathCmd::Close);
    Some(cmds)
}

/// 2D Boolean operation kind matching Figma `BooleanOperation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BooleanOp {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

impl BooleanOp {
    pub fn from_figma_name(s: &str) -> Option<Self> {
        match s {
            "UNION" => Some(Self::Union),
            "SUBTRACT" => Some(Self::Subtract),
            "INTERSECT" => Some(Self::Intersect),
            "EXCLUDE" | "XOR" => Some(Self::Exclude),
            _ => None,
        }
    }

    pub fn figma_name(self) -> &'static str {
        match self {
            Self::Union => "UNION",
            Self::Subtract => "SUBTRACT",
            Self::Intersect => "INTERSECT",
            Self::Exclude => "XOR",
        }
    }
}

type Pt = (f64, f64);

/// One closed path operand and its winding rule (`even_odd: true` = `ODD`, `false` = `NONZERO`).
#[derive(Clone, Debug, PartialEq)]
pub struct BooleanPath {
    pub cmds: Vec<PathCmd>,
    pub even_odd: bool,
}

struct FlattenedPart {
    rings: Vec<Vec<Pt>>,
    even_odd: bool,
}

/// Computes a 2D boolean combination (`UNION`, `SUBTRACT`, `INTERSECT`, `EXCLUDE`) of two
/// closed `NONZERO`-wound path command streams, returning `NONZERO`-wound boundary loops
/// (outer contours positive signed area, holes negative signed area).
pub fn boolean_combine(op: BooleanOp, path_a: &[PathCmd], path_b: &[PathCmd]) -> Vec<PathCmd> {
    boolean_combine_paths(
        op,
        &[BooleanPath {
            cmds: path_a.to_vec(),
            even_odd: false,
        }],
        &[BooleanPath {
            cmds: path_b.to_vec(),
            even_odd: false,
        }],
    )
}

/// Computes a 2D boolean combination of two multi-path, winding-rule-aware shapes, returning
/// canonical `NONZERO`-wound boundary loops (preserving holes, multi-contour inputs, and
/// `ODD`/`NONZERO` winding semantics).
pub fn boolean_combine_paths(
    op: BooleanOp,
    paths_a: &[BooleanPath],
    paths_b: &[BooleanPath],
) -> Vec<PathCmd> {
    let flatten_shape = |paths: &[BooleanPath]| -> Vec<FlattenedPart> {
        paths
            .iter()
            .filter_map(|p| {
                let rings = flatten_to_rings(&p.cmds, 16);
                (!rings.is_empty()).then_some(FlattenedPart {
                    rings,
                    even_odd: p.even_odd,
                })
            })
            .collect()
    };

    let shape_a = flatten_shape(paths_a);
    let shape_b = flatten_shape(paths_b);

    let pass_through = |paths: &[BooleanPath], parts: &[FlattenedPart]| -> Vec<PathCmd> {
        if paths.len() == 1 && !paths[0].even_odd {
            paths[0].cmds.clone()
        } else {
            stitch_loops(&canonical_boundary_edges(parts))
        }
    };

    if shape_a.is_empty() {
        return match op {
            BooleanOp::Union | BooleanOp::Exclude => pass_through(paths_b, &shape_b),
            BooleanOp::Subtract | BooleanOp::Intersect => Vec::new(),
        };
    }
    if shape_b.is_empty() {
        return match op {
            BooleanOp::Union | BooleanOp::Subtract | BooleanOp::Exclude => {
                pass_through(paths_a, &shape_a)
            }
            BooleanOp::Intersect => Vec::new(),
        };
    }

    let edges_a = canonical_boundary_edges(&shape_a);
    let edges_b = canonical_boundary_edges(&shape_b);

    let sub_a = split_against(&edges_a, &edges_b);
    let sub_b = split_against(&edges_b, &edges_a);

    let mut kept: Vec<(Pt, Pt)> = Vec::new();
    for (p0, p1) in sub_a {
        classify_subsegment(op, true, p0, p1, &shape_b, &mut kept);
    }
    for (p0, p1) in sub_b {
        classify_subsegment(op, false, p0, p1, &shape_a, &mut kept);
    }

    stitch_loops(&kept)
}

/// Decides whether canonical boundary subsegment `(p0, p1)` (interior on left `+n`, exterior on
/// right `-n`; from `A` when `from_a`, else `B`) belongs on the boolean result boundary,
/// breaking collinear boundary overlaps using both sides of the edge normal.
fn classify_subsegment(
    op: BooleanOp,
    from_a: bool,
    p0: Pt,
    p1: Pt,
    other_shape: &[FlattenedPart],
    kept: &mut Vec<(Pt, Pt)>,
) {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len = dx.hypot(dy);
    if len < 1e-7 {
        return;
    }
    let mid = ((p0.0 + p1.0) * 0.5, (p0.1 + p1.1) * 0.5);
    let eps = (len * 0.01).clamp(1e-9, 1e-4);
    let (nx, ny) = (-dy / len * eps, dx / len * eps);
    let in_left = point_in_shape((mid.0 + nx, mid.1 + ny), other_shape);
    let in_right = point_in_shape((mid.0 - nx, mid.1 - ny), other_shape);
    match (in_left, in_right) {
        // Strictly inside `other`.
        (true, true) => match op {
            BooleanOp::Intersect => kept.push((p0, p1)),
            BooleanOp::Subtract if !from_a => kept.push((p1, p0)),
            BooleanOp::Exclude => kept.push((p1, p0)),
            _ => {}
        },
        // Strictly outside `other`.
        (false, false) => match op {
            BooleanOp::Union | BooleanOp::Exclude => kept.push((p0, p1)),
            BooleanOp::Subtract if from_a => kept.push((p0, p1)),
            _ => {}
        },
        // Co-oriented shared boundary: keep once (from A) for Union and Intersect.
        (true, false) => {
            if from_a && matches!(op, BooleanOp::Union | BooleanOp::Intersect) {
                kept.push((p0, p1));
            }
        }
        // Anti-oriented touching seam: keep once (from A) for Subtract only.
        (false, true) => {
            if from_a && op == BooleanOp::Subtract {
                kept.push((p0, p1));
            }
        }
    }
}

fn flatten_to_rings(cmds: &[PathCmd], steps: usize) -> Vec<Vec<Pt>> {
    let mut rings = Vec::new();
    let mut cur: Vec<Pt> = Vec::new();
    let mut last = (0.0, 0.0);
    for &cmd in cmds {
        match cmd {
            PathCmd::MoveTo(x, y) => {
                if cur.len() >= 3 {
                    rings.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                last = (f64::from(x), f64::from(y));
                cur.push(last);
            }
            PathCmd::LineTo(x, y) => {
                last = (f64::from(x), f64::from(y));
                cur.push(last);
            }
            PathCmd::QuadTo(cx, cy, x, y) => {
                let p0 = last;
                let c = (f64::from(cx), f64::from(cy));
                let p2 = (f64::from(x), f64::from(y));
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let mt = 1.0 - t;
                    cur.push((
                        mt * mt * p0.0 + 2.0 * mt * t * c.0 + t * t * p2.0,
                        mt * mt * p0.1 + 2.0 * mt * t * c.1 + t * t * p2.1,
                    ));
                }
                last = p2;
            }
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                let p0 = last;
                let c1 = (f64::from(c1x), f64::from(c1y));
                let c2 = (f64::from(c2x), f64::from(c2y));
                let p3 = (f64::from(x), f64::from(y));
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let mt = 1.0 - t;
                    cur.push((
                        mt.powi(3) * p0.0
                            + 3.0 * mt * mt * t * c1.0
                            + 3.0 * mt * t * t * c2.0
                            + t.powi(3) * p3.0,
                        mt.powi(3) * p0.1
                            + 3.0 * mt * mt * t * c1.1
                            + 3.0 * mt * t * t * c2.1
                            + t.powi(3) * p3.1,
                    ));
                }
                last = p3;
            }
            PathCmd::Close => {
                if cur.len() >= 3 {
                    rings.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
    }
    if cur.len() >= 3 {
        rings.push(cur);
    }
    rings
}

fn seg_bbox_overlaps(a0: Pt, a1: Pt, b0: Pt, b1: Pt) -> bool {
    const PAD: f64 = 1e-6;
    let a_min_x = a0.0.min(a1.0) - PAD;
    let a_max_x = a0.0.max(a1.0) + PAD;
    let b_min_x = b0.0.min(b1.0);
    let b_max_x = b0.0.max(b1.0);
    if a_max_x < b_min_x || b_max_x < a_min_x {
        return false;
    }
    let a_min_y = a0.1.min(a1.1) - PAD;
    let a_max_y = a0.1.max(a1.1) + PAD;
    let b_min_y = b0.1.min(b1.1);
    let b_max_y = b0.1.max(b1.1);
    !(a_max_y < b_min_y || b_max_y < a_min_y)
}

/// Extracts canonical boundary edges of `parts` where every edge has the filled interior on its
/// left (`+n = (-dy, dx)`) and the exterior on its right (`-n`). Outer contours end up with
/// positive signed area, holes with negative signed area, and internal/duplicate edges are dropped.
fn canonical_boundary_edges(parts: &[FlattenedPart]) -> Vec<(Pt, Pt)> {
    let mut raw: Vec<(Pt, Pt)> = Vec::new();
    for part in parts {
        for ring in &part.rings {
            for i in 0..ring.len() {
                let p0 = ring[i];
                let p1 = ring[(i + 1) % ring.len()];
                if (p1.0 - p0.0).hypot(p1.1 - p0.1) > 1e-7 {
                    raw.push((p0, p1));
                }
            }
        }
    }

    let sub = split_self(&raw);
    let mut out: Vec<(Pt, Pt)> = Vec::new();
    for (p0, p1) in sub {
        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        let len = dx.hypot(dy);
        if len < 1e-7 {
            continue;
        }
        let mid = ((p0.0 + p1.0) * 0.5, (p0.1 + p1.1) * 0.5);
        let eps = (len * 0.01).clamp(1e-9, 1e-4);
        let (nx, ny) = (-dy / len * eps, dx / len * eps);
        let in_left = point_in_shape((mid.0 + nx, mid.1 + ny), parts);
        let in_right = point_in_shape((mid.0 - nx, mid.1 - ny), parts);
        let oriented = match (in_left, in_right) {
            (true, false) => (p0, p1),
            (false, true) => (p1, p0),
            _ => continue,
        };
        let duplicate = out.iter().any(|&(e0, e1)| {
            (e0.0 - oriented.0.0).hypot(e0.1 - oriented.0.1) < 1e-5
                && (e1.0 - oriented.1.0).hypot(e1.1 - oriented.1.1) < 1e-5
        });
        if !duplicate {
            out.push(oriented);
        }
    }
    out
}

fn split_self(edges: &[(Pt, Pt)]) -> Vec<(Pt, Pt)> {
    let mut out = Vec::with_capacity(edges.len());
    for (i, &(a0, a1)) in edges.iter().enumerate() {
        let mut ts = vec![0.0_f64, 1.0_f64];
        for (j, &(b0, b1)) in edges.iter().enumerate() {
            if i == j || !seg_bbox_overlaps(a0, a1, b0, b1) {
                continue;
            }
            if let Some(t) = seg_intersect_t(a0, a1, b0, b1) {
                if t > 1e-7 && t < 1.0 - 1e-7 {
                    ts.push(t);
                }
            } else {
                push_collinear_split_ts(a0, a1, b0, b1, &mut ts);
            }
        }
        ts.sort_by(|x, y| x.total_cmp(y));
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-7);
        for pair in ts.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            let p0 = (a0.0 + (a1.0 - a0.0) * t0, a0.1 + (a1.1 - a0.1) * t0);
            let p1 = (a0.0 + (a1.0 - a0.0) * t1, a0.1 + (a1.1 - a0.1) * t1);
            out.push((p0, p1));
        }
    }
    out
}

fn split_against(subject: &[(Pt, Pt)], clip: &[(Pt, Pt)]) -> Vec<(Pt, Pt)> {
    let mut out = Vec::new();
    for &(a0, a1) in subject {
        let mut ts = vec![0.0_f64, 1.0_f64];
        for &(b0, b1) in clip {
            if !seg_bbox_overlaps(a0, a1, b0, b1) {
                continue;
            }
            if let Some(t) = seg_intersect_t(a0, a1, b0, b1) {
                if t > 1e-7 && t < 1.0 - 1e-7 {
                    ts.push(t);
                }
            } else {
                push_collinear_split_ts(a0, a1, b0, b1, &mut ts);
            }
        }
        ts.sort_by(|x, y| x.total_cmp(y));
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-7);
        for pair in ts.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            let p0 = (a0.0 + (a1.0 - a0.0) * t0, a0.1 + (a1.1 - a0.1) * t0);
            let p1 = (a0.0 + (a1.0 - a0.0) * t1, a0.1 + (a1.1 - a0.1) * t1);
            out.push((p0, p1));
        }
    }
    out
}

fn push_collinear_split_ts(a0: Pt, a1: Pt, b0: Pt, b1: Pt, ts: &mut Vec<f64>) {
    let (rx, ry) = (a1.0 - a0.0, a1.1 - a0.1);
    let len2 = rx * rx + ry * ry;
    if len2 <= 1e-14 {
        return;
    }
    let len = len2.sqrt();
    for pt in [b0, b1] {
        let (px, py) = (pt.0 - a0.0, pt.1 - a0.1);
        if (px * ry - py * rx).abs() / len < 1e-7 {
            let t = (px * rx + py * ry) / len2;
            if t > 1e-7 && t < 1.0 - 1e-7 {
                ts.push(t);
            }
        }
    }
}

fn seg_intersect_t(a0: Pt, a1: Pt, b0: Pt, b1: Pt) -> Option<f64> {
    let r = (a1.0 - a0.0, a1.1 - a0.1);
    let s = (b1.0 - b0.0, b1.1 - b0.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() < 1e-10 {
        return None;
    }
    let qp = (b0.0 - a0.0, b0.1 - a0.1);
    let t = (qp.0 * s.1 - qp.1 * s.0) / denom;
    let u = (qp.0 * r.1 - qp.1 * r.0) / denom;
    if (-1e-7..=1.0 + 1e-7).contains(&t) && (-1e-7..=1.0 + 1e-7).contains(&u) {
        Some(t.clamp(0.0, 1.0))
    } else {
        None
    }
}

fn cross_2d(a: Pt, b: Pt, p: Pt) -> f64 {
    (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1)
}

fn point_in_part(p: Pt, part: &FlattenedPart) -> bool {
    if part.even_odd {
        let mut inside = false;
        for ring in &part.rings {
            let n = ring.len();
            for i in 0..n {
                let (x0, y0) = ring[i];
                let (x1, y1) = ring[(i + 1) % n];
                if ((y0 > p.1) != (y1 > p.1)) && (p.0 < (x1 - x0) * (p.1 - y0) / (y1 - y0) + x0) {
                    inside = !inside;
                }
            }
        }
        inside
    } else {
        let mut wn = 0i32;
        for ring in &part.rings {
            let n = ring.len();
            for i in 0..n {
                let a = ring[i];
                let b = ring[(i + 1) % n];
                if a.1 <= p.1 {
                    if b.1 > p.1 && cross_2d(a, b, p) > 0.0 {
                        wn += 1;
                    }
                } else if b.1 <= p.1 && cross_2d(a, b, p) < 0.0 {
                    wn -= 1;
                }
            }
        }
        wn != 0
    }
}

fn point_in_shape(p: Pt, parts: &[FlattenedPart]) -> bool {
    parts.iter().any(|part| point_in_part(p, part))
}

fn stitch_loops(segments: &[(Pt, Pt)]) -> Vec<PathCmd> {
    let mut used = vec![false; segments.len()];
    let mut cmds = Vec::new();
    for start_idx in 0..segments.len() {
        if used[start_idx] {
            continue;
        }
        used[start_idx] = true;
        let (first_pt, mut cur_pt) = segments[start_idx];
        let mut prev_pt = first_pt;
        let mut ring = vec![first_pt, cur_pt];
        loop {
            if (cur_pt.0 - first_pt.0).hypot(cur_pt.1 - first_pt.1) < 1e-3 && ring.len() >= 4 {
                break;
            }
            let in_dir = (cur_pt.0 - prev_pt.0, cur_pt.1 - prev_pt.1);
            let mut best: Option<(usize, f64, f64)> = None;
            for (j, &(s0, s1)) in segments.iter().enumerate() {
                if used[j] {
                    continue;
                }
                let d = (s0.0 - cur_pt.0).hypot(s0.1 - cur_pt.1);
                if d >= 1e-2 {
                    continue;
                }
                let out_dir = (s1.0 - s0.0, s1.1 - s0.1);
                let cross = in_dir.0 * out_dir.1 - in_dir.1 * out_dir.0;
                let dot = in_dir.0 * out_dir.0 + in_dir.1 * out_dir.1;
                let turn = cross.atan2(dot);
                let better = match best {
                    None => true,
                    Some((_, bd, bturn)) => {
                        if d < 1e-5 && bd < 1e-5 {
                            // At a shared pinch vertex, turn toward the left interior to keep
                            // contours simple instead of forming figure-eights.
                            turn > bturn
                        } else {
                            d < bd
                        }
                    }
                };
                if better {
                    best = Some((j, d, turn));
                }
            }
            let Some((next_j, _, _)) = best else {
                break;
            };
            used[next_j] = true;
            prev_pt = cur_pt;
            cur_pt = segments[next_j].1;
            ring.push(cur_pt);
        }
        if ring.len() >= 3 {
            cmds.push(PathCmd::MoveTo(ring[0].0 as f32, ring[0].1 as f32));
            for &(x, y) in &ring[1..ring.len() - 1] {
                cmds.push(PathCmd::LineTo(x as f32, y as f32));
            }
            cmds.push(PathCmd::Close);
        }
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::geometry::rounded_rect_commands;

    #[test]
    fn vector_network_roundtrips_byte_for_byte() {
        let net = VectorNetwork {
            vertices: vec![
                VectorVertex {
                    style: 0,
                    x: 0.0,
                    y: 0.0,
                },
                VectorVertex {
                    style: 0,
                    x: 100.0,
                    y: 0.0,
                },
                VectorVertex {
                    style: 0,
                    x: 50.0,
                    y: 80.0,
                },
            ],
            segments: vec![
                VectorSegment {
                    style: 0,
                    start: 0,
                    tangent_start: (0.0, 0.0),
                    end: 1,
                    tangent_end: (0.0, 0.0),
                },
                VectorSegment {
                    style: 0,
                    start: 1,
                    tangent_start: (0.0, 0.0),
                    end: 2,
                    tangent_end: (0.0, 0.0),
                },
                VectorSegment {
                    style: 0,
                    start: 2,
                    tangent_start: (0.0, 0.0),
                    end: 0,
                    tangent_end: (0.0, 0.0),
                },
            ],
            regions: vec![VectorRegion {
                winding_nonzero: true,
                style_id: 0,
                loops: vec![vec![0, 1, 2]],
            }],
        };
        let bytes = try_encode_vector_network(&net).expect("encode");
        let decoded = decode_vector_network(&bytes).expect("decode");
        assert_eq!(decoded, net);
        assert_eq!(
            try_encode_vector_network(&decoded).expect("re-encode"),
            bytes
        );
    }

    #[test]
    fn boolean_union_subtract_intersect_produce_closed_loops() {
        let a = rounded_rect_commands(100.0, 100.0, [0.0; 4], 0.0);
        let mut b = rounded_rect_commands(100.0, 100.0, [0.0; 4], 0.0);
        for cmd in &mut b {
            match cmd {
                PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => {
                    *x += 50.0;
                    *y += 50.0;
                }
                _ => {}
            }
        }
        for op in [
            BooleanOp::Union,
            BooleanOp::Subtract,
            BooleanOp::Intersect,
            BooleanOp::Exclude,
        ] {
            assert_eq!(BooleanOp::from_figma_name(op.figma_name()), Some(op));
            let res = boolean_combine(op, &a, &b);
            assert!(!res.is_empty(), "{op:?} produced empty output");
            assert_eq!(res.last(), Some(&PathCmd::Close));
        }
    }
}
