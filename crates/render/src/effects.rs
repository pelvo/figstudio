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
            .is_some_and(|p| p.items().iter().any(paint_has_alpha))
}

pub(crate) fn paint_has_alpha(p: &Val) -> bool {
    p.flag("visible") != Some(false)
        && p.num("opacity").unwrap_or(1.0) > 0.0
        && (p.text("type") != Some("SOLID")
            || p.get("color").and_then(|c| c.num("a")).unwrap_or(1.0) > 0.0)
}

/// Opaque geometry does not model paint alpha, style overrides, strokes or children.
pub(crate) fn approximate_silhouette(data: &Val, has_children: bool) -> bool {
    has_children
        || data
            .get("strokePaints")
            .is_some_and(|p| p.items().iter().any(paint_has_alpha))
        || data
            .path(&["vectorData", "styleOverrideTable"])
            .is_some_and(|p| !p.items().is_empty())
        || data.get("fillPaints").is_some_and(|p| {
            p.items().iter().any(|p| {
                paint_has_alpha(p)
                    && (p.text("type") != Some("SOLID")
                        || p.num("opacity").unwrap_or(1.0) < 1.0
                        || p.get("color").and_then(|c| c.num("a")).unwrap_or(1.0) < 1.0)
            })
        })
}

/// Combined path for the approximate inner mask; drop masks draw paths separately.
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

/// Count an effect once even when several parts of it are approximate.
pub(crate) fn is_approximate(
    effect: &Val,
    fill: &[GeomPath],
    world: Affine,
    silhouette: bool,
    fallback: bool,
) -> bool {
    let [a, b, c, d, _, _] = world.as_coeffs();
    let x = a.hypot(b);
    let y = c.hypot(d);
    let anisotropic = (x - y).abs() > 1e-6 * x.max(y) || (a * c + b * d).abs() > 1e-6 * x * y;
    let compound_inner = effect.text("type") == Some("INNER_SHADOW")
        && (fill.len() > 1
            || fill.iter().any(|g| {
                g.rule == Fill::NonZero
                    && g.path
                        .elements()
                        .iter()
                        .filter(|e| matches!(e, vello_cpu::kurbo::PathEl::MoveTo(_)))
                        .count()
                        > 1
            }));
    silhouette
        || compound_inner
        || effect.text("blendMode").is_some_and(|m| m != "NORMAL")
        || effect.num("spread").unwrap_or(0.0) < 0.0
        || (fallback && effect.num("spread").unwrap_or(0.0) > 0.0)
        || (anisotropic && effect.num("radius").unwrap_or(0.0) > 0.0)
}

struct Shadow {
    dx: f32,
    dy: f32,
    sigma: f32,
    color: AlphaColor<Srgb>,
    spread: f64,
}

impl Ctx<'_> {
    /// Charge every shadow attempt and bound Vello's retained filter pixmaps per node.
    pub(crate) fn admit_effect(&mut self, effect: &Val, shadows: &mut usize) -> bool {
        if !matches!(effect.text("type"), Some("DROP_SHADOW" | "INNER_SHADOW")) {
            return true;
        }
        let charged = self.charge_work();
        if !charged || *shadows >= 8 {
            self.stats.skipped_effects += 1;
            return false;
        }
        *shadows += 1;
        true
    }

    /// Reads the supported shadow parameters; the caller counts approximations.
    fn shadow(&self, effect: &Val) -> Shadow {
        let offset = effect.get("offset");
        let spread = f64::from(effect.num("spread").unwrap_or(0.0));
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

    pub(crate) fn fallback_shadow(&mut self, effect: &Val) -> FilterPrimitive {
        let s = self.shadow(effect);
        FilterPrimitive::DropShadow {
            dx: s.dx,
            dy: s.dy,
            std_deviation: s.sigma,
            color: s.color,
            edge_mode: EdgeMode::None,
        }
    }

    pub(crate) fn draw_drop_shadow(&mut self, effect: &Val, fill: &[GeomPath], world: Affine) {
        let s = self.shadow(effect);
        self.stats.shadows += 1;
        self.push_shadow_layer(&s, effect, world);
        for g in fill {
            self.rc.set_fill_rule(g.rule);
            self.rc.fill_path(&g.path);
            if s.spread > 0.0 {
                self.rc.set_fill_rule(Fill::NonZero);
                self.rc.fill_path(&spread_outline(&g.path, s.spread));
            }
        }
        self.rc.pop_layer();
    }

    pub(crate) fn draw_inner_shadow(&mut self, effect: &Val, fill: &[GeomPath], world: Affine) {
        let s = self.shadow(effect);
        self.stats.inner_shadows += 1;
        // shortcut: compound nonzero complements and multi-path clips are approximate
        // (counted by the caller); use a boolean mask compositor when exactness is needed.
        let (path, rule) = union(fill);
        self.rc.set_transform(world);
        self.rc.set_fill_rule(rule);
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
