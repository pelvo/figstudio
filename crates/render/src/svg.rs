//! SVG output: the renderer's drawing calls written as SVG elements instead of pixels.
//!
//! Every coordinate is written in output space (the viewport transform and each node's
//! transform applied), so the file needs no nested transforms. Text is drawn as outlines, as
//! the renderer draws it. Drawing that SVG 1.1 cannot express is counted in
//! [`Svg::approximated`]: sweep (angular) gradients, compositing other than source-over, filter
//! graphs other than one blur or drop shadow, and images held by handle instead of pixels.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::filter_effects::{Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, BezPath, PathEl, Rect, Shape};
use vello_cpu::peniko::{BlendMode, Compose, Extend, Fill, GradientKind, Mix};
use vello_cpu::{Image, ImageSource, PaintType, Pixmap};

use crate::Stats;
use crate::images::ImageRequest;
use crate::record::Canvas;

/// A finished SVG export.
#[derive(Debug, Clone)]
pub struct Svg {
    /// The document, or an empty string when it went over its size limit (see `over_limit`).
    pub markup: String,
    pub stats: Stats,
    /// Images still to read (see [`crate::Frame::wanted`]).
    pub wanted: Vec<ImageRequest>,
    /// Drawing calls SVG could not express exactly (see the module docs).
    pub approximated: usize,
    /// The markup passed the byte limit given to the export, so it was dropped.
    pub over_limit: bool,
}

/// An open `<g>` (every push opens exactly one).
struct Group {
    /// Byte offset of the group's `<g` in the body, where a mask attribute is inserted.
    start: usize,
    /// Mask shapes of destination-out fills drawn into this group, which erase what the
    /// group drew before them.
    erase: String,
}

/// Writes the renderer's drawing calls as SVG.
pub(crate) struct SvgCanvas {
    width: f64,
    height: f64,
    body: String,
    defs: String,
    next_id: usize,
    transform: Affine,
    paint: PaintType,
    paint_transform: Affine,
    rule: Fill,
    blend: BlendMode,
    /// The `fill` attributes of the current paint, written once per paint and transform.
    fill: Option<String>,
    /// Embedded images by pixmap address, so a repeated image is written once.
    images: HashMap<usize, (String, Arc<Pixmap>)>,
    open: Vec<Group>,
    approximated: usize,
    limit: usize,
    over: bool,
}

impl SvgCanvas {
    /// A canvas `width` x `height` output units that gives up past about `limit` bytes.
    pub(crate) fn new(width: f64, height: f64, limit: usize) -> Self {
        SvgCanvas {
            width,
            height,
            body: String::new(),
            defs: String::new(),
            next_id: 0,
            transform: Affine::IDENTITY,
            paint: PaintType::Solid(AlphaColor::BLACK),
            paint_transform: Affine::IDENTITY,
            rule: Fill::NonZero,
            blend: BlendMode::default(),
            fill: None,
            images: HashMap::new(),
            open: Vec::new(),
            approximated: 0,
            limit,
            over: false,
        }
    }

    /// Fills `rect` (output space) with `color`, for a page background.
    pub(crate) fn background(&mut self, color: AlphaColor<Srgb>) {
        let [r, g, b, a] = color.components;
        let _ = write!(
            self.body,
            r#"<rect width="{}" height="{}" fill="{}"{}/>"#,
            num(self.width),
            num(self.height),
            hex(r, g, b),
            opacity_attr("fill-opacity", a)
        );
    }

    /// The finished document, the approximation count and whether the limit was passed.
    pub(crate) fn finish(self) -> (String, usize, bool) {
        let view = Rect::new(0.0, 0.0, self.width, self.height);
        self.finish_view(view)
    }

    /// [`SvgCanvas::finish`] showing only `view` (output units): the `viewBox` crops, the
    /// drawing is unchanged.
    pub(crate) fn finish_view(mut self, view: Rect) -> (String, usize, bool) {
        if self.over {
            return (String::new(), self.approximated, true);
        }
        while !self.open.is_empty() {
            self.close_group();
        }
        let mut out = String::with_capacity(self.defs.len() + self.body.len() + 256);
        let _ = write!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{x} {y} {w} {h}" fill="none">"#,
            x = num(view.x0),
            y = num(view.y0),
            w = num(view.width()),
            h = num(view.height())
        );
        if !self.defs.is_empty() {
            out.push_str("<defs>");
            out.push_str(&self.defs);
            out.push_str("</defs>");
        }
        out.push_str(&self.body);
        out.push_str("</svg>");
        (out, self.approximated, false)
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    fn check(&mut self) {
        if self.body.len() + self.defs.len() > self.limit {
            self.over = true;
            self.body = String::new();
            self.defs = String::new();
            self.images.clear();
        }
    }

    /// ` fill="…"` (with opacity) for the current paint, defining a gradient or pattern once.
    fn fill_attrs(&mut self) -> String {
        if let Some(fill) = &self.fill {
            return fill.clone();
        }
        let paint = self.paint.clone();
        let space = self.transform * self.paint_transform;
        let fill = match paint {
            PaintType::Solid(color) => {
                let [r, g, b, a] = color.components;
                format!(
                    r#" fill="{}"{}"#,
                    hex(r, g, b),
                    opacity_attr("fill-opacity", a)
                )
            }
            PaintType::Gradient(gradient) => {
                let id = self.id("g");
                let spread = match gradient.extend {
                    Extend::Pad => "",
                    Extend::Repeat => r#" spreadMethod="repeat""#,
                    Extend::Reflect => r#" spreadMethod="reflect""#,
                };
                let head = match gradient.kind {
                    GradientKind::Linear(p) => format!(
                        r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}"{}{spread}>"#,
                        num(p.start.x),
                        num(p.start.y),
                        num(p.end.x),
                        num(p.end.y),
                        matrix_attr("gradientTransform", space)
                    ),
                    GradientKind::Radial(p) => format!(
                        r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{}" cy="{}" r="{}" fx="{}" fy="{}" fr="{}"{}{spread}>"#,
                        num(p.end_center.x),
                        num(p.end_center.y),
                        num(f64::from(p.end_radius)),
                        num(p.start_center.x),
                        num(p.start_center.y),
                        num(f64::from(p.start_radius)),
                        matrix_attr("gradientTransform", space)
                    ),
                    GradientKind::Sweep(_) => {
                        // SVG has no sweep gradient: the first stop's colour stands in.
                        self.approximated += 1;
                        let first = gradient.stops.first().map_or(AlphaColor::TRANSPARENT, |s| {
                            s.color.to_alpha_color::<Srgb>()
                        });
                        let [r, g, b, a] = first.components;
                        let fill = format!(
                            r#" fill="{}"{}"#,
                            hex(r, g, b),
                            opacity_attr("fill-opacity", a)
                        );
                        self.fill = Some(fill.clone());
                        return fill;
                    }
                };
                let close = if matches!(gradient.kind, GradientKind::Linear(_)) {
                    "</linearGradient>"
                } else {
                    "</radialGradient>"
                };
                self.defs.push_str(&head);
                for stop in gradient.stops.iter() {
                    let [r, g, b, a] = stop.color.to_alpha_color::<Srgb>().components;
                    let _ = write!(
                        self.defs,
                        r#"<stop offset="{}" stop-color="{}"{}/>"#,
                        num(f64::from(stop.offset)),
                        hex(r, g, b),
                        opacity_attr("stop-opacity", a)
                    );
                }
                self.defs.push_str(close);
                format!(r#" fill="url(#{id})""#)
            }
            PaintType::Image(image) => self.pattern(&image, space),
        };
        self.fill = Some(fill.clone());
        fill
    }

    /// A pattern drawing `image` in `space` (image pixels to output units).
    fn pattern(&mut self, image: &Image, space: Affine) -> String {
        let ImageSource::Pixmap(pixmap) = &image.image else {
            self.approximated += 1;
            return r#" fill="none""#.to_owned();
        };
        let key = Arc::as_ptr(pixmap) as usize;
        let image_id = match self.images.get(&key) {
            Some((id, _)) => id.clone(),
            None => {
                let id = self.id("i");
                // An encoding failure leaves the area unfilled, counted.
                let Ok(png) = Pixmap::clone(pixmap).into_png() else {
                    self.approximated += 1;
                    return r#" fill="none""#.to_owned();
                };
                let _ = write!(
                    self.defs,
                    r#"<image id="{id}" width="{}" height="{}" href="data:image/png;base64,{}"/>"#,
                    pixmap.width(),
                    pixmap.height(),
                    STANDARD.encode(png)
                );
                self.images.insert(key, (id.clone(), pixmap.clone()));
                id
            }
        };
        let id = self.id("p");
        let alpha = image.sampler.alpha;
        let _ = write!(
            self.defs,
            r##"<pattern id="{id}" patternUnits="userSpaceOnUse" width="{}" height="{}"{}><use href="#{image_id}"{}/></pattern>"##,
            pixmap.width(),
            pixmap.height(),
            matrix_attr("patternTransform", space),
            opacity_attr("opacity", alpha)
        );
        format!(r#" fill="url(#{id})""#)
    }

    fn open_group(&mut self, attrs: &str) {
        self.open.push(Group {
            start: self.body.len(),
            erase: String::new(),
        });
        self.body.push_str("<g");
        self.body.push_str(attrs);
        self.body.push('>');
        self.check();
    }

    fn close_group(&mut self) {
        let Some(group) = self.open.pop() else {
            return;
        };
        self.body.push_str("</g>");
        if group.erase.is_empty() || self.over {
            return;
        }
        // Destination-out: a mask that is white except under the erasing shapes.
        let id = self.id("m");
        let _ = write!(
            self.defs,
            r##"<mask id="{id}" maskUnits="userSpaceOnUse" x="0" y="0" width="{w}" height="{h}"><rect width="{w}" height="{h}" fill="#fff"/>{}</mask>"##,
            group.erase,
            w = num(self.width),
            h = num(self.height)
        );
        let at = group.start + "<g".len();
        if at <= self.body.len() {
            self.body.insert_str(at, &format!(r#" mask="url(#{id})""#));
        }
        self.check();
    }

    /// A destination-out fill of `d`: recorded as a mask shape of the innermost group. It
    /// erases by the paint's alpha.
    fn erase(&mut self, d: &str, rule: &str) {
        let alpha = match &self.paint {
            PaintType::Solid(color) => color.components[3],
            _ => {
                self.approximated += 1;
                1.0
            }
        };
        let Some(group) = self.open.last_mut() else {
            // Nothing isolates the erase: it would cut through the whole picture.
            self.approximated += 1;
            return;
        };
        let _ = write!(
            group.erase,
            r##"<path d="{d}" fill="#000"{}{rule}/>"##,
            opacity_attr("fill-opacity", alpha)
        );
    }

    /// A `<filter>` for `filter` drawn under `self.transform`; `None` when SVG cannot express it.
    fn filter(&mut self, filter: &Filter) -> Option<String> {
        let [primitive] = filter.graph.primitives.as_slice() else {
            return None;
        };
        let [a, b, c, d, _, _] = self.transform.as_coeffs();
        let (sx, sy) = (a.hypot(b), c.hypot(d));
        // A blur under rotation with unequal scales has no axis-aligned SVG equivalent.
        if (b.abs() > 1e-9 || c.abs() > 1e-9) && (sx - sy).abs() > 1e-6 * sx.max(sy) {
            self.approximated += 1;
        }
        let blur = |sigma: f32| {
            format!(
                "{} {}",
                num(f64::from(sigma) * sx),
                num(f64::from(sigma) * sy)
            )
        };
        let shadow = |dx: f32, dy: f32, sigma: f32, color: AlphaColor<Srgb>| {
            let offset = self.transform.as_coeffs();
            let (ox, oy) = (
                offset[0] * f64::from(dx) + offset[2] * f64::from(dy),
                offset[1] * f64::from(dx) + offset[3] * f64::from(dy),
            );
            let [r, g, bl, al] = color.components;
            format!(
                r#"<feGaussianBlur in="SourceAlpha" stdDeviation="{}"/><feOffset dx="{}" dy="{}" result="o"/><feFlood flood-color="{}" flood-opacity="{}"/><feComposite in2="o" operator="in""#,
                blur(sigma),
                num(ox),
                num(oy),
                hex(r, g, bl),
                num(f64::from(al))
            )
        };
        let body = match primitive {
            FilterPrimitive::GaussianBlur { std_deviation, .. } => {
                format!(
                    r#"<feGaussianBlur stdDeviation="{}"/>"#,
                    blur(*std_deviation)
                )
            }
            FilterPrimitive::DropShadowOnly {
                dx,
                dy,
                std_deviation,
                color,
                ..
            } => format!("{}/>", shadow(*dx, *dy, *std_deviation, *color)),
            FilterPrimitive::DropShadow {
                dx,
                dy,
                std_deviation,
                color,
                ..
            } => format!(
                r#"{} result="s"/><feMerge><feMergeNode in="s"/><feMergeNode in="SourceGraphic"/></feMerge>"#,
                shadow(*dx, *dy, *std_deviation, *color)
            ),
            _ => return None,
        };
        let id = self.id("f");
        let _ = write!(
            self.defs,
            r#"<filter id="{id}" filterUnits="userSpaceOnUse" x="0" y="0" width="{}" height="{}" color-interpolation-filters="sRGB">{body}</filter>"#,
            num(self.width),
            num(self.height)
        );
        Some(id)
    }

    fn clip(&mut self, path: &BezPath) {
        let id = self.id("c");
        let rule = if self.rule == Fill::EvenOdd {
            r#" clip-rule="evenodd""#
        } else {
            ""
        };
        let shape = self.transform * path.clone();
        let d = path_data(&shape);
        let _ = write!(
            self.defs,
            r#"<clipPath id="{id}" clipPathUnits="userSpaceOnUse"><path d="{d}"{rule}/></clipPath>"#
        );
        self.open_group(&format!(r#" clip-path="url(#{id})""#));
    }
}

impl Canvas for SvgCanvas {
    fn set_transform(&mut self, t: Affine) {
        if t != self.transform {
            self.transform = t;
            self.fill = None;
        }
    }
    fn set_paint(&mut self, paint: PaintType) {
        self.paint = paint;
        self.fill = None;
    }
    fn set_paint_transform(&mut self, t: Affine) {
        self.paint_transform = t;
        self.fill = None;
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        self.rule = rule;
    }
    fn set_blend_mode(&mut self, mode: BlendMode) {
        self.blend = mode;
    }
    fn fill_path(&mut self, path: &BezPath) {
        if self.over {
            return;
        }
        let d = path_data(&(self.transform * path.clone()));
        if d.is_empty() {
            return;
        }
        let rule = if self.rule == Fill::EvenOdd {
            r#" fill-rule="evenodd""#
        } else {
            ""
        };
        match self.blend.compose {
            Compose::SrcOver => {}
            Compose::DestOut => return self.erase(&d, rule),
            // The other Porter-Duff modes have no SVG fill equivalent: left out, counted.
            _ => {
                self.approximated += 1;
                return;
            }
        }
        // A mask erases the whole group, so drawing after an erase would be erased too.
        if self.open.last().is_some_and(|g| !g.erase.is_empty()) {
            self.approximated += 1;
        }
        let fill = self.fill_attrs();
        let blend = match blend_name(self.blend.mix) {
            Some(name) => format!(r#" style="mix-blend-mode:{name}""#),
            None => String::new(),
        };
        let _ = write!(self.body, r#"<path d="{d}"{fill}{rule}{blend}/>"#);
        self.check();
    }
    fn fill_rect(&mut self, rect: &Rect) {
        self.fill_path(&rect.to_path(0.1));
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        self.clip(path);
    }
    fn push_clip_path(&mut self, path: &BezPath) {
        self.clip(path);
    }
    fn pop_clip(&mut self) {
        self.close_group();
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        if opacity >= 1.0 {
            self.open_group("");
        } else {
            self.open_group(&format!(r#" opacity="{}""#, num(f64::from(opacity))));
        }
    }
    fn push_filter_layer(&mut self, filter: Filter, _reach: f64) {
        match self.filter(&filter) {
            Some(id) => self.open_group(&format!(r#" filter="url(#{id})""#)),
            None => {
                self.approximated += 1;
                self.open_group("");
            }
        }
    }
    fn pop_layer(&mut self) {
        self.close_group();
    }
    fn full(&self) -> bool {
        self.over
    }
}

/// The CSS `mix-blend-mode` of `mix`; `None` for normal.
fn blend_name(mix: Mix) -> Option<&'static str> {
    Some(match mix {
        Mix::Multiply => "multiply",
        Mix::Screen => "screen",
        Mix::Overlay => "overlay",
        Mix::Darken => "darken",
        Mix::Lighten => "lighten",
        Mix::ColorDodge => "color-dodge",
        Mix::ColorBurn => "color-burn",
        Mix::HardLight => "hard-light",
        Mix::SoftLight => "soft-light",
        Mix::Difference => "difference",
        Mix::Exclusion => "exclusion",
        Mix::Hue => "hue",
        Mix::Saturation => "saturation",
        Mix::Color => "color",
        Mix::Luminosity => "luminosity",
        _ => return None,
    })
}

/// SVG path data of `path`.
fn path_data(path: &BezPath) -> String {
    let mut d = String::new();
    for el in path.elements() {
        let _ = match el {
            PathEl::MoveTo(p) => write!(d, "M{} {}", num(p.x), num(p.y)),
            PathEl::LineTo(p) => write!(d, "L{} {}", num(p.x), num(p.y)),
            PathEl::QuadTo(a, p) => {
                write!(d, "Q{} {} {} {}", num(a.x), num(a.y), num(p.x), num(p.y))
            }
            PathEl::CurveTo(a, b, p) => write!(
                d,
                "C{} {} {} {} {} {}",
                num(a.x),
                num(a.y),
                num(b.x),
                num(b.y),
                num(p.x),
                num(p.y)
            ),
            PathEl::ClosePath => write!(d, "Z"),
        };
    }
    d
}

/// ` name="matrix(…)"`, or nothing for the identity.
fn matrix_attr(name: &str, t: Affine) -> String {
    if t == Affine::IDENTITY {
        return String::new();
    }
    let [a, b, c, d, e, f] = t.as_coeffs();
    format!(
        r#" {name}="matrix({} {} {} {} {} {})""#,
        num(a),
        num(b),
        num(c),
        num(d),
        num(e),
        num(f)
    )
}

/// ` name="a"` unless `a` is fully opaque.
fn opacity_attr(name: &str, a: f32) -> String {
    if a >= 1.0 {
        String::new()
    } else {
        format!(r#" {name}="{}""#, num(f64::from(a.max(0.0))))
    }
}

/// `#rrggbb` of sRGB components in 0..=1.
fn hex(r: f32, g: f32, b: f32) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

/// `v` with at most three decimals and no trailing zeros; non-finite values become 0.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_owned();
    }
    let mut s = format!("{:.3}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { "0".to_owned() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    #[test]
    fn numbers_are_short_and_finite() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.12345), "0.123");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(f64::NAN), "0");
    }

    #[test]
    fn an_erasing_fill_masks_its_group() {
        let mut c = SvgCanvas::new(10.0, 10.0, usize::MAX);
        c.push_opacity_layer(1.0);
        c.fill_path(&rect(0.0, 0.0, 10.0, 10.0));
        c.set_blend_mode(BlendMode::new(Mix::Normal, Compose::DestOut));
        c.fill_path(&rect(2.0, 2.0, 8.0, 8.0));
        c.set_blend_mode(BlendMode::default());
        c.pop_layer();
        let (svg, approximated, over) = c.finish();
        assert_eq!(approximated, 0);
        assert!(!over);
        assert!(svg.contains(r#"<g mask="url(#m1)">"#), "{svg}");
        assert!(svg.contains(r##"<mask id="m1""##), "{svg}");
        assert_eq!(svg.matches("<path").count(), 2, "{svg}");
    }

    #[test]
    fn an_erase_outside_any_group_is_counted() {
        let mut c = SvgCanvas::new(10.0, 10.0, usize::MAX);
        c.set_blend_mode(BlendMode::new(Mix::Normal, Compose::DestOut));
        c.fill_path(&rect(0.0, 0.0, 5.0, 5.0));
        let (svg, approximated, _) = c.finish();
        assert_eq!(approximated, 1);
        assert!(!svg.contains("<path"));
    }

    #[test]
    fn going_over_the_limit_drops_everything() {
        let mut c = SvgCanvas::new(10.0, 10.0, 64);
        for _ in 0..10 {
            c.fill_path(&rect(0.0, 0.0, 5.0, 5.0));
        }
        assert!(c.full());
        let (svg, _, over) = c.finish();
        assert!(over);
        assert!(svg.is_empty());
    }

    fn gradient(kind: impl Into<GradientKind>) -> PaintType {
        let g = vello_cpu::peniko::Gradient {
            kind: kind.into(),
            ..Default::default()
        }
        .with_stops(
            &[
                AlphaColor::<Srgb>::new([1.0, 0.0, 0.0, 1.0]),
                AlphaColor::<Srgb>::new([0.0, 0.0, 1.0, 0.5]),
            ][..],
        );
        PaintType::Gradient(g)
    }

    #[test]
    fn a_linear_gradient_is_defined_once_in_output_space() {
        let mut c = SvgCanvas::new(10.0, 10.0, usize::MAX);
        c.set_transform(Affine::translate((2.0, 0.0)));
        c.set_paint(gradient(vello_cpu::peniko::LinearGradientPosition::new(
            (0.0, 0.0),
            (10.0, 0.0),
        )));
        c.fill_path(&rect(0.0, 0.0, 4.0, 4.0));
        c.fill_path(&rect(4.0, 4.0, 8.0, 8.0));
        let (svg, approximated, _) = c.finish();
        assert_eq!(approximated, 0);
        assert_eq!(svg.matches("<linearGradient").count(), 1, "{svg}");
        assert!(
            svg.contains(r#"gradientTransform="matrix(1 0 0 1 2 0)""#),
            "{svg}"
        );
        assert!(
            svg.contains(r##"<stop offset="1" stop-color="#0000ff" stop-opacity="0.5"/>"##),
            "{svg}"
        );
        assert_eq!(svg.matches(r#"fill="url(#g1)""#).count(), 2, "{svg}");
    }

    #[test]
    fn a_sweep_gradient_is_counted_and_drawn_in_its_first_colour() {
        let mut c = SvgCanvas::new(10.0, 10.0, usize::MAX);
        c.set_paint(gradient(vello_cpu::peniko::SweepGradientPosition::new(
            (5.0, 5.0),
            0.0,
            std::f32::consts::TAU,
        )));
        c.fill_path(&rect(0.0, 0.0, 4.0, 4.0));
        let (svg, approximated, _) = c.finish();
        assert_eq!(approximated, 1);
        assert!(svg.contains(r##"fill="#ff0000""##), "{svg}");
    }

    #[test]
    fn the_view_crops_through_the_view_box() {
        let mut c = SvgCanvas::new(100.0, 100.0, usize::MAX);
        c.fill_path(&rect(10.0, 20.0, 15.0, 25.0));
        let (svg, _, _) = c.finish_view(Rect::new(10.0, 20.0, 50.0, 50.0));
        assert!(
            svg.contains(r#"width="40" height="30" viewBox="10 20 40 30""#),
            "{svg}"
        );
    }
}
