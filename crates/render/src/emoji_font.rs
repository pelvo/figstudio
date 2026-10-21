//! Native colour font lookup and bounded PNG decoding.
use std::{io::Cursor, sync::Arc};

use fontdb::{Database, Family, Query};
use ttf_parser::{Face, GlyphId, RasterGlyphImage, RasterImageFormat};
use vello_cpu::{
    PixelMetadata, Pixmap,
    kurbo::{Affine, BezPath},
    peniko::ImageAlphaType,
};

use crate::{emoji::Glyph, text::Builder};

const FAMILIES: &[&str] = &[
    "Apple Color Emoji",
    "Noto Color Emoji",
    "Segoe UI Emoji",
    "Twemoji Mozilla",
    "EmojiOne Color",
];

pub(crate) fn resolve(db: &Database, ch: char, pixels: u16) -> Option<Glyph> {
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
                let id = face.glyph_index(ch)?;
                if let Some((strike, image, sbix)) = closest_strike(&face, id, pixels) {
                    let origin = if sbix {
                        face.glyph_bounding_box(id).map(|b| {
                            let upem = f64::from(face.units_per_em());
                            (f64::from(b.x_min) / upem, f64::from(b.y_min) / upem)
                        })
                    } else {
                        None
                    };
                    return Some(Glyph::Bitmap {
                        image,
                        x: f64::from(strike.x),
                        y: f64::from(strike.y),
                        ppem: strike.pixels_per_em,
                        origin: origin.unwrap_or_default(),
                    });
                }
                let mut outline = monochrome(&face, id)?;
                outline.apply_affine(Affine::scale(1.0 / f64::from(face.units_per_em())));
                Some(Glyph::Mono(outline))
            })
            .flatten();
        match glyph {
            Some(Glyph::Bitmap { .. }) => return glyph,
            Some(Glyph::Mono(_)) if mono.is_none() => mono = glyph,
            _ => {}
        }
    }
    mono
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

/// Enumerate sbix directly; CBDT's public selector has an eight-bit ppem domain.
/// Unsupported or corrupt strikes must not hide usable PNGs in the same face.
fn closest_strike<'a>(
    face: &'a Face<'a>,
    glyph: GlyphId,
    pixels: u16,
) -> Option<(RasterGlyphImage<'a>, Arc<Pixmap>, bool)> {
    let sbix = face
        .tables()
        .sbix
        .into_iter()
        .flat_map(|t| t.strikes)
        .filter_map(|s| s.get(glyph))
        .map(|s| (s, true));
    let cbdt = face
        .tables()
        .cbdt
        .into_iter()
        .flat_map(|t| (1..=255).filter_map(move |ppem| t.get(glyph, ppem)))
        .map(|s| (s, false));
    let mut candidates: Vec<_> = sbix
        .chain(cbdt)
        .filter(|(s, _)| s.format == RasterImageFormat::PNG && s.pixels_per_em > 0)
        .collect();
    candidates.sort_by_key(|(s, _)| {
        (
            pixels.abs_diff(s.pixels_per_em),
            std::cmp::Reverse(s.pixels_per_em),
        )
    });
    candidates.dedup_by_key(|(s, sbix)| (*sbix, s.pixels_per_em, s.data.as_ptr()));
    candidates.into_iter().find_map(|(s, sbix)| {
        let image = decode_png(s.data)?;
        (image.width() == s.width && image.height() == s.height).then_some((s, image, sbix))
    })
}

fn decode_png(bytes: &[u8]) -> Option<Arc<Pixmap>> {
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
mod tests {
    use super::*;
    use image::ImageEncoder as _;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|i| [i as u8, 80, 200, 255])
            .collect();
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn synthetic_sbix_decodes_colour_and_chooses_the_nearest_strike() {
        let data = crate::text_test_font::colour_emoji(&png(2, 2));
        let face = Face::parse(&data, 0).unwrap();
        let id = face.glyph_index('❤').unwrap();
        assert_eq!(closest_strike(&face, id, 30).unwrap().0.pixels_per_em, 16);
        assert_eq!(closest_strike(&face, id, 50).unwrap().0.pixels_per_em, 64);
        let mut db = Database::new();
        db.load_font_data(data);
        let Some(Glyph::Bitmap { image: pixmap, .. }) = resolve(&db, '❤', 30) else {
            panic!("missing sbix colour glyph")
        };
        assert_eq!(pixmap.width(), 2);
        assert_eq!(
            pixmap.data_as_u8_slice(),
            &[
                0, 80, 200, 255, 1, 80, 200, 255, 2, 80, 200, 255, 3, 80, 200, 255
            ]
        );
        assert!(resolve(&db, 'A', 30).is_none());
    }

    #[test]
    fn malformed_or_oversized_strikes_are_not_decoded() {
        assert!(decode_png(b"not a PNG").is_none());
        assert!(decode_png(&png(1025, 1)).is_none());
        let mut db = Database::new();
        db.load_font_data(crate::text_test_font::colour_emoji(b"not a PNG"));
        assert!(resolve(&db, '❤', 30).is_none());
    }
}
