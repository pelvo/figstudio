//! Figma paints (solid, gradients, images) to Vello paints.

use std::collections::HashMap;
use std::sync::Arc;

use figview_fig::{Document, Val, hex};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, Point, Rect};
use vello_cpu::peniko::{
    ColorStop, ColorStops, Extend, Gradient, ImageAlphaType, ImageQuality, ImageSampler, LinearGradientPosition,
    RadialGradientPosition, SweepGradientPosition,
};
use vello_cpu::{Image, ImageSource, PaintType, PixelMetadata, Pixmap};

use crate::Stats;

/// Largest image dimension handed to the rasterizer; bigger images are downscaled once.
const MAX_IMAGE_SIDE: u32 = 8192;

#[derive(Default)]
pub struct ImageCache {
    decoded: HashMap<String, Option<Arc<Pixmap>>>,
}

impl ImageCache {
    pub fn get(&mut self, doc: &Document, hash: &str) -> Option<Arc<Pixmap>> {
        if let Some(cached) = self.decoded.get(hash) {
            return cached.clone();
        }
        let decoded = doc.images.get(hash).and_then(|bytes| decode_image(bytes));
        self.decoded.insert(hash.to_string(), decoded.clone());
        decoded
    }
}

fn decode_image(bytes: &[u8]) -> Option<Arc<Pixmap>> {
    let mut img = image::load_from_memory(bytes).ok()?;
    if img.width() > MAX_IMAGE_SIDE || img.height() > MAX_IMAGE_SIDE {
        img = img.thumbnail(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE);
    }
    let rgba = img.to_rgba8();
    let (w, h) = (u16::try_from(rgba.width()).ok()?, u16::try_from(rgba.height()).ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    let meta = PixelMetadata { may_have_transparency: true, alpha_type: ImageAlphaType::Alpha };
    Some(Arc::new(Pixmap::from_parts(rgba.into_raw(), w, h, meta)))
}

pub fn color(value: Option<&Val>, opacity: f32) -> Option<AlphaColor<Srgb>> {
    let c = value?;
    let ch = |k: &str, d: f32| c.num(k).unwrap_or(d).clamp(0.0, 1.0);
    Some(AlphaColor::new([ch("r", 0.0), ch("g", 0.0), ch("b", 0.0), ch("a", 1.0) * opacity.clamp(0.0, 1.0)]))
}

/// A paint ready for the render context, plus the transform that positions it in node space.
pub struct ResolvedPaint {
    pub paint: PaintType,
    pub transform: Affine,
    /// Area the paint is limited to, in node space (image `FIT` leaves the rest empty).
    pub clip: Option<Rect>,
}

/// Resolves one Figma paint for a node of size `w × h`. Returns `None` for hidden or empty paints.
pub fn resolve(
    p: &Val,
    w: f64,
    h: f64,
    doc: &Document,
    images: &mut ImageCache,
    stats: &mut Stats,
) -> Option<ResolvedPaint> {
    if p.flag("visible") == Some(false) {
        return None;
    }
    let opacity = p.num("opacity").unwrap_or(1.0);
    if opacity <= 0.0 {
        return None;
    }
    let kind = p.text("type").unwrap_or("SOLID");
    // Gradient and image transforms map node-normalized space into paint space.
    let unit_to_node = Affine::scale_non_uniform(w.max(1e-6), h.max(1e-6));
    let paint_inverse = crate::geom::affine(p.get("transform")).inverse();
    match kind {
        "SOLID" => Some(ResolvedPaint {
            paint: PaintType::Solid(color(p.get("color"), opacity)?),
            transform: Affine::IDENTITY,
            clip: None,
        }),
        "GRADIENT_LINEAR" | "GRADIENT_RADIAL" | "GRADIENT_ANGULAR" | "GRADIENT_DIAMOND" => {
            let mut stops = ColorStops::default();
            for stop in p.get("stops").map(Val::items).unwrap_or_default() {
                let Some(c) = color(stop.get("color"), opacity) else { continue };
                stops.push(ColorStop::from((stop.num("position").unwrap_or(0.0).clamp(0.0, 1.0), c)));
            }
            if stops.is_empty() {
                return None;
            }
            let kind = match kind {
                "GRADIENT_LINEAR" => LinearGradientPosition { start: Point::new(0.0, 0.5), end: Point::new(1.0, 0.5) }.into(),
                "GRADIENT_ANGULAR" => SweepGradientPosition {
                    center: Point::new(0.5, 0.5),
                    start_angle: 0.0,
                    end_angle: std::f32::consts::TAU,
                }
                .into(),
                _ => {
                    if kind == "GRADIENT_DIAMOND" {
                        stats.approximated_paints += 1;
                    }
                    RadialGradientPosition {
                        start_center: Point::new(0.5, 0.5),
                        start_radius: 0.0,
                        end_center: Point::new(0.5, 0.5),
                        end_radius: 0.5,
                    }
                    .into()
                }
            };
            let gradient = Gradient { kind, stops, extend: Extend::Pad, ..Default::default() };
            Some(ResolvedPaint { paint: PaintType::Gradient(gradient), transform: unit_to_node * paint_inverse, clip: None })
        }
        "IMAGE" => {
            let hash = p.path(&["image", "hash"]).and_then(Val::as_bytes).map(hex);
            let pixmap = hash.as_deref().and_then(|h| images.get(doc, h));
            let Some(pixmap) = pixmap else {
                stats.missing_images += 1;
                // Neutral placeholder so the layer stays visible.
                return Some(ResolvedPaint {
                    paint: PaintType::Solid(AlphaColor::new([0.82, 0.83, 0.86, opacity])),
                    transform: Affine::IDENTITY,
                    clip: None,
                });
            };
            let (iw, ih) = (f64::from(pixmap.width()), f64::from(pixmap.height()));
            let mode = p.text("imageScaleMode").unwrap_or("STRETCH");
            let mut extend = Extend::Pad;
            let mut clip = None;
            let transform = match mode {
                "FILL" | "FIT" => {
                    let s = if mode == "FILL" { (w / iw).max(h / ih) } else { (w / iw).min(h / ih) };
                    let (x, y) = ((w - iw * s) / 2.0, (h - ih * s) / 2.0);
                    if mode == "FIT" {
                        clip = Some(Rect::new(x, y, x + iw * s, y + ih * s));
                    }
                    Affine::translate((x, y)) * Affine::scale(s)
                }
                "TILE" => {
                    extend = Extend::Repeat;
                    Affine::scale(f64::from(p.num("scale").unwrap_or(1.0)).max(1e-3))
                }
                _ => unit_to_node * paint_inverse * Affine::scale_non_uniform(1.0 / iw, 1.0 / ih),
            };
            let image = Image {
                image: ImageSource::Pixmap(pixmap),
                sampler: ImageSampler {
                    x_extend: extend,
                    y_extend: extend,
                    quality: ImageQuality::Medium,
                    alpha: opacity,
                },
            };
            Some(ResolvedPaint { paint: PaintType::Image(image), transform, clip })
        }
        _ => {
            stats.unsupported_paints += 1;
            None
        }
    }
}
