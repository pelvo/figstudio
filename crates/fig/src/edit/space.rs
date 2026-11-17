//! Coordinate spaces for editing: 2×3 affine transforms, a node's transform to its page, and
//! layer bounds in any ancestor's space.
//!
//! Figma stores `transform` as `m00 m01 m02 / m10 m11 m12`: a point `(x, y)` in the node maps
//! to `(m00·x + m01·y + m02, m10·x + m11·y + m12)` in its parent.

use std::collections::HashSet;

use crate::{Document, Guid, Obj, Val};

/// A 2×3 affine transform in Figma's field order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub m00: f64,
    pub m01: f64,
    pub m02: f64,
    pub m10: f64,
    pub m11: f64,
    pub m12: f64,
}

impl Xform {
    pub const IDENTITY: Xform = Xform {
        m00: 1.0,
        m01: 0.0,
        m02: 0.0,
        m10: 0.0,
        m11: 1.0,
        m12: 0.0,
    };

    /// A translation by `(x, y)`.
    pub fn translate(x: f64, y: f64) -> Xform {
        Xform {
            m02: x,
            m12: y,
            ..Xform::IDENTITY
        }
    }

    /// The `transform` of node data (identity when absent; missing fields take identity values).
    pub fn of(data: &Val) -> Xform {
        let Some(t) = data.get("transform") else {
            return Xform::IDENTITY;
        };
        let m = |k: &str, default: f64| t.num(k).map_or(default, f64::from);
        Xform {
            m00: m("m00", 1.0),
            m01: m("m01", 0.0),
            m02: m("m02", 0.0),
            m10: m("m10", 0.0),
            m11: m("m11", 1.0),
            m12: m("m12", 0.0),
        }
    }

    /// As a `transform` value.
    pub fn to_val(self) -> Val {
        Val::Obj(Obj::from([
            ("m00", Val::Float(self.m00)),
            ("m01", Val::Float(self.m01)),
            ("m02", Val::Float(self.m02)),
            ("m10", Val::Float(self.m10)),
            ("m11", Val::Float(self.m11)),
            ("m12", Val::Float(self.m12)),
        ]))
    }

    /// `self ∘ other`: apply `other` first, then `self`.
    pub fn compose(self, other: Xform) -> Xform {
        Xform {
            m00: self.m00 * other.m00 + self.m01 * other.m10,
            m01: self.m00 * other.m01 + self.m01 * other.m11,
            m02: self.m00 * other.m02 + self.m01 * other.m12 + self.m02,
            m10: self.m10 * other.m00 + self.m11 * other.m10,
            m11: self.m10 * other.m01 + self.m11 * other.m11,
            m12: self.m10 * other.m02 + self.m11 * other.m12 + self.m12,
        }
    }

    pub fn determinant(self) -> f64 {
        self.m00 * self.m11 - self.m01 * self.m10
    }

    /// The inverse, or `None` for a degenerate (zero-scale) transform.
    pub fn inverse(self) -> Option<Xform> {
        let det = self.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (
            self.m00 / det,
            self.m01 / det,
            self.m10 / det,
            self.m11 / det,
        );
        Some(Xform {
            m00: d,
            m01: -b,
            m02: -(d * self.m02 - b * self.m12),
            m10: -c,
            m11: a,
            m12: -(-c * self.m02 + a * self.m12),
        })
    }

    pub fn apply(self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.m00 * x + self.m01 * y + self.m02,
            self.m10 * x + self.m11 * y + self.m12,
        )
    }
}

/// An axis-aligned rectangle `(x0, y0)–(x1, y1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Bounds {
    pub fn width(self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(self) -> f64 {
        self.y1 - self.y0
    }

    pub fn union(self, o: Bounds) -> Bounds {
        Bounds {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    /// The bounds of the points.
    fn of_points(points: impl IntoIterator<Item = (f64, f64)>) -> Option<Bounds> {
        points.into_iter().fold(None, |acc, (x, y)| {
            let p = Bounds {
                x0: x,
                y0: y,
                x1: x,
                y1: y,
            };
            Some(acc.map_or(p, |b: Bounds| b.union(p)))
        })
    }
}

/// A node's `size` (negative or missing values read as 0).
pub fn size(data: &Val) -> (f64, f64) {
    let get = |k: &str| {
        data.path(&["size", k])
            .and_then(Val::as_f32)
            .map_or(0.0, |v| f64::from(v).max(0.0))
    };
    (get("x"), get("y"))
}

/// The transform from `id` to its page (the product of its own and its ancestors'
/// transforms; pages and the document contribute nothing). A parent loop ends the walk.
pub fn world_transform(doc: &Document, id: Guid) -> Xform {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = Some(id);
    while let Some(c) = cur {
        if !seen.insert(c) {
            break;
        }
        let Some(node) = doc.node(c) else { break };
        if matches!(node.kind.as_str(), "CANVAS" | "DOCUMENT") {
            break;
        }
        chain.push(Xform::of(&node.data));
        cur = node.parent;
    }
    chain
        .into_iter()
        .rev()
        .fold(Xform::IDENTITY, |acc, t| acc.compose(t))
}

/// `id`'s layer bounds (its `0,0 – size` box, rotation included; no stroke or effect outsets)
/// in the coordinate space of `space` (an ancestor, or a page for page coordinates). `None`
/// when `id` is missing or `space` has a degenerate transform.
pub fn layer_bounds(doc: &Document, id: Guid, space: Guid) -> Option<Bounds> {
    let node = doc.node(id)?;
    let to_space = world_transform(doc, space)
        .inverse()?
        .compose(world_transform(doc, id));
    let (w, h) = size(&node.data);
    Bounds::of_points([(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|p| to_space.apply(p)))
}

/// `id`'s layer bounds in its parent's space, from its own transform and size.
pub fn bounds_in_parent(data: &Val) -> Option<Bounds> {
    let t = Xform::of(data);
    let (w, h) = size(data);
    Bounds::of_points([(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|p| t.apply(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_undoes_a_rotated_scaled_translation() {
        let (s, c) = 0.7f64.sin_cos();
        let t = Xform {
            m00: 2.0 * c,
            m01: -3.0 * s,
            m02: 10.0,
            m10: 2.0 * s,
            m11: 3.0 * c,
            m12: -4.0,
        };
        let p = t.inverse().unwrap().apply(t.apply((5.0, 7.0)));
        assert!((p.0 - 5.0).abs() < 1e-9 && (p.1 - 7.0).abs() < 1e-9);
        let id = t.compose(t.inverse().unwrap());
        assert!((id.m00 - 1.0).abs() < 1e-9 && id.m01.abs() < 1e-9 && id.m02.abs() < 1e-9);
    }

    #[test]
    fn a_zero_scale_transform_has_no_inverse() {
        let t = Xform {
            m00: 0.0,
            m11: 0.0,
            ..Xform::IDENTITY
        };
        assert!(t.inverse().is_none());
    }
}
