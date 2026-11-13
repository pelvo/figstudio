//! Lossless `vectorNetworkBlob` codec and 2D polygon / flattened-Bezier Boolean operations
//! (`UNION`, `SUBTRACT`, `INTERSECT`, `EXCLUDE`).

use serde::{Deserialize, Serialize};

use crate::edit::geometry::PathCmd;

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
    if v_count
        .checked_mul(12)?
        .checked_add(s_count.checked_mul(28)?)?
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

/// Encodes a [`VectorNetwork`] into Figma's binary `vectorNetworkBlob` format.
pub fn encode_vector_network(net: &VectorNetwork) -> Vec<u8> {
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

/// Converts [`VectorNetwork`] regions into `(PathCmds, even_odd)` outlines for `fillGeometry`.
pub fn network_to_fill_paths(net: &VectorNetwork) -> Vec<(Vec<PathCmd>, bool)> {
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
            out.push((cmds, !region.winding_nonzero));
        }
    }
    out
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
            Self::Exclude => "EXCLUDE",
        }
    }
}

type Pt = (f64, f64);

/// Computes a 2D boolean combination (`UNION`, `SUBTRACT`, `INTERSECT`, `EXCLUDE`) of two
/// closed path command streams by subdividing curves into dense polylines, splitting all
/// intersecting directed segments, filtering sub-segments by interior point-in-polygon tests,
/// and stitching closed boundary loops.
pub fn boolean_combine(op: BooleanOp, path_a: &[PathCmd], path_b: &[PathCmd]) -> Vec<PathCmd> {
    let rings_a = flatten_to_rings(path_a, 16);
    let rings_b = flatten_to_rings(path_b, 16);
    if rings_a.is_empty() {
        return match op {
            BooleanOp::Union | BooleanOp::Exclude => path_b.to_vec(),
            BooleanOp::Subtract | BooleanOp::Intersect => Vec::new(),
        };
    }
    if rings_b.is_empty() {
        return match op {
            BooleanOp::Union | BooleanOp::Subtract | BooleanOp::Exclude => path_a.to_vec(),
            BooleanOp::Intersect => Vec::new(),
        };
    }

    let edges_a = oriented_ccw_edges(&rings_a);
    let edges_b = oriented_ccw_edges(&rings_b);

    let sub_a = split_against(&edges_a, &edges_b);
    let sub_b = split_against(&edges_b, &edges_a);

    let mut kept: Vec<(Pt, Pt)> = Vec::new();
    for (p0, p1) in sub_a {
        let mid = ((p0.0 + p1.0) * 0.5, (p0.1 + p1.1) * 0.5);
        // Slightly perturb along inward normal of A to break collinear ties cleanly.
        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        let len = dx.hypot(dy);
        if len < 1e-7 {
            continue;
        }
        let in_b = point_in_rings(mid, &rings_b);
        match op {
            BooleanOp::Union if !in_b => kept.push((p0, p1)),
            BooleanOp::Subtract if !in_b => kept.push((p0, p1)),
            BooleanOp::Intersect if in_b => kept.push((p0, p1)),
            BooleanOp::Exclude => {
                if in_b {
                    kept.push((p1, p0));
                } else {
                    kept.push((p0, p1));
                }
            }
            _ => {}
        }
    }

    for (p0, p1) in sub_b {
        let mid = ((p0.0 + p1.0) * 0.5, (p0.1 + p1.1) * 0.5);
        let len = (p1.0 - p0.0).hypot(p1.1 - p0.1);
        if len < 1e-7 {
            continue;
        }
        let in_a = point_in_rings(mid, &rings_a);
        match op {
            BooleanOp::Union if !in_a => kept.push((p0, p1)),
            BooleanOp::Subtract if in_a => kept.push((p1, p0)), // reversed hole/cut boundary!
            BooleanOp::Intersect if in_a => kept.push((p0, p1)),
            BooleanOp::Exclude => {
                if in_a {
                    kept.push((p1, p0));
                } else {
                    kept.push((p0, p1));
                }
            }
            _ => {}
        }
    }

    stitch_loops(&kept)
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

fn signed_area(ring: &[Pt]) -> f64 {
    let mut a = 0.0;
    for i in 0..ring.len() {
        let (x0, y0) = ring[i];
        let (x1, y1) = ring[(i + 1) % ring.len()];
        a += x0 * y1 - x1 * y0;
    }
    a * 0.5
}

fn oriented_ccw_edges(rings: &[Vec<Pt>]) -> Vec<(Pt, Pt)> {
    let mut out = Vec::new();
    for ring in rings {
        let mut pts = ring.clone();
        if signed_area(&pts) < 0.0 {
            pts.reverse();
        }
        for i in 0..pts.len() {
            let p0 = pts[i];
            let p1 = pts[(i + 1) % pts.len()];
            if (p1.0 - p0.0).hypot(p1.1 - p0.1) > 1e-7 {
                out.push((p0, p1));
            }
        }
    }
    out
}

fn split_against(subject: &[(Pt, Pt)], clip: &[(Pt, Pt)]) -> Vec<(Pt, Pt)> {
    let mut out = Vec::new();
    for &(a0, a1) in subject {
        let mut ts = vec![0.0_f64, 1.0_f64];
        for &(b0, b1) in clip {
            if let Some(t) = seg_intersect_t(a0, a1, b0, b1)
                && t > 1e-7
                && t < 1.0 - 1e-7
            {
                ts.push(t);
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

fn point_in_rings(p: Pt, rings: &[Vec<Pt>]) -> bool {
    let mut inside = false;
    for ring in rings {
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
        let mut ring = vec![first_pt, cur_pt];
        loop {
            if (cur_pt.0 - first_pt.0).hypot(cur_pt.1 - first_pt.1) < 1e-3 && ring.len() >= 4 {
                break;
            }
            let mut best: Option<(usize, f64)> = None;
            for (j, &(s0, _)) in segments.iter().enumerate() {
                if used[j] {
                    continue;
                }
                let d = (s0.0 - cur_pt.0).hypot(s0.1 - cur_pt.1);
                if d < 1e-2 && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((j, d));
                }
            }
            let Some((next_j, _)) = best else {
                break;
            };
            used[next_j] = true;
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
        let bytes = encode_vector_network(&net);
        let decoded = decode_vector_network(&bytes).expect("decode");
        assert_eq!(decoded, net);
        assert_eq!(encode_vector_network(&decoded), bytes);
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
            let res = boolean_combine(op, &a, &b);
            assert!(!res.is_empty(), "{op:?} produced empty output");
            assert_eq!(res.last(), Some(&PathCmd::Close));
        }
    }
}
