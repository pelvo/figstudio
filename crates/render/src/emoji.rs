//! Emoji markers use a colour strike or a counted outline fallback, never their blob.
use std::sync::Arc;

use figstudio_fig::Val;
use vello_cpu::kurbo::{Affine, BezPath, Rect, Shape};
use vello_cpu::peniko::{Fill, ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, PaintType, Pixmap};

use crate::{Ctx, geom::GeomPath, paint, text};

#[derive(Clone, Debug)]
#[cfg_attr(
    not(feature = "system-fonts"),
    expect(dead_code, reason = "no font decoder in wasm")
)]
pub(crate) enum Glyph {
    Bitmap {
        image: Arc<Pixmap>,
        x: f64,
        y: f64,
        ppem: u16,
        /// sbix contour origin in em units, y up; zero for CBDT/contourless sbix.
        origin: (f64, f64),
    },
    /// Outline in em units, y up.
    Mono(BezPath),
}

pub(crate) struct Placed {
    glyph: Glyph,
    transform: Affine,
    pub(crate) rect: Rect,
}

fn fill(data: &Val) -> Option<&Val> {
    data.get("fillPaints")?
        .items()
        .iter()
        .find(|p| p.flag("visible") != Some(false) && p.num("opacity").unwrap_or(1.0) > 0.0)
}

impl Ctx<'_> {
    /// Shared classification and placement for drawing and metadata, with no canvas work.
    pub(crate) fn emojis(&mut self, data: &Val, world: Affine) -> Vec<Placed> {
        if fill(data).is_none() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for glyph in text::visible_glyphs(data)
            .iter()
            .filter(|g| text::is_emoji(g))
        {
            let mut bases = glyph
                .get("emojiCodePoints")
                .map(Val::items)
                .unwrap_or_default()
                .iter()
                .filter_map(Val::as_u32)
                .filter_map(char::from_u32)
                .filter(|c| !matches!(*c as u32, 0x200D | 0xFE0E | 0xFE0F | 0x1F3FB..=0x1F3FF));
            let base = bases.next();
            let sequence = bases.next().is_some();
            let size = f64::from(glyph.num("fontSize").unwrap_or(12.0));
            let pos = glyph.get("position");
            let x = f64::from(pos.and_then(|p| p.num("x")).unwrap_or(0.0));
            let y = f64::from(pos.and_then(|p| p.num("y")).unwrap_or(0.0));
            if !size.is_finite() || size <= 0.0 || !x.is_finite() || !y.is_finite() {
                self.stats.emoji_skipped += 1;
                continue;
            }
            let pixels = (size * paint::linear_scale(world))
                .round()
                .clamp(1.0, f64::from(u16::MAX)) as u16;
            let shaped = self
                .emoji_source
                .source
                .as_ref()
                .and_then(|_| {
                    let points = glyph.get("emojiCodePoints")?.items();
                    // Bound keys before allocating; never truncate a cluster to fit the cache.
                    if points.len() > 256 {
                        return None;
                    }
                    points
                        .iter()
                        .map(|p| p.as_u32().and_then(char::from_u32))
                        .collect::<Option<String>>()
                })
                .and_then(|cluster| self.emoji_source.raster(&cluster, pixels));
            let fully_shaped = shaped.is_some();
            let resolved = shaped.or_else(|| {
                base.and_then(|ch| self.fonts.as_mut().and_then(|f| f.emoji(ch, pixels)))
            });
            let Some(resolved) = resolved else {
                self.stats.emoji_skipped += 1;
                continue;
            };
            self.stats.emoji_approximated +=
                usize::from((sequence && !fully_shaped) || matches!(resolved, Glyph::Mono(_)));
            let (transform, rect) = match &resolved {
                Glyph::Bitmap {
                    image,
                    x: ox,
                    y: oy,
                    ppem,
                    origin,
                } => {
                    let scale = size / f64::from(*ppem);
                    let left = x + origin.0 * size + *ox * scale;
                    let top = y - origin.1 * size - (*oy + f64::from(image.height())) * scale;
                    let rect = Rect::new(
                        left,
                        top,
                        left + f64::from(image.width()) * scale,
                        top + f64::from(image.height()) * scale,
                    );
                    (Affine::translate((left, top)) * Affine::scale(scale), rect)
                }
                Glyph::Mono(outline) => {
                    let transform =
                        Affine::translate((x, y)) * Affine::scale_non_uniform(size, -size);
                    let rect = transform.transform_rect_bbox(outline.bounding_box());
                    (transform, rect)
                }
            };
            out.push(Placed {
                glyph: resolved,
                transform,
                rect,
            });
        }
        out
    }

    pub(crate) fn draw_placed_emojis(
        &mut self,
        data: &Val,
        world: Affine,
        w: f64,
        h: f64,
        emojis: Vec<Placed>,
    ) {
        let Some(fill) = fill(data) else {
            return;
        };
        for Placed {
            glyph,
            transform,
            rect,
        } in emojis
        {
            match glyph {
                Glyph::Bitmap { image, .. } => {
                    self.rc.set_transform(world);
                    self.rc.set_fill_rule(Fill::NonZero);
                    self.rc.set_paint(PaintType::Image(Image {
                        image: ImageSource::Pixmap(image),
                        sampler: ImageSampler {
                            quality: ImageQuality::Medium,
                            alpha: (fill.num("opacity").unwrap_or(1.0)
                                * fill.get("color").and_then(|c| c.num("a")).unwrap_or(1.0))
                            .clamp(0.0, 1.0),
                            ..Default::default()
                        },
                    }));
                    self.rc.set_paint_transform(transform);
                    self.rc.fill_path(&rect.to_path(0.1));
                    self.rc.set_paint_transform(Affine::IDENTITY);
                }
                Glyph::Mono(mut outline) => {
                    outline.apply_affine(transform);
                    self.paint_paths(data, "fillPaints", &[GeomPath::plain(outline)], world, w, h);
                }
            }
        }
    }
}
