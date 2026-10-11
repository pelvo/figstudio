//! The tile grid: fixed-size tiles in device pixels on a page-space grid per zoom level.
//!
//! Level `n` draws the page at `2^(n/2)` device pixels per page unit. The canvas shows the
//! lowest level at least as sharp as the screen (`zoom * pixels_per_point`) and scales its
//! tiles down by at most `1/√2`. Tile `(x, y)` of level `n` covers page units
//! `[x, x + 1) * TILE / scale` (and the same for `y`). Pure; drawn by `tiles_view.rs`.

use figstudio_fig::Guid;
use figstudio_render::kurbo::{Affine, Rect};

/// Tile side in device pixels.
pub const TILE: u16 = 512;
/// Bytes of one tile's RGBA pixels (as an uploaded texture).
pub const TILE_BYTES: usize = TILE as usize * TILE as usize * 4;
/// Levels outside this range are clamped (zoom limits times any sane pixels per point).
const LEVELS: std::ops::RangeInclusive<i32> = -40..=40;
/// Most tiles across or down a view: a 16384-px canvas needs 32 (plus the ring).
const MAX_SPAN: u16 = 64;

/// One tile of one page of one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// `Document::instance`: every opened document gets its own.
    pub doc: u64,
    pub page: Guid,
    pub level: i32,
    pub x: i32,
    pub y: i32,
}

/// Device pixels per page unit at `level`.
pub fn scale(level: i32) -> f64 {
    (f64::from(level) / 2.0).exp2()
}

/// The lowest level drawing at least `zoom_px` device pixels per page unit.
pub fn level_for(zoom_px: f32) -> i32 {
    let z = f64::from(zoom_px.max(f32::MIN_POSITIVE));
    // The small slack keeps an exact level (zoom 100 % at 1 ppp) from rounding up.
    let n = (2.0 * z.log2() - 1e-6).ceil();
    // Clamped first, so the cast is exact.
    n.clamp(f64::from(*LEVELS.start()), f64::from(*LEVELS.end())) as i32
}

impl Key {
    /// The page area this tile covers.
    pub fn page_rect(&self) -> Rect {
        let side = f64::from(TILE) / scale(self.level);
        let (x, y) = (f64::from(self.x) * side, f64::from(self.y) * side);
        Rect::new(x, y, x + side, y + side)
    }

    /// Page-to-tile-pixel transform for the render viewport.
    pub fn transform(&self) -> Affine {
        let t = f64::from(TILE);
        Affine::translate((-f64::from(self.x) * t, -f64::from(self.y) * t))
            * Affine::scale(scale(self.level))
    }
}

/// Tiles `x0..x1` by `y0..y1` (end exclusive) of one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub level: i32,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Range {
    /// The tiles of `level` touching the page area `visible`.
    pub fn covering(level: i32, visible: Rect) -> Range {
        let k = scale(level) / f64::from(TILE);
        // `as` saturates, so absurd pans give an empty or clamped range, never a panic; the
        // span is capped so such a range never lists billions of tiles.
        let span = |a: f64, b: f64| {
            let (a, b) = ((a * k).floor(), (b * k).ceil());
            let b = b.min(a + f64::from(MAX_SPAN));
            (a as i32, b as i32)
        };
        let ((x0, x1), (y0, y1)) = (span(visible.x0, visible.x1), span(visible.y0, visible.y1));
        Range {
            level,
            x0,
            y0,
            x1,
            y1,
        }
    }

    /// One more tile on every side (the prefetch ring).
    pub fn grow(self, n: i32) -> Range {
        Range {
            x0: self.x0.saturating_sub(n),
            y0: self.y0.saturating_sub(n),
            x1: self.x1.saturating_add(n),
            y1: self.y1.saturating_add(n),
            ..self
        }
    }

    /// Every tile, nearest the range's centre first.
    pub fn by_distance(&self, doc: u64, page: Guid) -> Vec<Key> {
        let (cx, cy) = (
            (f64::from(self.x0) + f64::from(self.x1)) / 2.0,
            (f64::from(self.y0) + f64::from(self.y1)) / 2.0,
        );
        let mut keys: Vec<Key> = (self.y0..self.y1)
            .flat_map(|y| (self.x0..self.x1).map(move |x| (x, y)))
            .map(|(x, y)| Key {
                doc,
                page,
                level: self.level,
                x,
                y,
            })
            .collect();
        let dist = |k: &Key| {
            let (dx, dy) = (f64::from(k.x) + 0.5 - cx, f64::from(k.y) + 0.5 - cy);
            dx * dx + dy * dy
        };
        keys.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
        keys
    }
}

/// The page area shown on a canvas of `size` points, for `zoom` and `pan` (see `tab::View`).
pub fn visible_page(zoom: f32, pan: [f32; 2], size: [f32; 2]) -> Rect {
    let z = f64::from(zoom.max(f32::MIN_POSITIVE));
    let (px, py) = (f64::from(pan[0]), f64::from(pan[1]));
    Rect::new(
        -px / z,
        -py / z,
        (f64::from(size[0]) - px) / z,
        (f64::from(size[1]) - py) / z,
    )
}

/// A cached tile: its texture (or anything, in tests) and how fresh it is.
#[derive(Debug)]
pub struct Entry<T> {
    pub value: T,
    pub stats: figstudio_render::Stats,
    /// `Some(n)`: drawn with placeholders while images loaded, requested when `n` images had
    /// arrived; such a tile is drawn but rendered again once more images arrive.
    pub images: Option<u64>,
    used: u64,
}

impl<T> Entry<T> {
    pub fn fresh(&self, images_arrived: u64) -> bool {
        self.images.is_none_or(|n| n == images_arrived)
    }
}

/// Least-recently-used tiles, at most `budget` of them.
#[derive(Debug)]
pub struct Cache<T> {
    tiles: std::collections::HashMap<Key, Entry<T>>,
    budget: usize,
    clock: u64,
}

impl<T> Cache<T> {
    pub fn new(budget: usize) -> Self {
        Cache {
            tiles: std::collections::HashMap::new(),
            budget: budget.max(1),
            clock: 0,
        }
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn get(&self, key: &Key) -> Option<&Entry<T>> {
        self.tiles.get(key)
    }

    /// Marks a tile as just used (drawn), so it is evicted last.
    pub fn touch(&mut self, key: &Key) {
        self.clock += 1;
        if let Some(e) = self.tiles.get_mut(key) {
            e.used = self.clock;
        }
    }

    /// Stores a tile, evicting the least recently used ones beyond the budget.
    pub fn insert(
        &mut self,
        key: Key,
        value: T,
        stats: figstudio_render::Stats,
        images: Option<u64>,
    ) {
        self.clock += 1;
        let used = self.clock;
        self.tiles.insert(
            key,
            Entry {
                value,
                stats,
                images,
                used,
            },
        );
        while self.tiles.len() > self.budget {
            let Some(oldest) = self
                .tiles
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.tiles.remove(&oldest);
        }
    }

    /// Drops every tile `keep` refuses (a closed document).
    pub fn retain(&mut self, mut keep: impl FnMut(&Key) -> bool) {
        self.tiles.retain(|k, _| keep(k));
    }

    /// Cached tiles of `doc`/`page` touching `visible`, in drawing order: coarser levels
    /// first, `level` (the sharp one) last, so each missing tile shows the best tile cached
    /// under it (a coarser one stretched, or a finer one shrunk) and never empty canvas.
    pub fn draw_order(&self, doc: u64, page: Guid, level: i32, visible: Rect) -> Vec<Key> {
        let mut keys: Vec<Key> = self
            .tiles
            .keys()
            .filter(|k| k.doc == doc && k.page == page)
            .filter(|k| k.page_rect().intersect(visible).area() > 0.0)
            .copied()
            .collect();
        keys.sort_by_key(|k| (k.level == level, k.level, k.y, k.x));
        keys
    }
}

/// The tiles of `range` to render, nearest its centre first: those not cached, and those
/// cached with placeholders before more images arrived. Only the nearest half of the cache
/// budget is considered, so new tiles never evict the ones on screen (a canvas larger than
/// that shows fallback tiles at its edges).
pub fn wanted<T>(cache: &Cache<T>, range: Range, doc: u64, page: Guid, arrived: u64) -> Vec<Key> {
    range
        .by_distance(doc, page)
        .into_iter()
        .take((cache.budget() / 2).max(1))
        .filter(|k| cache.get(k).is_none_or(|e| !e.fresh(arrived)))
        .collect()
}

/// The batch to send now, or `None` while `pending` (the last batch, minus finished tiles)
/// already holds exactly the `wanted` tiles. A new batch replaces the old one in the worker,
/// so tiles that left the view are skipped instead of rendered.
pub fn next_batch(wanted: Vec<Key>, pending: &[Key]) -> Option<Vec<Key>> {
    let same = wanted.len() == pending.len() && wanted.iter().all(|k| pending.contains(k));
    (!same).then_some(wanted)
}

#[cfg(test)]
#[path = "tiles_tests.rs"]
mod tests;
