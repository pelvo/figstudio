//! Lossless `vectorNetworkBlob` codec and the boolean operation kinds (computed in
//! [`crate::edit::outline`]).

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
            // The file format's name (Figma's panel says Exclude).
            Self::Exclude => "XOR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
