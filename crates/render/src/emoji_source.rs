//! Platform adapters return baseline-relative ink; only owned pixels cross threads.
#[cfg(test)]
#[path = "emoji_cache_review_tests.rs"]
mod review_tests;
#[cfg(test)]
#[path = "emoji_source_tests.rs"]
mod tests;
use std::{collections::HashMap, sync::Arc};

use vello_cpu::{PixelMetadata, Pixmap, peniko::ImageAlphaType};

/// A complete emoji cluster shaped by the embedding platform, never just its first scalar.
/// Implementations may be called on render workers and must not retain thread-bound handles.
pub trait EmojiSource: Send + Sync {
    /// Canonical request size. Equal values must produce interchangeable rasters for a cluster.
    /// This must be idempotent; adapters with a fixed raster cap should expose it here.
    fn raster_size(&self, requested: u16) -> u16 {
        requested
    }

    /// Rasterize at `pixels` pixels per em. Return `None` when unavailable.
    /// Ink alone does not attest font/sequence support: unverified rasters are counted.
    fn raster(&self, cluster: &str, pixels: u16) -> Option<EmojiRaster>;
}

/// Premultiplied pixels and the ink's top-left offset from the baseline (positive y down).
#[derive(Clone, Debug)]
pub struct EmojiRaster {
    pub(crate) image: Arc<Pixmap>,
    pub(crate) left: f64,
    pub(crate) top: f64,
    ppem: Option<u16>,
    fully_shaped: bool,
}

impl EmojiRaster {
    fn glyph(self, requested: u16) -> (crate::emoji::Glyph, bool) {
        (
            crate::emoji::Glyph::Bitmap {
                x: self.left,
                y: -self.top - f64::from(self.image.height()),
                image: self.image,
                ppem: self.ppem.unwrap_or(requested),
                origin: (0.0, 0.0),
            },
            self.fully_shaped,
        )
    }

    /// Attest complete-cluster support: no .notdef, omitted required scalars, or
    /// decomposed fallback for requested sequence semantics. The source must verify this;
    /// nonempty pixels, cmap coverage, or merely completing a shaping call are insufficient.
    pub fn with_verified_shaping(mut self) -> Self {
        self.fully_shaped = true;
        self
    }

    /// Override the requested size when the adapter rasterized at a smaller ppem.
    pub fn with_ppem(mut self, ppem: u16) -> Option<Self> {
        if ppem == 0 {
            return None;
        }
        self.ppem = Some(ppem);
        Some(self)
    }

    /// Validate bounded straight-alpha RGBA, premultiply it, and reject empty ink.
    pub fn from_rgba(
        mut rgba: Vec<u8>,
        width: u16,
        height: u16,
        left: f64,
        top: f64,
    ) -> Option<Self> {
        if width == 0
            || height == 0
            || width > 1024
            || height > 1024
            || !left.is_finite()
            || !top.is_finite()
            || rgba.len() != usize::from(width) * usize::from(height) * 4
            || !rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0)
        {
            return None;
        }
        for p in rgba.as_chunks_mut::<4>().0 {
            for i in 0..3 {
                p[i] = ((u16::from(p[i]) * u16::from(p[3]) + 127) / 255) as u8;
            }
        }
        Some(Self {
            image: Arc::new(Pixmap::from_parts(
                rgba,
                width,
                height,
                PixelMetadata {
                    alpha_type: ImageAlphaType::AlphaPremultiplied,
                    ..Default::default()
                },
            )),
            left,
            top,
            ppem: None,
            fully_shaped: false,
        })
    }
}

enum Cached {
    Raster(EmojiRaster),
    /// `skipped` counts later lookups of this key only, so busy pages cannot shorten the wait.
    Missing {
        skipped: u32,
        delay: u32,
    },
}

impl Cached {
    fn bytes(&self) -> usize {
        match self {
            Self::Raster(r) => r.image.data_as_u8_slice().len(),
            Self::Missing { .. } => 0,
        }
    }
}

/// Renderer-local, bounded cache. JS work never runs while holding a shared lock.
pub(crate) struct Cache {
    pub(crate) source: Option<Arc<dyn EmojiSource>>,
    entries: HashMap<(String, u16), (Cached, u64)>,
    bytes: usize,
    clock: u64,
    byte_budget: usize,
    /// A first miss happened since the last [`Cache::take_retry_due`]; one more pass retries it.
    retry_due: bool,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            source: None,
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
            byte_budget: 16 * 1024 * 1024,
            retry_due: false,
        }
    }
}

impl Cache {
    pub(crate) fn new(source: Option<Arc<dyn EmojiSource>>) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }

    /// Whether a pass since the last call left a miss that is due an immediate retry.
    pub(crate) fn take_retry_due(&mut self) -> bool {
        std::mem::take(&mut self.retry_due)
    }

    fn store(&mut self, key: (String, u16), cached: Cached) {
        let bytes = cached.bytes();
        if let Some((old, _)) = self.entries.remove(&key) {
            self.bytes -= old.bytes();
        }
        while self.entries.len() >= 256 || self.bytes + bytes > self.byte_budget {
            let Some(oldest) = self
                .entries
                .iter()
                .max_by_key(|(_, (_, used))| self.clock.wrapping_sub(*used))
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some((old, _)) = self.entries.remove(&oldest) {
                self.bytes -= old.bytes();
            }
        }
        self.bytes += bytes;
        self.entries.insert(key, (cached, self.clock));
    }

    pub(crate) fn raster(
        &mut self,
        cluster: &str,
        pixels: u16,
    ) -> Option<(crate::emoji::Glyph, bool)> {
        let source = self.source.as_ref()?;
        if cluster.is_empty() || cluster.len() > 1024 || pixels == 0 {
            return None;
        }
        let pixels = source.raster_size(pixels);
        if pixels == 0 {
            return None;
        }
        let key = (cluster.to_owned(), pixels);
        self.clock = self.clock.wrapping_add(1);
        let mut retry = false;
        if let Some((cached, used)) = self.entries.get_mut(&key) {
            *used = self.clock;
            match cached {
                Cached::Raster(raster) => return Some(raster.clone().glyph(pixels)),
                Cached::Missing { skipped, delay } => {
                    *skipped += 1;
                    if *skipped < *delay {
                        return None;
                    }
                    retry = true;
                }
            }
        }
        let raster = source.raster(cluster, pixels);
        let bytes = raster
            .as_ref()
            .map_or(0, |r| r.image.data_as_u8_slice().len());
        // A valid image can exceed a configured cache budget: draw it without disturbing the cache.
        if bytes > self.byte_budget {
            return raster.map(|r| r.glyph(pixels));
        }
        // Give a newly loading font one immediate retry, then back off by this key's lookups.
        self.retry_due |= raster.is_none() && !retry;
        let cached = raster.as_ref().map_or(
            Cached::Missing {
                skipped: 0,
                delay: if retry { 64 } else { 1 },
            },
            |r| Cached::Raster(r.clone()),
        );
        self.store(key, cached);
        raster.map(|r| r.glyph(pixels))
    }
}
