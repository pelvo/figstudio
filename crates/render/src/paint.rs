//! Figma paints (solid, gradients, images) to Vello paints.

use figstudio_fig::{Document, Val, hex};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, Point, Rect};
use vello_cpu::peniko::{
    ColorStop, ColorStops, Extend, Gradient, ImageQuality, ImageSampler, LinearGradientPosition,
    RadialGradientPosition, SweepGradientPosition,
};
use vello_cpu::{Image, ImageSource, PaintType};

use crate::Stats;
use crate::images::{Decoded, ImageRequest, Images, cache_key, clamp_level, level_for};

pub fn color(value: Option<&Val>, opacity: f32) -> Option<AlphaColor<Srgb>> {
    let c = value?;
    let ch = |k: &str, d: f32| c.num(k).unwrap_or(d).clamp(0.0, 1.0);
    Some(AlphaColor::new([
        ch("r", 0.0),
        ch("g", 0.0),
        ch("b", 0.0),
        ch("a", 1.0) * opacity.clamp(0.0, 1.0),
    ]))
}

/// A paint ready for the render context, plus the transform that positions it in node space.
pub struct ResolvedPaint {
    pub paint: PaintType,
    pub transform: Affine,
    /// Area the paint is limited to, in node space (image `FIT` leaves the rest empty).
    pub clip: Option<Rect>,
    /// Hash of the image file an image paint draws, when the pixels are that whole file
    /// (decoded at full size, or as large as allowed), so an SVG can embed the file itself.
    pub source: Option<String>,
}

/// Where image paints come from, and where requests for unread images go.
pub(crate) struct ImageCtx<'a> {
    pub images: &'a Images,
    pub wanted: &'a mut Vec<ImageRequest>,
    /// Output pixels per node unit (the largest scale of the node's transform).
    pub screen: f64,
    /// Always the full image at full size, never a thumbnail or a smaller level (an SVG,
    /// which may be shown at any size).
    pub full: bool,
    pub shaders: Option<&'a dyn crate::shader::ShaderFills>,
    /// Shader clock (ms): the playhead's time, 0 at rest.
    pub time_ms: f64,
}

/// Resolves one Figma paint for a node of size `w × h`. Returns `None` for hidden or empty paints.
pub(crate) fn resolve(
    p: &Val,
    w: f64,
    h: f64,
    doc: &Document,
    img: &mut ImageCtx<'_>,
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
            source: None,
        }),
        "GRADIENT_LINEAR" | "GRADIENT_RADIAL" | "GRADIENT_ANGULAR" | "GRADIENT_DIAMOND" => {
            let mut stops = ColorStops::default();
            for stop in p.get("stops").map(Val::items).unwrap_or_default() {
                let Some(c) = color(stop.get("color"), opacity) else {
                    continue;
                };
                stops.push(ColorStop::from((
                    stop.num("position").unwrap_or(0.0).clamp(0.0, 1.0),
                    c,
                )));
            }
            if stops.is_empty() {
                return None;
            }
            let kind = match kind {
                "GRADIENT_LINEAR" => LinearGradientPosition {
                    start: Point::new(0.0, 0.5),
                    end: Point::new(1.0, 0.5),
                }
                .into(),
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
            let gradient = Gradient {
                kind,
                stops,
                extend: Extend::Pad,
                ..Default::default()
            };
            Some(ResolvedPaint {
                paint: PaintType::Gradient(gradient),
                transform: unit_to_node * paint_inverse,
                clip: None,
                source: None,
            })
        }
        "IMAGE" => {
            let Some((decoded, natural, source)) = pick_image(p, w, h, doc, img, stats) else {
                // Neutral placeholder so the layer stays visible.
                return Some(ResolvedPaint {
                    paint: PaintType::Solid(AlphaColor::new([0.82, 0.83, 0.86, opacity])),
                    transform: Affine::IDENTITY,
                    clip: None,
                    source: None,
                });
            };
            let (layout, extend, clip) = image_layout(p, w, h, natural);
            // The decoded copy may be smaller than the original (a level or the thumbnail).
            let pixmap = decoded.pixmap;
            let fit = Affine::scale_non_uniform(
                natural.0 / f64::from(pixmap.width()),
                natural.1 / f64::from(pixmap.height()),
            );
            let image = Image {
                image: ImageSource::Pixmap(pixmap),
                sampler: ImageSampler {
                    x_extend: extend,
                    y_extend: extend,
                    quality: ImageQuality::Medium,
                    alpha: opacity,
                },
            };
            Some(ResolvedPaint {
                paint: PaintType::Image(image),
                transform: layout * fit,
                clip,
                source,
            })
        }
        "CUSTOM" => {
            let (pw, ph) = crate::shader::pixel_size(w, h);
            let shaded = img.shaders.map_or(crate::shader::Shaded::Unavailable, |s| {
                s.fill(doc, p, pw, ph, img.time_ms)
            });
            let pixmap = match shaded {
                crate::shader::Shaded::Drawn(pixmap) => pixmap,
                crate::shader::Shaded::Pending => {
                    stats.images_loading += 1;
                    return None;
                }
                crate::shader::Shaded::Unavailable => {
                    stats.unsupported_paints += 1;
                    return None;
                }
            };
            let fit = Affine::scale_non_uniform(
                w / f64::from(pixmap.width()),
                h / f64::from(pixmap.height()),
            );
            Some(ResolvedPaint {
                paint: PaintType::Image(Image {
                    image: ImageSource::Pixmap(pixmap),
                    sampler: ImageSampler {
                        x_extend: Extend::Pad,
                        y_extend: Extend::Pad,
                        quality: ImageQuality::Medium,
                        alpha: opacity,
                    },
                }),
                transform: fit,
                clip: None,
                source: None,
            })
        }
        _ => {
            stats.unsupported_paints += 1;
            None
        }
    }
}

/// Placement of an image of `natural` size (pixels) in a `w × h` node: the transform from
/// image pixels to node space, the edge mode and the clip.
fn image_layout(p: &Val, w: f64, h: f64, natural: (f64, f64)) -> (Affine, Extend, Option<Rect>) {
    let (iw, ih) = (natural.0.max(1e-6), natural.1.max(1e-6));
    match p.text("imageScaleMode").unwrap_or("STRETCH") {
        mode @ ("FILL" | "FIT") => {
            let s = if mode == "FILL" {
                (w / iw).max(h / ih)
            } else {
                (w / iw).min(h / ih)
            };
            let (x, y) = ((w - iw * s) / 2.0, (h - ih * s) / 2.0);
            let clip = (mode == "FIT").then(|| Rect::new(x, y, x + iw * s, y + ih * s));
            (
                Affine::translate((x, y)) * Affine::scale(s),
                Extend::Pad,
                clip,
            )
        }
        "TILE" => (
            Affine::scale(f64::from(p.num("scale").unwrap_or(1.0)).max(1e-3)),
            Extend::Repeat,
            None,
        ),
        _ => {
            let unit_to_node = Affine::scale_non_uniform(w.max(1e-6), h.max(1e-6));
            let paint_inverse = crate::geom::affine(p.get("transform")).inverse();
            (
                unit_to_node * paint_inverse * Affine::scale_non_uniform(1.0 / iw, 1.0 / ih),
                Extend::Pad,
                None,
            )
        }
    }
}

/// The largest length scale of a transform's linear part.
pub(crate) fn linear_scale(t: Affine) -> f64 {
    let [a, b, c, d, _, _] = t.as_coeffs();
    a.hypot(b).max(c.hypot(d))
}

/// Long side, in output pixels, up to which a paint's Figma thumbnail is drawn instead of the
/// image. Figma's thumbnails are at most 512 px; one under 256 px only exists for an image
/// that is no bigger, so this never draws anything blurrier than the image itself would be.
const THUMBNAIL_UP_TO: f64 = 256.0;

/// Chooses what to draw for an image paint: the Figma thumbnail when the image is drawn small,
/// otherwise the image decoded at the level the view needs. The choice depends only on the
/// paint and the view, so a frame looks the same whatever was decoded before. Anything not
/// decoded yet is requested; meanwhile the nearest cached copy (or a placeholder) is drawn.
/// Returns the image, the original's size and the hash of the file when the image is that
/// file at the finest level, or `None` for a placeholder.
fn pick_image(
    p: &Val,
    w: f64,
    h: f64,
    doc: &Document,
    img: &mut ImageCtx<'_>,
    stats: &mut Stats,
) -> Option<(Decoded, (f64, f64), Option<String>)> {
    let hash_of = |key: &str| p.path(&[key, "hash"]).and_then(Val::as_bytes).map(hex);
    let key = |h: &str| cache_key(doc, h);
    let available = |h: Option<String>| {
        h.filter(|h| {
            (doc.images.contains_key(h) || doc.lazy_images.contains_key(h))
                && !img.images.has_failed(&key(h))
        })
    };
    let full = available(hash_of("image"));
    let thumb = available(hash_of("imageThumbnail")).filter(|t| Some(t) != full.as_ref());
    // Size of the original: stored on the paint by Figma, else learnt from a decoded copy.
    let original = match (p.num("originalImageWidth"), p.num("originalImageHeight")) {
        (Some(w), Some(h)) if w >= 1.0 && h >= 1.0 => Some((w as u32, h as u32)),
        _ => full
            .as_deref()
            .and_then(|f| img.images.nearest(&key(f), 0))
            .map(|d| d.natural),
    };
    let natural = original.map(|(w, h)| (f64::from(w), f64::from(h)));
    // Output pixels per image pixel, and the image's long side on screen.
    let scale = natural.map(|n| img.screen * linear_scale(image_layout(p, w, h, n).0));
    let long_px = natural.zip(scale).map(|(n, s)| n.0.max(n.1) * s);

    // What should be drawn: (hash, level).
    let target = match (&full, &thumb) {
        (_, Some(t))
            if full.is_none() || (!img.full && long_px.is_some_and(|px| px <= THUMBNAIL_UP_TO)) =>
        {
            Some((t.clone(), 0))
        }
        (Some(f), _) => {
            let level = if img.full {
                0
            } else {
                scale.map_or(0, level_for)
            };
            Some((f.clone(), original.map_or(level, |o| clamp_level(o, level))))
        }
        _ => None,
    };
    let mut loading = false;
    let chosen = target.and_then(|(hash, level)| {
        let decoded = img
            .images
            .get(&key(&hash), level)
            .or_else(|| fetch(&hash, level, doc, img, &mut loading));
        if let Some(d) = decoded {
            let source = d.finest.then_some(hash);
            return Some((d, source));
        }
        // Not decoded yet: the nearest copy of the image, else of the thumbnail.
        let fallback =
            |h: &Option<String>, l| h.as_deref().and_then(|h| img.images.nearest(&key(h), l));
        fallback(&full, level)
            .or_else(|| fallback(&thumb, 0))
            .map(|d| (d, None))
    });
    if loading {
        stats.images_loading += 1;
    } else if chosen.is_none() {
        stats.missing_images += 1;
    }
    let (chosen, source) = chosen?;
    let natural = natural.unwrap_or((f64::from(chosen.natural.0), f64::from(chosen.natural.1)));
    Some((chosen, natural, source))
}

/// Decodes an image whose bytes are in the document, or asks for an unread one.
fn fetch(
    hash: &str,
    level: u8,
    doc: &Document,
    img: &mut ImageCtx<'_>,
    loading: &mut bool,
) -> Option<Decoded> {
    let key = cache_key(doc, hash);
    if let Some(bytes) = doc.images.get(hash) {
        img.images.insert_encoded(&key, bytes, level);
        return img.images.get(&key, level);
    }
    let entry = doc.lazy_images.get(hash)?;
    *loading = true;
    if img.images.begin_request(&key, level) {
        img.wanted.push(ImageRequest {
            hash: hash.to_string(),
            key,
            level,
            entry: entry.clone(),
        });
    }
    None
}
