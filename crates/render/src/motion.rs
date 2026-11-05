//! Motion playback: the values a layer's keyframes give it at the playhead ([`Playhead`]),
//! applied to its data before it is drawn.
//!
//! The renderer knows nothing of keyframes: a [`Motion`] (figstudio-mcp-core's player) reads
//! them and answers with a [`Pose`]. The renderer adds what only the walk knows: the delay of
//! the instances a layer is drawn in.

use std::fmt;
use std::sync::Arc;

use figstudio_fig::{Document, Node, Obj, Val};
use vello_cpu::kurbo::{Affine, Point, Vec2};

use crate::geom::{affine, base_shape, size};

/// Reads a layer's motion.
pub trait Motion: Send + Sync {
    /// The pose of `node` (with `data`, after instance overrides) at `time_us` on the
    /// playback clock, its keyframes delayed by `delay_us`; `None` when it does not move.
    fn pose(
        &self,
        doc: &Document,
        node: &Node,
        data: &Val,
        delay_us: f64,
        time_us: f64,
    ) -> Option<Pose>;
    /// How long an instance delays its component's motion (µs).
    fn instance_delay(&self, data: &Val) -> f64;
}

/// Where playback is: what reads the motion, and the time (µs).
#[derive(Clone)]
pub struct Playhead {
    pub motion: Arc<dyn Motion>,
    pub time_us: f64,
}

impl fmt::Debug for Playhead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Playhead")
            .field("time_us", &self.time_us)
            .finish_non_exhaustive()
    }
}

/// A layer's animated values at one moment. Fields left at their default leave the layer
/// as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Opacity, 0 to 1.
    pub opacity: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Moved by this much in the parent's space.
    pub translate: Vec2,
    /// Turned by this many radians, clockwise, about the origin.
    pub rotation: f64,
    /// Scaled about the origin.
    pub scale: Vec2,
    /// Sheared along x by this many radians.
    pub shear: f64,
    /// The pivot as fractions of the layer's size (the centre by default).
    pub origin: Point,
}

impl Default for Pose {
    fn default() -> Self {
        Pose {
            opacity: None,
            width: None,
            height: None,
            translate: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::new(1.0, 1.0),
            shear: 0.0,
            origin: Point::new(0.5, 0.5),
        }
    }
}

/// Writes `pose` into a layer's `data`.
pub(crate) fn apply(kind: &str, data: &mut Val, pose: &Pose) {
    let Val::Obj(o) = data else {
        return;
    };
    if let Some(opacity) = pose.opacity {
        o.insert("opacity", Val::Float(opacity.clamp(0.0, 1.0)));
    }
    if pose.width.is_some() || pose.height.is_some() {
        let (w, h) = size(&Val::Obj(o.clone()));
        let mut s = Obj::new();
        s.insert("x", Val::Float(pose.width.unwrap_or(w).max(0.0)));
        s.insert("y", Val::Float(pose.height.unwrap_or(h).max(0.0)));
        o.insert("size", Val::Obj(s));
        // Stored outlines are for the old size: redraw from the shape where there is one.
        if base_shape(kind, &Val::Obj(o.clone())).is_some() {
            o.remove("fillGeometry");
            o.remove("strokeGeometry");
        }
    }
    let moves = pose.translate != Vec2::ZERO
        || pose.rotation != 0.0
        || pose.scale != Vec2::new(1.0, 1.0)
        || pose.shear != 0.0;
    if !moves {
        return;
    }
    let (w, h) = size(&Val::Obj(o.clone()));
    let pivot = Vec2::new(pose.origin.x * w, pose.origin.y * h);
    let local = Affine::translate(pivot)
        * Affine::rotate(pose.rotation)
        * Affine::skew(pose.shear.tan(), 0.0)
        * Affine::scale_non_uniform(pose.scale.x, pose.scale.y)
        * Affine::translate(-pivot);
    let t = Affine::translate(pose.translate) * affine(o.get("transform")) * local;
    let [m00, m10, m01, m11, m02, m12] = t.as_coeffs();
    let mut m = Obj::new();
    for (k, v) in [
        ("m00", m00),
        ("m01", m01),
        ("m02", m02),
        ("m10", m10),
        ("m11", m11),
        ("m12", m12),
    ] {
        m.insert(k, Val::Float(v));
    }
    o.insert("transform", Val::Obj(m));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, h: f64) -> Val {
        let mut s = Obj::new();
        s.insert("x", Val::Float(w));
        s.insert("y", Val::Float(h));
        let mut o = Obj::new();
        o.insert("size", Val::Obj(s));
        o.insert("fillGeometry", Val::Arr(Vec::new()));
        Val::Obj(o)
    }

    #[test]
    fn a_turn_pivots_on_the_centre_and_a_move_is_in_the_parent() {
        let mut d = rect(10.0, 20.0);
        let pose = Pose {
            rotation: std::f64::consts::PI,
            translate: Vec2::new(3.0, 0.0),
            ..Pose::default()
        };
        apply("ROUNDED_RECTANGLE", &mut d, &pose);
        let t = affine(d.get("transform"));
        // The centre stays put (then moves 3 right); the top-left corner swings to the
        // bottom-right.
        assert!((t * Point::new(5.0, 10.0) - Point::new(8.0, 10.0)).hypot() < 1e-9);
        assert!((t * Point::new(0.0, 0.0) - Point::new(13.0, 20.0)).hypot() < 1e-9);
    }

    #[test]
    fn a_new_size_drops_stored_outlines_of_a_shape_only() {
        let pose = Pose {
            width: Some(40.0),
            ..Pose::default()
        };
        let mut d = rect(10.0, 20.0);
        apply("ROUNDED_RECTANGLE", &mut d, &pose);
        assert_eq!(size(&d), (40.0, 20.0));
        assert!(d.get("fillGeometry").is_none());
        let mut v = rect(10.0, 20.0);
        apply("VECTOR", &mut v, &pose);
        assert!(v.get("fillGeometry").is_some());
    }
}
