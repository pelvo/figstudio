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
    /// `None` when the loops do not form closed, connected outlines (or paths were not built).
    path: Option<BezPath>,
}

struct Network {
    vertices: Vec<(Point, u32)>,
    segments: Vec<Segment>,
    /// Filled only when paths are built; empty otherwise.
    regions: Vec<Region>,
    /// Whether each segment lies on some region loop.
    covered: Vec<bool>,
}

fn blob<'a>(data: &Val, blobs: &'a [Vec<u8>]) -> Option<&'a [u8]> {
    let index = data.path(&["vectorData", "vectorNetworkBlob"])?.as_u32()?;
    Some(blobs.get(usize::try_from(index).ok()?)?.as_slice())
}

/// Decodes and validates the blob. Region paths are only built when `build` is set.
fn parse(bytes: &[u8], build: bool) -> Option<Network> {
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
        covered: Vec::new(),
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
    net.covered = vec![false; net.segments.len()];
    let mut edges = Vec::new();
    for _ in 0..regions {
        let packed = r.word()?;
        let count = r.count()?;
        if count.checked_mul(4)? > r.0.len() {
            return None;
        }
        let mut path = build.then(BezPath::new);
        for _ in 0..count {
            let count = r.count()?;
            let mut indices = Reader(r.take(count.checked_mul(4)?)?);
            edges.clear();
            for _ in 0..count {
                let index = indices.count()?;
                *net.covered.get_mut(index)? = true;
                if build {
                    edges.push(index);
                }
            }
            if build {
                path = path.and_then(|mut all| {
                    all.extend(loop_path(&net, &edges)?);
                    Some(all)
                });
            }
        }
        if build {
            net.regions.push(Region { packed, path });
        }
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
    let Some(net) = blob(data, blobs).and_then(|b| parse(b, false)) else {
        return false;
    };
    !net.covered.is_empty() && net.covered.iter().all(|&s| s)
}

/// Whether `paints` holds a paint that would be drawn.
fn visible(paints: Option<&Val>) -> bool {
    paints.is_some_and(|p| {
        p.items()
            .iter()
            .any(|p| p.flag("visible") != Some(false) && p.num("opacity").unwrap_or(1.0) > 0.0)
    })
}

/// Whether the node, or a region style it carries, has a fill that could be drawn.
fn has_visible_fill(data: &Val) -> bool {
    visible(data.get("fillPaints"))
        || data
            .path(&["vectorData", "styleOverrideTable"])
            .is_some_and(|t| t.items().iter().any(|e| visible(e.get("fillPaints"))))
}

/// Rebuilds a vector's fill paths from its network. This is the single owner of the
/// unsupported-paint count for a missing fill: nothing is counted without a potentially visible
/// fill, and a fill that yields no path is counted once.
pub(crate) fn fills(data: &Val, blobs: &[Vec<u8>], unsupported: &mut usize) -> Vec<GeomPath> {
    if !has_visible_fill(data) {
        return Vec::new();
    }
    let Some((net, scale)) = decode(data, blobs) else {
        *unsupported += 1;
        return Vec::new();
    };
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
    *unsupported += if paths.is_empty() {
        skipped.max(1)
    } else {
        skipped
    };
    paths
}

/// The decoded network with its path-building transform, or `None` when it cannot be used.
fn decode(data: &Val, blobs: &[Vec<u8>]) -> Option<(Network, vello_cpu::kurbo::Affine)> {
    let net = parse(blob(data, blobs)?, true)?;
    let normalized = data
        .path(&["vectorData", "normalizedSize"])
        .unwrap_or(data.get("size")?);
    let (w, h) = size(data);
    let (nw, nh) = (
        f64::from(normalized.num("x")?),
        f64::from(normalized.num("y")?),
    );
    if !w.is_finite()
        || !h.is_finite()
        || !nw.is_finite()
        || !nh.is_finite()
        || nw <= 0.0
        || nh <= 0.0
    {
        return None;
    }
    Some((
        net,
        vello_cpu::kurbo::Affine::scale_non_uniform(w / nw, h / nh),
    ))
}
