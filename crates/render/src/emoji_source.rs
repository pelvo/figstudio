//! Platform adapters return baseline-relative ink; only owned pixels cross threads.
#[cfg(test)]
#[path = "emoji_source_tests.rs"]
mod tests;
use std::{collections::HashMap, sync::Arc};

use vello_cpu::{PixelMetadata, Pixmap, peniko::ImageAlphaType};

/// A complete emoji cluster shaped by the embedding platform, never just its first scalar.
/// Implementations may be called on render workers and must not retain thread-bound handles.
pub trait EmojiSource: Send + Sync {
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

/// Renderer-local, bounded cache. JS work never runs while holding a shared lock.
#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) source: Option<Arc<dyn EmojiSource>>,
    entries: HashMap<(String, u16), (EmojiRaster, u64)>,
    bytes: usize,
    clock: u64,
}

impl Cache {
    pub(crate) fn new(source: Option<Arc<dyn EmojiSource>>) -> Self {
        Self {
            source,
            ..Self::default()
        }
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
        let key = (cluster.to_owned(), pixels);
        self.clock = self.clock.wrapping_add(1);
        let raster = if let Some((cached, last_used)) = self.entries.get_mut(&key) {
            *last_used = self.clock;
            cached.clone()
        } else {
            // Empty ink can be transient (e.g. a platform font still loading).
            let raster = source.raster(cluster, pixels)?;
            let bytes = raster.image.data_as_u8_slice().len();
            while self.entries.len() >= 256 || self.bytes + bytes > 16 * 1024 * 1024 {
                let oldest = self
                    .entries
                    .iter()
                    .max_by_key(|(_, (_, used))| self.clock.wrapping_sub(*used))
                    .map(|(key, _)| key.clone())?;
                if let Some((old, _)) = self.entries.remove(&oldest) {
                    self.bytes -= old.image.data_as_u8_slice().len();
                }
            }
            self.bytes += bytes;
            self.entries.insert(key, (raster.clone(), self.clock));
            raster
        };
        Some((
            crate::emoji::Glyph::Bitmap {
                x: raster.left,
                y: -raster.top - f64::from(raster.image.height()),
                image: raster.image,
                ppem: raster.ppem.unwrap_or(pixels),
                origin: (0.0, 0.0),
            },
            raster.fully_shaped,
        ))
    }
}
