//! Checked vector-network decoding shared by fill reconstruction and stroke alignment.
//!
//! Blob layout: `u32` vertex, segment and region counts; 12-byte vertices (style, x, y);
//! 28-byte segments (style, start, tangent, end, tangent); per region a packed word (bit 0 =
//! non-zero winding, upper bits = style id), a loop count, then each loop's length and indices.
use figstudio_fig::Val;
use vello_cpu::kurbo::{BezPath, Point, Vec2};
use vello_cpu::peniko::Fill;

use crate::geom::{GeomPath, size};

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(len)?;
        self.0 = tail;
        Some(head)
    }

    fn word(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn count(&mut self) -> Option<usize> {
        usize::try_from(self.word()?).ok()
    }

    fn float(&mut self) -> Option<f64> {
        let value = f64::from(f32::from_bits(self.word()?));
        value.is_finite().then_some(value)
    }

    fn point(&mut self) -> Option<Point> {
        Some(Point::new(self.float()?, self.float()?))
    }
}

struct Segment {
    start: usize,
    end: usize,
    a: Vec2,
    b: Vec2,
    style: u32,
}

struct Region {
    packed: u32,
    loops: Vec<Vec<usize>>,
    /// `None` when the loops do not form closed, connected outlines.
    path: Option<BezPath>,
}

struct Network {
    vertices: Vec<(Point, u32)>,
    segments: Vec<Segment>,
    regions: Vec<Region>,
}

fn blob<'a>(data: &Val, blobs: &'a [Vec<u8>]) -> Option<&'a [u8]> {
    let index = data.path(&["vectorData", "vectorNetworkBlob"])?.as_u32()?;
    Some(blobs.get(usize::try_from(index).ok()?)?.as_slice())
}

fn parse(bytes: &[u8]) -> Option<Network> {
    let mut r = Reader(bytes);
    let (vertices, segments, regions) = (r.count()?, r.count()?, r.count()?);
    let vertex_bytes = r.take(vertices.checked_mul(12)?)?;
    let segment_bytes = r.take(segments.checked_mul(28)?)?;
    if regions.checked_mul(8)? > r.0.len() {
        return None;
    }
    let mut net = Network {
        vertices: Vec::new(),
        segments: Vec::new(),
        regions: Vec::new(),
    };
    let mut v = Reader(vertex_bytes);
    for _ in 0..vertices {
        let style = v.word()?;
        net.vertices.push((v.point()?, style));
    }
    let mut s = Reader(segment_bytes);
    for _ in 0..segments {
        let style = s.word()?;
        let start = s.count()?;
        let a = s.point()?.to_vec2();
        let end = s.count()?;
        let b = s.point()?.to_vec2();
        net.vertices.get(start)?;
        net.vertices.get(end)?;
        net.segments.push(Segment {
            start,
            end,
            a,
            b,
            style,
        });
    }
    for _ in 0..regions {
        let packed = r.word()?;
        let count = r.count()?;
        if count.checked_mul(4)? > r.0.len() {
            return None;
        }
        let mut loops = Vec::new();
        let mut path = Some(BezPath::new());
        for _ in 0..count {
            let count = r.count()?;
            let mut indices = Reader(r.take(count.checked_mul(4)?)?);
            let mut edges = Vec::new();
            for _ in 0..count {
                let index = indices.count()?;
                net.segments.get(index)?;
                edges.push(index);
            }
            path = path.and_then(|mut all| {
                all.extend(loop_path(&net, &edges)?);
                Some(all)
            });
            loops.push(edges);
        }
        net.regions.push(Region {
            packed,
            loops,
            path,
        });
    }
    r.0.is_empty().then_some(net)
}

fn loop_path(net: &Network, edges: &[usize]) -> Option<BezPath> {
    directed_loop(net, edges, false).or_else(|| directed_loop(net, edges, true))
}

fn directed_loop(net: &Network, edges: &[usize], reverse: bool) -> Option<BezPath> {
    let first = net.segments.get(*edges.first()?)?;
    let start = if reverse { first.end } else { first.start };
    let mut current = start;
    let mut path = BezPath::new();
    path.move_to(net.vertices.get(start)?.0);
    for &index in edges {
        let s = net.segments.get(index)?;
        let (end, a, b) = if s.start == current {
            (s.end, s.a, s.b)
        } else if s.end == current {
            (s.start, s.b, s.a)
        } else {
            return None;
        };
        let p = net.vertices.get(current)?.0;
        let q = net.vertices.get(end)?.0;
        if a == Vec2::ZERO && b == Vec2::ZERO {
            path.line_to(q);
        } else {
            path.curve_to(p + a, q + b, q);
        }
        current = end;
    }
    if current != start {
        return None;
    }
    path.close_path();
    Some(path)
}

pub(crate) fn closed(data: &Val, blobs: &[Vec<u8>]) -> bool {
    let Some(net) = blob(data, blobs).and_then(parse) else {
        return false;
    };
    let mut used = vec![false; net.segments.len()];
    for region in &net.regions {
        for edges in &region.loops {
            for &index in edges {
                let Some(slot) = used.get_mut(index) else {
                    return false;
                };
                *slot = true;
            }
        }
    }
    !used.is_empty() && used.iter().all(|&s| s)
}

pub(crate) fn fills(
    data: &Val,
    blobs: &[Vec<u8>],
    unsupported: &mut usize,
) -> Option<Vec<GeomPath>> {
    let net = parse(blob(data, blobs)?)?;
    let normalized = data
        .path(&["vectorData", "normalizedSize"])
        .unwrap_or(data.get("size")?);
    let (w, h) = size(data);
    if !w.is_finite() || !h.is_finite() {
        return None;
    }
    let (nw, nh) = (
        f64::from(normalized.num("x")?),
        f64::from(normalized.num("y")?),
    );
    if !nw.is_finite() || !nh.is_finite() || nw <= 0.0 || nh <= 0.0 {
        return None;
    }
    let scale = vello_cpu::kurbo::Affine::scale_non_uniform(w / nw, h / nh);
    let styles = data
        .path(&["vectorData", "styleOverrideTable"])
        .map(Val::items)
        .unwrap_or_default();
    let mut paths = Vec::new();
    let mut skipped = net.vertices.iter().filter(|v| v.1 != 0).count()
        + net.segments.iter().filter(|s| s.style != 0).count();
    for region in net.regions {
        let style = region.packed >> 1;
        if style != 0
            && !styles
                .iter()
                .any(|s| s.get("styleID").and_then(Val::as_u32) == Some(style))
        {
            skipped += 1;
        }
        match region.path {
            Some(path) if !path.elements().is_empty() => {
                let path = scale * path;
                if !path.is_finite() {
                    skipped += 1;
                    continue;
                }
                paths.push(GeomPath {
                    path,
                    rule: if region.packed & 1 == 0 {
                        Fill::EvenOdd
                    } else {
                        Fill::NonZero
                    },
                    style,
                });
            }
            Some(_) => {}
            None => skipped += 1,
        }
    }
    // shortcut: vertex corner radii and segment-scoped styles are not reconstructed; count them.
    *unsupported += skipped;
    Some(paths)
}
