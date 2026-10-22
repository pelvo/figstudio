//! Native strike metadata and bounded decoded images; font bytes remain owned by fontdb.
use crate::emoji::Glyph;
use fontdb::{Database, ID};
use std::{mem::size_of, sync::Arc};
use ttf_parser::{Face, GlyphId};
use vello_cpu::{Pixmap, kurbo::BezPath};

pub(crate) const CACHED_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct Resolved {
    face: ID,
    glyph: GlyphId,
    strikes: Vec<Strike>,
    mono: Option<BezPath>,
    clock: u64,
}

struct Strike {
    // None is untried/evicted; Some(None) is a definitive bad font strike.
    image: Option<Option<Arc<Pixmap>>>,
    record: usize,
    used: u64,
    x: i16,
    y: i16,
    ppem: u16,
    width: u16,
    height: u16,
    origin: (f64, f64),
}

impl Resolved {
    pub(super) fn new(face: &Face<'_>, font: ID, glyph: GlyphId) -> Self {
        // Reserve room for the largest allowed PNG before retaining any metadata.
        let max = (CACHED_BYTES - 4 * 1024 * 1024 - size_of::<Self>()) / size_of::<Strike>();
        let strikes = super::strikes(face, glyph)
            .into_iter()
            .take(max)
            .enumerate()
            .map(|(record, (s, sbix))| {
                let origin = if sbix {
                    face.glyph_bounding_box(glyph)
                        .map(|b| {
                            let upem = f64::from(face.units_per_em());
                            (f64::from(b.x_min) / upem, f64::from(b.y_min) / upem)
                        })
                        .unwrap_or_default()
                } else {
                    (0.0, 0.0)
                };
                Strike {
                    image: None,
                    record,
                    used: 0,
                    x: s.x,
                    y: s.y,
                    ppem: s.pixels_per_em,
                    width: s.width,
                    height: s.height,
                    origin,
                }
            })
            .collect();
        Self {
            face: font,
            glyph,
            strikes,
            mono: None,
            clock: 0,
        }
    }

    pub(super) fn with_mono(mut self, path: BezPath) -> Self {
        self.strikes = Vec::new();
        self.mono = Some(path);
        self
    }

    pub(super) fn is_bitmap(&self) -> bool {
        !self.strikes.is_empty()
    }

    pub(crate) fn bytes(&self) -> usize {
        size_of::<Self>()
            + self.strikes.capacity() * size_of::<Strike>()
            + self
                .strikes
                .iter()
                .filter_map(|s| s.image.as_ref()?.as_ref())
                .map(|p| p.data_as_u8_slice().len())
                .sum::<usize>()
            + self
                .mono
                .as_ref()
                .map_or(0, |p| std::mem::size_of_val(p.elements()))
    }

    fn load(&self, db: &Database, index: usize) -> Option<Arc<Pixmap>> {
        let s = &self.strikes[index];
        db.with_face_data(self.face, |bytes, index| {
            let face = Face::parse(bytes, index).ok()?;
            let records = super::strikes(&face, self.glyph);
            let (raw, _) = records.get(s.record)?;
            let image = super::decode_png(raw.data)?;
            (image.width() == s.width && image.height() == s.height).then_some(image)
        })
        .flatten()
    }

    fn make_room(&mut self, bytes: usize) {
        while self.bytes() + bytes > CACHED_BYTES {
            let oldest = self
                .strikes
                .iter()
                .enumerate()
                .filter(|(_, s)| matches!(s.image, Some(Some(_))))
                .max_by_key(|(_, s)| self.clock.wrapping_sub(s.used))
                .map(|(i, _)| i);
            let Some(oldest) = oldest else { break };
            self.strikes[oldest].image = None;
        }
    }

    pub(crate) fn glyph(&mut self, db: &Database, pixels: u16) -> Option<Glyph> {
        let mut order: Vec<_> = (0..self.strikes.len()).collect();
        order.sort_by_key(|i| {
            (
                pixels.abs_diff(self.strikes[*i].ppem),
                std::cmp::Reverse(self.strikes[*i].ppem),
            )
        });
        for index in order {
            self.clock = self.clock.wrapping_add(1);
            self.strikes[index].used = self.clock;
            if self.strikes[index].image.is_none() {
                let image = self.load(db, index);
                if let Some(image) = &image {
                    self.make_room(image.data_as_u8_slice().len());
                }
                self.strikes[index].image = Some(image);
            }
            let s = &self.strikes[index];
            if let Some(Some(image)) = &s.image {
                return Some(Glyph::Bitmap {
                    image: image.clone(),
                    x: f64::from(s.x),
                    y: f64::from(s.y),
                    ppem: s.ppem,
                    origin: s.origin,
                });
            }
        }
        self.mono.as_ref().map(|p| Glyph::Mono(p.clone()))
    }
}
