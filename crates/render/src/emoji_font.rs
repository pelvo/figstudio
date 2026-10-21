//! Native colour font lookup and bounded PNG decoding.
use std::{io::Cursor, sync::Arc};

use fontdb::{Database, Family, Query};
use ttf_parser::{Face, GlyphId, RasterGlyphImage, RasterImageFormat};
use vello_cpu::{
    PixelMetadata, Pixmap,
    kurbo::{Affine, BezPath},
    peniko::ImageAlphaType,
};

use crate::text::Builder;

const FAMILIES: &[&str] = &[
    "Apple Color Emoji",
    "Noto Color Emoji",
    "Segoe UI Emoji",
    "Twemoji Mozilla",
    "EmojiOne Color",
];

#[path = "emoji_font_cache.rs"]
mod cache;
pub(crate) use cache::CACHED_BYTES;
pub(crate) use cache::Resolved;

pub(crate) fn resolve(db: &Database, ch: char, pixels: u16) -> Option<Resolved> {
    #[cfg(test)]
    RESOLVES.set(RESOLVES.get() + 1);
    let mut mono = None;
    for name in FAMILIES.iter().chain(crate::text::SYMBOL_FAMILIES) {
        let Some(id) = db.query(&Query {
            families: &[Family::Name(name)],
            ..Default::default()
        }) else {
            continue;
        };
        let glyph = db
            .with_face_data(id, |bytes, index| {
                let face = Face::parse(bytes, index).ok()?;
                let glyph_id = face.glyph_index(ch)?;
                from_face(db, &face, id, glyph_id, pixels)
            })
            .flatten();
        if let Some(resolved) = glyph {
            if resolved.is_bitmap() {
                return Some(resolved);
            }
            if mono.is_none() {
                mono = Some(resolved);
            }
        }
    }
    mono
}

fn from_face(
    db: &Database,
    face: &Face<'_>,
    font: fontdb::ID,
    id: GlyphId,
    pixels: u16,
) -> Option<Resolved> {
    let mut resolved = Resolved::new(face, font, id);
    if resolved.glyph(db, pixels).is_some() {
        return Some(resolved);
    }
    let mut outline = monochrome(face, id)?;
    outline.apply_affine(Affine::scale(1.0 / f64::from(face.units_per_em())));
    Some(resolved.with_mono(outline))
}

fn monochrome(face: &Face<'_>, id: GlyphId) -> Option<BezPath> {
    let mut outline = Builder(BezPath::new());
    if face.outline_glyph(id, &mut outline).is_some() && !outline.0.elements().is_empty() {
        return Some(outline.0);
    }
    let mut layers = MonoLayers {
        face,
        path: BezPath::new(),
        transforms: vec![Affine::IDENTITY],
    };
    face.paint_color_glyph(id, 0, ttf_parser::RgbaColor::new(0, 0, 0, 255), &mut layers)?;
    (!layers.path.elements().is_empty()).then_some(layers.path)
}

/// COLR layers flattened to monochrome; palette, clipping and compositing are approximated.
struct MonoLayers<'a, 'b> {
    face: &'a Face<'b>,
    path: BezPath,
    transforms: Vec<Affine>,
}

impl<'b> ttf_parser::colr::Painter<'b> for MonoLayers<'_, 'b> {
    fn outline_glyph(&mut self, id: GlyphId) {
        let mut outline = Builder(BezPath::new());
        if self.face.outline_glyph(id, &mut outline).is_some() {
            outline
                .0
                .apply_affine(self.transforms.last().copied().unwrap_or(Affine::IDENTITY));
            self.path.extend(outline.0.elements().iter().copied());
        }
    }
    fn paint(&mut self, _: ttf_parser::colr::Paint<'b>) {}
    fn push_clip(&mut self) {}
    fn push_clip_box(&mut self, _: ttf_parser::colr::ClipBox) {}
    fn pop_clip(&mut self) {}
    fn push_layer(&mut self, _: ttf_parser::colr::CompositeMode) {}
    fn pop_layer(&mut self) {}
    fn push_transform(&mut self, t: ttf_parser::Transform) {
        let parent = self.transforms.last().copied().unwrap_or(Affine::IDENTITY);
        self.transforms
            .push(parent * Affine::new([t.a, t.b, t.c, t.d, t.e, t.f].map(f64::from)));
    }
    fn pop_transform(&mut self) {
        self.transforms.pop();
    }
}

/// Walk every size record once, including distinct records at the same ppem.
/// Unsupported or corrupt strikes must not hide usable PNGs in the same face.
fn strikes<'a>(face: &'a Face<'a>, glyph: GlyphId) -> Vec<(RasterGlyphImage<'a>, bool)> {
    let sbix = face
        .tables()
        .sbix
        .into_iter()
        .flat_map(|t| t.strikes)
        .filter_map(|s| s.get(glyph))
        .map(|s| (s, true));
    let embedded = crate::emoji_bitmap::strikes(face, glyph)
        .into_iter()
        .map(|s| (s, false));
    sbix.chain(embedded)
        .filter(|(s, _)| {
            s.format == RasterImageFormat::PNG
                && s.pixels_per_em > 0
                && s.data.len() <= 4 * 1024 * 1024
        })
        .collect()
}

#[cfg(test)]
thread_local! {
    pub(crate) static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static RESOLVES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn decode_png(bytes: &[u8]) -> Option<Arc<Pixmap>> {
    #[cfg(test)]
    DECODES.set(DECODES.get() + 1);
    // Font strikes are untrusted: bound encoded data, PNG dimensions and decoder allocations.
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.into_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    Some(Arc::new(Pixmap::from_parts(
        rgba.into_raw(),
        w as u16,
        h as u16,
        PixelMetadata {
            alpha_type: ImageAlphaType::Alpha,
            ..Default::default()
        },
    )))
}

#[cfg(test)]
#[path = "emoji_font_tests.rs"]
mod tests;
