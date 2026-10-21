//! Native colour font lookup and bounded PNG decoding.
use std::{
    io::Cursor,
    sync::{Arc, OnceLock},
};

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

pub(crate) struct Resolved {
    strikes: Vec<Strike>,
    mono: Option<Glyph>,
}

struct Strike {
    data: Vec<u8>,
    image: OnceLock<Option<Arc<Pixmap>>>,
    x: i16,
    y: i16,
    ppem: u16,
    width: u16,
    height: u16,
    origin: (f64, f64),
}

impl Resolved {
    pub(crate) fn glyph(&self, pixels: u16) -> Option<Glyph> {
        let mut strikes: Vec<_> = self.strikes.iter().collect();
        strikes.sort_by_key(|s| (pixels.abs_diff(s.ppem), std::cmp::Reverse(s.ppem)));
        for s in strikes {
            let image = s.image.get_or_init(|| {
                let image = decode_png(&s.data)?;
                (image.width() == s.width && image.height() == s.height).then_some(image)
            });
            if let Some(image) = image {
                return Some(Glyph::Bitmap {
                    image: image.clone(),
                    x: f64::from(s.x),
                    y: f64::from(s.y),
                    ppem: s.ppem,
                    origin: s.origin,
                });
            }
        }
        self.mono.clone()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.strikes
            .iter()
            .map(|s| {
                s.data.len()
                    + s.image
                        .get()
                        .and_then(Option::as_ref)
                        .map_or(0, |p| p.data_as_u8_slice().len())
            })
            .sum()
    }
}

pub(crate) fn resolve(db: &Database, ch: char, pixels: u16) -> Option<Resolved> {
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
                from_face(&face, id, pixels)
            })
            .flatten();
        if let Some(resolved) = glyph {
            if !resolved.strikes.is_empty() {
                return Some(resolved);
            }
            if mono.is_none() {
                mono = Some(resolved);
            }
        }
    }
    mono
}

fn from_face(face: &Face<'_>, id: GlyphId, pixels: u16) -> Option<Resolved> {
    let mut resolved = Resolved {
        strikes: Vec::new(),
        mono: None,
    };
    for (s, sbix) in strikes(face, id) {
        let origin = if sbix {
            face.glyph_bounding_box(id)
                .map(|b| {
                    let upem = f64::from(face.units_per_em());
                    (f64::from(b.x_min) / upem, f64::from(b.y_min) / upem)
                })
                .unwrap_or_default()
        } else {
            (0.0, 0.0)
        };
        resolved.strikes.push(Strike {
            data: s.data.to_vec(),
            image: OnceLock::new(),
            x: s.x,
            y: s.y,
            ppem: s.pixels_per_em,
            width: s.width,
            height: s.height,
            origin,
        });
    }
    if resolved.glyph(pixels).is_some() {
        return Some(resolved);
    }
    let mut outline = monochrome(face, id)?;
    outline.apply_affine(Affine::scale(1.0 / f64::from(face.units_per_em())));
    Some(Resolved {
        strikes: Vec::new(),
        mono: Some(Glyph::Mono(outline)),
    })
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
fn strikes<'a>(face: &'a Face<'a>, glyph: GlyphId) -> Vec<(RasterGlyphImage<'a>, bool)> {
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
        .filter(|(s, _)| {
            s.format == RasterImageFormat::PNG
                && s.pixels_per_em > 0
                && s.data.len() <= 4 * 1024 * 1024
        })
        .collect();
    candidates.sort_by_key(|(s, sbix)| (*sbix, s.pixels_per_em, s.data.as_ptr()));
    candidates.dedup_by_key(|(s, sbix)| (*sbix, s.pixels_per_em, s.data.as_ptr()));
    candidates
}

#[cfg(test)]
thread_local! {
    pub(crate) static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
