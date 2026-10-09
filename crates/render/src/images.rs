//! Decoded images, shared between renders, threads and documents (keys are content hashes).
//!
//! Images are decoded at the resolution the view needs, not at full size: level `n` is the
//! image downscaled by `2^n` on each side. A page at fit zoom shows most images a few dozen
//! pixels wide, so this keeps memory and decode time proportional to what is on screen.
//! When a paint's Figma thumbnail (`imageThumbnail`, at most 512 px) is large enough, it is
//! used instead of the full image.
//!
//! Two ways images get here:
//! - **Bytes in the document** (`Document::images`): decoded during the render.
//! - **Lazily opened documents** (`Document::lazy_images`): the render never reads files. It
//!   uses whatever is cached (a placeholder, the thumbnail or a coarser level meanwhile) and
//!   lists what it wants in [`Frame::wanted`](crate::Frame::wanted). The caller reads those
//!   entries and calls [`Images::insert_encoded`] (or [`Images::fail`]), then renders again.
//!   Every request handed out must be answered one of these two ways: until then it counts as
//!   in flight and is not requested again.
//!
//! The cache holds at most `budget` bytes of pixels, dropping the least recently drawn images;
//! images drawn in the current frame are never dropped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use figstudio_fig::archive::Entry;
use vello_cpu::peniko::ImageAlphaType;
use vello_cpu::{PixelMetadata, Pixmap};

/// Largest image side handed to the rasterizer.
pub const MAX_IMAGE_SIDE: u32 = 8192;
/// Finest level wanted when an image is drawn smaller than this many times its size.
const MAX_LEVEL: u8 = 12;
/// Default cache size.
pub const DEFAULT_BUDGET: usize = 1 << 30;

/// An image the render needs from a lazily opened document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRequest {
    pub hash: String,
    /// Decode at this level (see the module docs); pass it to [`Images::insert_encoded`].
    pub level: u8,
    /// Where the image is in the document's archive.
    pub entry: Entry,
}

/// A decoded image and the size of the original.
#[derive(Clone)]
pub struct Decoded {
    pub pixmap: Arc<Pixmap>,
    /// Size of the full image in pixels.
    pub natural: (u32, u32),
    pub level: u8,
    /// No finer level exists (full size, or capped at [`MAX_IMAGE_SIDE`]).
    pub finest: bool,
}

impl Decoded {
    fn bytes(&self) -> usize {
        self.pixmap.data_as_u8_slice().len()
    }
}

struct Slot {
    image: Decoded,
    used: u64,
}

struct Inner {
    /// By hash, then by requested level.
    slots: HashMap<String, BTreeMap<u8, Slot>>,
    failed: HashSet<String>,
    in_flight: HashSet<(String, u8)>,
    bytes: usize,
    budget: usize,
    frame: u64,
}

/// Shared handle to the cache; clones refer to the same images.
#[derive(Clone)]
pub struct Images(Arc<Mutex<Inner>>);

impl Default for Images {
    fn default() -> Self {
        Images::new(DEFAULT_BUDGET)
    }
}

impl Images {
    /// A cache holding at most `budget` bytes of pixels (more only while one frame needs it).
    pub fn new(budget: usize) -> Self {
        Images(Arc::new(Mutex::new(Inner {
            slots: HashMap::new(),
            failed: HashSet::new(),
            in_flight: HashSet::new(),
            bytes: 0,
            budget,
            frame: 0,
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts a frame: images drawn from now on are kept over older ones.
    pub(crate) fn begin_frame(&self) {
        self.lock().frame += 1;
    }

    /// The copy of `hash` decoded for `level`, marked as drawn in this frame. Draws use
    /// exactly this copy, so what a frame shows does not depend on what was decoded before.
    pub(crate) fn get(&self, hash: &str, level: u8) -> Option<Decoded> {
        let mut inner = self.lock();
        let frame = inner.frame;
        let slot = inner.slots.get_mut(hash)?.get_mut(&level)?;
        slot.used = frame;
        Some(slot.image.clone())
    }

    /// The cached copy of `hash` closest to `level` (finer first), to show while the right
    /// one is read.
    pub(crate) fn nearest(&self, hash: &str, level: u8) -> Option<Decoded> {
        let mut inner = self.lock();
        let frame = inner.frame;
        let levels = inner.slots.get_mut(hash)?;
        let key = levels
            .keys()
            .copied()
            .min_by_key(|l| (l.abs_diff(level), *l > level))?;
        let slot = levels.get_mut(&key)?;
        slot.used = frame;
        Some(slot.image.clone())
    }

    pub(crate) fn has_failed(&self, hash: &str) -> bool {
        self.lock().failed.contains(hash)
    }

    /// Marks `hash` at `level` as requested; false if it already was (or cannot be read).
    pub(crate) fn begin_request(&self, hash: &str, level: u8) -> bool {
        let mut inner = self.lock();
        !inner.failed.contains(hash) && inner.in_flight.insert((hash.to_string(), level))
    }

    /// Decodes `bytes` for `level` and stores the result. Returns false when the bytes are
    /// not a readable image; it is then treated as missing. Decoding happens outside the lock,
    /// so call it from any thread.
    pub fn insert_encoded(&self, hash: &str, bytes: &[u8], level: u8) -> bool {
        let decoded = decode(bytes, level);
        let mut inner = self.lock();
        inner.in_flight.remove(&(hash.to_string(), level));
        match decoded {
            Some(image) => {
                inner.insert(hash, level, image);
                true
            }
            None => {
                inner.failed.insert(hash.to_string());
                false
            }
        }
    }

    /// The bytes for a request could not be read: treat the image as missing.
    pub fn fail(&self, hash: &str, level: u8) {
        let mut inner = self.lock();
        inner.in_flight.remove(&(hash.to_string(), level));
        inner.failed.insert(hash.to_string());
    }

    /// Bytes of pixels held.
    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }

    /// Requests handed out and not answered yet.
    pub fn in_flight(&self) -> usize {
        self.lock().in_flight.len()
    }
}

impl Inner {
    fn insert(&mut self, hash: &str, level: u8, image: Decoded) {
        let frame = self.frame;
        let bytes = image.bytes();
        let old = self
            .slots
            .entry(hash.to_string())
            .or_default()
            .insert(level, Slot { image, used: frame });
        if let Some(old) = old {
            self.bytes -= old.image.bytes();
        }
        self.bytes += bytes;
        // Least recently drawn first; never what this frame drew.
        while self.bytes > self.budget {
            let Some((victim, victim_level)) = self
                .slots
                .iter()
                .flat_map(|(h, levels)| levels.iter().map(move |(l, s)| (h, *l, s.used)))
                .filter(|(_, _, used)| *used < frame)
                .min_by_key(|(_, _, used)| *used)
                .map(|(h, l, _)| (h.clone(), l))
            else {
                break;
            };
            if let Some(levels) = self.slots.get_mut(&victim) {
                if let Some(s) = levels.remove(&victim_level) {
                    self.bytes -= s.image.bytes();
                }
                if levels.is_empty() {
                    self.slots.remove(&victim);
                }
            }
        }
    }
}

/// The level at which an image drawn at `scale` output pixels per image pixel stays sharp.
pub(crate) fn level_for(scale: f64) -> u8 {
    if !(scale.is_finite() && scale > 0.0) || scale >= 1.0 {
        return 0;
    }
    let l = (1.0 / scale).log2().floor();
    if l >= f64::from(MAX_LEVEL) {
        MAX_LEVEL
    } else {
        l.max(0.0) as u8
    }
}

/// `n / 2^level`, rounded up, at least 1.
fn shrink(n: u32, level: u8) -> u32 {
    n.div_ceil(1u32 << level).max(1)
}

/// The level actually used for an image of `natural` size asked for at `level`: no smaller
/// than 1 px, no larger than [`MAX_IMAGE_SIDE`].
pub(crate) fn clamp_level(natural: (u32, u32), level: u8) -> u8 {
    let long = natural.0.max(natural.1).max(1);
    // The coarsest level still reaching 1 px is as far as shrinking goes.
    let coarsest = (u32::BITS - long.leading_zeros()).saturating_sub(1) as u8;
    let mut level = level.min(coarsest);
    while shrink(long, level) > MAX_IMAGE_SIDE {
        level += 1;
    }
    level
}

/// Decodes an image for `level` (see [`clamp_level`]).
pub(crate) fn decode(bytes: &[u8], level: u8) -> Option<Decoded> {
    let img = image::load_from_memory(bytes).ok()?;
    let natural = (img.width(), img.height());
    if natural.0 == 0 || natural.1 == 0 {
        return None;
    }
    let long = natural.0.max(natural.1);
    let level = clamp_level(natural, level);
    let finest = level == 0 || shrink(long, level - 1) > MAX_IMAGE_SIDE;
    let (w, h) = (shrink(natural.0, level), shrink(natural.1, level));
    let img = if level > 0 {
        img.thumbnail_exact(w, h)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (w, h) = (
        u16::try_from(rgba.width()).ok()?,
        u16::try_from(rgba.height()).ok()?,
    );
    let meta = PixelMetadata {
        may_have_transparency: true,
        alpha_type: ImageAlphaType::Alpha,
    };
    Some(Decoded {
        pixmap: Arc::new(Pixmap::from_parts(rgba.into_raw(), w, h, meta)),
        natural,
        level,
        finest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn levels_halve_each_side() {
        assert_eq!(level_for(1.0), 0);
        assert_eq!(level_for(3.0), 0);
        assert_eq!(level_for(0.5), 1);
        assert_eq!(level_for(0.3), 1);
        assert_eq!(level_for(0.01), 6);
        assert_eq!(level_for(0.0), 0);
        let d = decode(&png(301, 100), 2).unwrap();
        assert_eq!((d.pixmap.width(), d.pixmap.height()), (76, 25));
        assert_eq!((d.natural, d.level, d.finest), ((301, 100), 2, false));
        let d = decode(&png(10, 10), 9).unwrap();
        assert_eq!(
            (d.pixmap.width(), d.level),
            (2, 3),
            "stops at the last level above 1 px"
        );
        assert!(decode(&png(4, 4), 0).unwrap().finest);
        assert!(decode(b"not an image", 0).is_none());
    }

    #[test]
    fn budget_drops_least_recently_drawn_but_not_this_frame() {
        let images = Images::new(3 * 64 * 64 * 4);
        let bytes = png(64, 64);
        for (i, h) in ["a", "b", "c"].iter().enumerate() {
            images.begin_frame();
            assert!(images.begin_request(h, 0));
            assert!(!images.begin_request(h, 0), "requested twice");
            images.insert_encoded(h, &bytes, 0);
            assert_eq!(images.bytes(), (i + 1) * 64 * 64 * 4);
        }
        images.begin_frame();
        images.get("a", 0);
        images.insert_encoded("d", &bytes, 0);
        assert!(
            images.get("b", 0).is_none(),
            "least recently drawn is dropped"
        );
        assert!(images.get("a", 0).is_some() && images.get("c", 0).is_some());
        // Everything drawn this frame stays, even over the budget.
        images.begin_frame();
        for h in ["a", "c", "d"] {
            images.get(h, 0);
        }
        images.insert_encoded("e", &bytes, 0);
        assert_eq!(images.bytes(), 4 * 64 * 64 * 4);
        assert_eq!(images.in_flight(), 0);
    }

    #[test]
    fn levels_are_kept_apart_and_failures_remembered() {
        let images = Images::default();
        let bytes = png(64, 64);
        images.insert_encoded("a", &bytes, 0);
        images.insert_encoded("a", &bytes, 3);
        assert_eq!(images.get("a", 0).unwrap().pixmap.width(), 64);
        assert_eq!(images.get("a", 3).unwrap().pixmap.width(), 8);
        assert!(images.get("a", 1).is_none(), "only the level asked for");
        assert_eq!(
            images.nearest("a", 1).unwrap().level,
            0,
            "nearest, finer first"
        );
        assert_eq!(images.nearest("a", 2).unwrap().level, 3);
        assert!(!images.insert_encoded("b", b"junk", 0));
        assert!(images.has_failed("b") && !images.begin_request("b", 0));
        assert!(images.begin_request("c", 2));
        images.fail("c", 2);
        assert!(images.has_failed("c") && images.in_flight() == 0);
        assert_eq!(clamp_level((10, 10), 9), 3);
        assert_eq!(clamp_level((20_000, 100), 0), 2, "within MAX_IMAGE_SIDE");
    }
}
