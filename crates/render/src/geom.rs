//! Geometry: transforms, sizes, blob paths and fallback shapes.

use figstudio_fig::Val;
use vello_cpu::kurbo::{
    Affine, BezPath, Ellipse, Point, Rect, RoundedRect, RoundedRectRadii, Shape,
};
use vello_cpu::peniko::Fill;

/// Figma's 2x3 matrix (`m00 m01 m02 / m10 m11 m12`) as a kurbo affine.
pub fn affine(value: Option<&Val>) -> Affine {
    let Some(t) = value else {
        return Affine::IDENTITY;
    };
    let m = |k: &str, default: f64| t.num(k).map_or(default, f64::from);
    Affine::new([
        m("m00", 1.0),
        m("m10", 0.0),
        m("m01", 0.0),
        m("m11", 1.0),
        m("m02", 0.0),
        m("m12", 0.0),
    ])
}

pub fn size(data: &Val) -> (f64, f64) {
    let s = data.get("size");
    let get = |k: &str| {
        s.and_then(|s| s.num(k))
            .map_or(0.0, |v| f64::from(v).max(0.0))
    };
    (get("x"), get("y"))
}

/// Decodes a Figma path blob: `0` close, `1` move, `2` line, `3` quad, `4` cubic, little-endian f32s.
pub fn blob_path(bytes: &[u8]) -> Option<BezPath> {
    let mut path = BezPath::new();
    let read = |at: usize| -> Option<f64> {
        let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(f64::from(f32::from_le_bytes(raw)))
    };
    // Figma continues drawing after a close without a new move; kurbo needs an explicit
    // move back to the subpath start, and a segment never starts without a current point.
    let mut start = Point::ZERO;
    let mut need_move = true;
    let mut i = 0;
    while i < bytes.len() {
        let cmd = bytes[i];
        i += 1;
        if matches!(cmd, 2..=4) && need_move {
            path.move_to(start);
            need_move = false;
        }
        match cmd {
            0 => {
                if !need_move {
                    path.close_path();
                }
                need_move = true;
            }
            1 => {
                start = Point::new(read(i)?, read(i + 4)?);
                path.move_to(start);
                need_move = false;
                i += 8;
            }
            2 => {
                path.line_to((read(i)?, read(i + 4)?));
                i += 8;
            }
            3 => {
                path.quad_to((read(i)?, read(i + 4)?), (read(i + 8)?, read(i + 12)?));
                i += 16;
            }
            4 => {
                path.curve_to(
                    (read(i)?, read(i + 4)?),
                    (read(i + 8)?, read(i + 12)?),
                    (read(i + 16)?, read(i + 20)?),
                );
                i += 24;
            }
            _ => return None,
        }
    }
    Some(path)
}

/// Bounding box of the control points of a Figma path blob, read without building the path.
/// `None` for a malformed or empty blob (which draws nothing).
pub fn blob_bounds(bytes: &[u8]) -> Option<Rect> {
    let read = |at: usize| -> Option<f64> {
        let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(f64::from(f32::from_le_bytes(raw)))
    };
    let mut bounds: Option<Rect> = None;
    let mut grow = |x: f64, y: f64| {
        let r = Rect::new(x, y, x, y);
        bounds = Some(bounds.map_or(r, |b| b.union(r)));
    };
    let mut i = 0;
    while i < bytes.len() {
        let points = match bytes[i] {
            0 => 0,
            1 | 2 => 1,
            3 => 2,
            4 => 3,
            _ => return None,
        };
        i += 1;
        for k in 0..points {
            grow(read(i + 8 * k)?, read(i + 8 * k + 4)?);
        }
        i += 8 * points;
    }
    bounds.filter(|b| b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite())
}

/// Whether a vector's network (`vectorData.vectorNetworkBlob`) is closed: every segment lies on
/// a region loop, so its fill outline is the boundary inside/outside strokes align to.
/// True when every segment of the vector network lies on some region loop.
pub fn closed_network(data: &Val, blobs: &[Vec<u8>]) -> bool {
    crate::network::closed(data, blobs)
}

/// One precomputed path. `style` points into `vectorData.styleOverrideTable` (0 = node paints).
pub struct GeomPath {
    pub path: BezPath,
    pub rule: Fill,
    pub style: u32,
}

impl GeomPath {
    pub fn plain(path: BezPath) -> Self {
        GeomPath {
            path,
            rule: Fill::NonZero,
            style: 0,
        }
    }
}

/// Precomputed geometry (`fillGeometry`, `strokeGeometry`) referencing message blobs.
pub fn geometry(
    data: &Val,
    key: &str,
    blobs: &[Vec<u8>],
    unsupported: &mut usize,
) -> Vec<GeomPath> {
    let mut out = Vec::new();
    let Some(list) = data.get(key) else {
        return out;
    };
    for item in list.items() {
        let Some(index) = item.get("commandsBlob").and_then(Val::as_u32) else {
            *unsupported += 1;
            continue;
        };
        let rule = if item.text("windingRule") == Some("ODD") {
            Fill::EvenOdd
        } else {
            Fill::NonZero
        };
        let style = item.get("styleID").and_then(Val::as_u32).unwrap_or(0);
        if let Some(path) = blobs.get(index as usize).and_then(|b| blob_path(b)) {
            out.push(GeomPath { path, rule, style });
        } else {
            *unsupported += 1;
        }
    }
    out
}

pub fn corner_radii(data: &Val, w: f64, h: f64) -> RoundedRectRadii {
    let limit = (w.min(h) / 2.0).max(0.0);
    let clamp = |v: Option<f32>| v.map_or(0.0, |v| f64::from(v).clamp(0.0, limit));
    if data.flag("rectangleCornerRadiiIndependent") == Some(true) {
        RoundedRectRadii::new(
            clamp(data.num("rectangleTopLeftCornerRadius")),
            clamp(data.num("rectangleTopRightCornerRadius")),
            clamp(data.num("rectangleBottomRightCornerRadius")),
            clamp(data.num("rectangleBottomLeftCornerRadius")),
        )
    } else {
        RoundedRectRadii::from_single_radius(clamp(data.num("cornerRadius")))
    }
}

/// Node types laid out as boxes that can hold children.
pub fn is_container(kind: &str) -> bool {
    matches!(
        kind,
        "FRAME" | "SYMBOL" | "INSTANCE" | "SECTION" | "COMPONENT_SET"
    )
}

/// Shape used when a node has no precomputed fill geometry, and as the clip path of frames.
pub fn base_shape(kind: &str, data: &Val) -> Option<BezPath> {
    let (w, h) = size(data);
    if w <= 0.0 && h <= 0.0 {
        return None;
    }
    let rect = Rect::new(0.0, 0.0, w, h);
    match kind {
        "ELLIPSE" => Some(Ellipse::from_rect(rect).to_path(0.1)),
        "TEXT" | "VECTOR" | "LINE" | "STAR" | "REGULAR_POLYGON" | "BOOLEAN_OPERATION" | "SLICE" => {
            None
        }
        _ => Some(RoundedRect::from_rect(rect, corner_radii(data, w, h)).to_path(0.1)),
    }
}

/// Whether a frame-like node clips its children ("Clip content").
pub fn clips_content(kind: &str, data: &Val) -> bool {
    is_container(kind)
        && kind != "SECTION"
        && data.flag("resizeToFit") != Some(true)
        && data.flag("frameMaskDisabled") != Some(true)
}

/// Axis-aligned bounds of `rect` after `transform`.
pub fn transformed_bounds(transform: Affine, rect: Rect) -> Rect {
    let corners = [
        transform * Point::new(rect.x0, rect.y0),
        transform * Point::new(rect.x1, rect.y0),
        transform * Point::new(rect.x0, rect.y1),
        transform * Point::new(rect.x1, rect.y1),
    ];
    let mut out = Rect::from_points(corners[0], corners[1]);
    out = out.union_pt(corners[2]);
    out.union_pt(corners[3])
}
