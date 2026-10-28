//! Export bounds: the area the renderer's drawing calls cover, worked out without drawing.
//!
//! Exports are cropped to it, as Figma crops to a layer's render bounds: a fill covers its
//! path's box, cut by the clips around it; a blur widens what it holds by its radius (Figma's
//! radius, twice the Gaussian's standard deviation); a drop shadow adds a copy moved by its
//! offset and widened by its radius. Erasing fills (destination-out) cover nothing.

use vello_cpu::PaintType;
use vello_cpu::filter_effects::{Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, BezPath, Rect, Shape, Vec2};
use vello_cpu::peniko::{BlendMode, Compose, Fill};

use crate::paint::linear_scale;
use crate::record::Canvas;

/// What an open group does to the area of what is drawn inside it (output units).
#[derive(Clone, Copy, Debug)]
enum Effect {
    /// Nothing (an opacity layer, or a filter that keeps geometry).
    None,
    /// Cuts to this box.
    Clip(Rect),
    /// Widens by this much on every side.
    Grow(f64),
    /// Adds a copy moved by `offset` and widened by `grow`; `keep` keeps the original too.
    Shadow { offset: Vec2, grow: f64, keep: bool },
}

impl Effect {
    fn apply(self, area: Rect) -> Rect {
        match self {
            Effect::None => area,
            Effect::Clip(clip) => area.intersect(clip),
            Effect::Grow(by) => area.inflate(by, by),
            Effect::Shadow { offset, grow, keep } => {
                let shadow = (area + offset).inflate(grow, grow);
                if keep { shadow.union(area) } else { shadow }
            }
        }
    }
}

/// A canvas that records the union of what its fills cover, in output units.
#[derive(Debug, Default)]
pub(crate) struct InkCanvas {
    transform: Affine,
    blend: BlendMode,
    open: Vec<Effect>,
    ink: Option<Rect>,
}

impl InkCanvas {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The covered area; `None` when nothing was drawn.
    pub(crate) fn ink(&self) -> Option<Rect> {
        self.ink
    }

    fn add(&mut self, local: Rect) {
        if self.blend.compose != Compose::SrcOver {
            return;
        }
        let mut area = self.transform.transform_rect_bbox(local);
        // Innermost group first: a shadow is moved before an outer clip cuts it.
        for effect in self.open.iter().rev() {
            area = effect.apply(area);
        }
        if area.width() > 0.0 && area.height() > 0.0 {
            self.ink = Some(self.ink.map_or(area, |ink| ink.union(area)));
        }
    }

    fn effect_of(&self, filter: &Filter, reach: f64) -> Effect {
        let scale = linear_scale(self.transform);
        let offset = |dx: f32, dy: f32| {
            let [a, b, c, d, _, _] = self.transform.as_coeffs();
            let (dx, dy) = (f64::from(dx), f64::from(dy));
            Vec2::new(a * dx + c * dy, b * dx + d * dy)
        };
        match filter.graph.primitives.as_slice() {
            [FilterPrimitive::GaussianBlur { std_deviation, .. }] => {
                Effect::Grow(2.0 * f64::from(*std_deviation) * scale)
            }
            [
                FilterPrimitive::DropShadow {
                    dx,
                    dy,
                    std_deviation,
                    ..
                },
            ] => Effect::Shadow {
                offset: offset(*dx, *dy),
                grow: 2.0 * f64::from(*std_deviation) * scale,
                keep: true,
            },
            [
                FilterPrimitive::DropShadowOnly {
                    dx,
                    dy,
                    std_deviation,
                    ..
                },
            ] => Effect::Shadow {
                offset: offset(*dx, *dy),
                grow: 2.0 * f64::from(*std_deviation) * scale,
                keep: false,
            },
            // Anything else: the renderer's own bound on how far it can reach.
            _ => Effect::Grow(reach.abs() * scale),
        }
    }
}

impl Canvas for InkCanvas {
    fn set_transform(&mut self, t: Affine) {
        self.transform = t;
    }
    fn set_paint(&mut self, _paint: PaintType) {}
    fn set_paint_transform(&mut self, _t: Affine) {}
    fn set_fill_rule(&mut self, _rule: Fill) {}
    fn set_blend_mode(&mut self, mode: BlendMode) {
        self.blend = mode;
    }
    fn fill_path(&mut self, path: &BezPath) {
        self.add(path.bounding_box());
    }
    fn fill_rect(&mut self, rect: &Rect) {
        self.add(*rect);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        let clip = self.transform.transform_rect_bbox(path.bounding_box());
        self.open.push(Effect::Clip(clip));
    }
    fn push_clip_path(&mut self, path: &BezPath) {
        self.push_clip_layer(path);
    }
    fn pop_clip(&mut self) {
        self.open.pop();
    }
    fn push_opacity_layer(&mut self, _opacity: f32) {
        self.open.push(Effect::None);
    }
    fn push_filter_layer(&mut self, filter: Filter, reach: f64) {
        let effect = self.effect_of(&filter, reach);
        self.open.push(effect);
    }
    fn pop_layer(&mut self) {
        self.open.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vello_cpu::color::{AlphaColor, Srgb};
    use vello_cpu::filter_effects::EdgeMode;
    use vello_cpu::peniko::Mix;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    #[test]
    fn clips_cut_and_erasing_fills_add_nothing() {
        let mut c = InkCanvas::new();
        c.push_clip_path(&rect(0.0, 0.0, 50.0, 50.0));
        c.fill_path(&rect(40.0, 40.0, 90.0, 90.0));
        c.pop_clip();
        c.fill_path(&rect(10.0, 20.0, 15.0, 25.0));
        c.set_blend_mode(BlendMode::new(Mix::Normal, Compose::DestOut));
        c.fill_path(&rect(0.0, 0.0, 200.0, 200.0));
        assert_eq!(c.ink(), Some(Rect::new(10.0, 20.0, 50.0, 50.0)));
    }

    #[test]
    fn a_drop_shadow_adds_a_moved_copy_widened_by_its_radius() {
        let mut c = InkCanvas::new();
        // Figma radius 4 is a standard deviation of 2; offset 4 down.
        c.push_filter_layer(
            Filter::from_primitive(FilterPrimitive::DropShadow {
                dx: 0.0,
                dy: 4.0,
                std_deviation: 2.0,
                color: AlphaColor::<Srgb>::BLACK,
                edge_mode: EdgeMode::None,
            }),
            0.0,
        );
        c.fill_path(&rect(0.0, 0.0, 100.0, 100.0));
        c.pop_layer();
        assert_eq!(c.ink(), Some(Rect::new(-4.0, 0.0, 104.0, 108.0)));
    }
}
