//! Drop and inner shadows drawn from the node's fill shape.
//!
//! Figma composites every drop shadow behind the node in list order, and an inner shadow is
//! the blurred, offset outside of the shape clipped to the shape and drawn over the fills.

use figstudio_fig::Val;
use vello_cpu::PaintType;
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, BezPath, Shape, Stroke, StrokeOpts};
use vello_cpu::peniko::Fill;

use crate::geom::GeomPath;
use crate::{Ctx, effect_reach, paint};

/// Whether the node's own fill shape can stand in for the node when casting shadows.
pub(crate) fn casts_shadow(data: &Val, fill_paths: &[GeomPath]) -> bool {
    !fill_paths.is_empty()
        && data
            .get("fillPaints")
            .is_some_and(|p| p.items().iter().any(|p| p.flag("visible") != Some(false)))
}

/// All fill paths as one path, with the rule of the first.
fn union(fill: &[GeomPath]) -> (BezPath, Fill) {
    let mut path = BezPath::new();
    for g in fill {
        path.extend(g.path.iter());
    }
    (path, fill.first().map_or(Fill::NonZero, |g| g.rule))
}

fn spread_outline(path: &BezPath, spread: f64) -> BezPath {
    let style = Stroke::new(spread * 2.0);
    vello_cpu::kurbo::stroke(path.iter(), &style, &StrokeOpts::default(), 0.1)
}

struct Shadow {
    dx: f32,
    dy: f32,
    sigma: f32,
    color: AlphaColor<Srgb>,
    spread: f64,
}

impl Ctx<'_> {
    /// Reads a shadow effect; counts a blend mode or negative spread that is not applied.
    fn shadow(&mut self, effect: &Val) -> Shadow {
        let offset = effect.get("offset");
        let spread = f64::from(effect.num("spread").unwrap_or(0.0));
        if spread < 0.0 || effect.text("blendMode").is_some_and(|m| m != "NORMAL") {
            self.stats.approximated_effects += 1;
        }
        Shadow {
            dx: offset.and_then(|o| o.num("x")).unwrap_or(0.0),
            dy: offset.and_then(|o| o.num("y")).unwrap_or(0.0),
            sigma: effect.num("radius").unwrap_or(0.0).max(0.0) / 2.0,
            color: paint::color(effect.get("color"), 1.0)
                .unwrap_or(AlphaColor::new([0.0, 0.0, 0.0, 0.25])),
            spread: spread.max(0.0),
        }
    }

    fn push_shadow_layer(&mut self, s: &Shadow, effect: &Val, world: Affine) {
        self.rc.set_transform(world);
        self.rc.push_filter_layer(
            Filter::from_primitive(FilterPrimitive::DropShadowOnly {
                dx: s.dx,
                dy: s.dy,
                std_deviation: s.sigma,
                color: s.color,
                edge_mode: EdgeMode::None,
            }),
            effect_reach(effect),
        );
        self.rc.set_paint(PaintType::Solid(AlphaColor::BLACK));
    }

    pub(crate) fn draw_drop_shadow(&mut self, effect: &Val, fill: &[GeomPath], world: Affine) {
        let s = self.shadow(effect);
        self.stats.shadows += 1;
        let (path, rule) = union(fill);
        self.push_shadow_layer(&s, effect, world);
        self.rc.set_fill_rule(rule);
        self.rc.fill_path(&path);
        if s.spread > 0.0 {
            self.rc.set_fill_rule(Fill::NonZero);
            self.rc.fill_path(&spread_outline(&path, s.spread));
        }
        self.rc.pop_layer();
    }

    pub(crate) fn draw_inner_shadow(&mut self, effect: &Val, fill: &[GeomPath], world: Affine) {
        let s = self.shadow(effect);
        self.stats.inner_shadows += 1;
        let (path, rule) = union(fill);
        self.rc.set_transform(world);
        self.rc.push_clip_layer(&path);
        self.push_shadow_layer(&s, effect, world);
        // The outside of the shape, reaching past anything the blur can carry inwards.
        let margin = effect_reach(effect) + 1.0;
        let mut outside = path.clone();
        outside.extend(
            path.bounding_box()
                .inflate(margin, margin)
                .path_elements(0.1),
        );
        self.rc.set_fill_rule(if rule == Fill::NonZero {
            Fill::EvenOdd
        } else {
            rule
        });
        self.rc.fill_path(&outside);
        if s.spread > 0.0 {
            self.rc.set_fill_rule(Fill::NonZero);
            self.rc.fill_path(&spread_outline(&path, s.spread));
        }
        self.rc.pop_layer();
        self.rc.pop_layer();
    }
}
