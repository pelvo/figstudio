//! The tile grid: fixed-size tiles in device pixels on a page-space grid per zoom level.
//!
//! Level `n` draws the page at `2^(n/2)` device pixels per page unit. The canvas shows the
//! lowest level at least as sharp as the screen (`zoom * pixels_per_point`) and scales its
//! tiles down by at most `1/√2`. Tile `(x, y)` of level `n` covers page units
//! `[x, x + 1) * TILE / scale` (and the same for `y`). Pure; drawn by `tiles_view.rs`.

use figstudio_fig::Guid;
use figstudio_render::kurbo::{Affine, Rect};

pub use crate::tile_cache::Cache;

/// Tile side in device pixels (the part drawn on screen).
pub const TILE: u16 = 512;
/// Pixels of the neighbouring tiles rendered around each tile, so linear filtering at a tile
/// edge blends with the neighbour's pixels instead of repeating the edge (no seams).
pub const GUTTER: u16 = 2;
/// Side of a tile's texture: the tile and its gutter on both sides.
pub const TEXTURE: u16 = TILE + 2 * GUTTER;
/// Bytes of one tile's texture (RGBA), gutter included.
pub const TILE_BYTES: usize = TEXTURE as usize * TEXTURE as usize * 4;

/// The texture coordinates of the tile inside its texture (the gutter is not drawn).
pub fn uv() -> [f32; 2] {
    let t = f32::from(TEXTURE);
    [f32::from(GUTTER) / t, f32::from(GUTTER + TILE) / t]
}
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

    /// Page-to-texture-pixel transform for the render viewport ([`TEXTURE`] square): the
    /// tile's page rect lands on pixels `GUTTER..GUTTER + TILE`.
    pub fn transform(&self) -> Affine {
        let (t, g) = (f64::from(TILE), f64::from(GUTTER));
        Affine::translate((g - f64::from(self.x) * t, g - f64::from(self.y) * t))
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

    /// How many tiles the range holds, without listing them.
    pub fn count(&self) -> u64 {
        let span = |a: i32, b: i32| u64::try_from(i64::from(b) - i64::from(a)).unwrap_or(0);
        span(self.x0, self.x1) * span(self.y0, self.y1)
    }

    /// Whether `key` is at one of this range's positions (any document or page).
    pub fn contains(&self, key: &Key) -> bool {
        key.level == self.level
            && (self.x0..self.x1).contains(&key.x)
            && (self.y0..self.y1).contains(&key.y)
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

/// The level and tiles to show for `visible`: `level`, or a coarser one when its tiles would
/// fill more than half the cache (a huge canvas on a small budget), plus a one-tile prefetch
/// ring when that still fits. Every tile of the plan then fits in the cache at once.
pub fn plan(level: i32, visible: Rect, budget: usize) -> Range {
    let cap = (budget / 2).max(1) as f64;
    // Worst case over every pan: a misaligned view touches one more tile per axis.
    let fits = |level: i32, ring: f64| {
        let k = scale(level) / f64::from(TILE);
        let n = |extent: f64| (extent * k).ceil() + 1.0 + 2.0 * ring;
        n(visible.width()) * n(visible.height()) <= cap
    };
    let mut level = level;
    while level > *LEVELS.start() && !fits(level, 0.0) {
        level -= 1;
    }
    let range = Range::covering(level, visible);
    if fits(level, 1.0) {
        range.grow(1)
    } else {
        range
    }
}

/// The tiles of `range` (from [`plan`], so at most half the cache) to render, nearest its
/// centre first: those not cached, and those cached with placeholders before more images
/// arrived.
pub fn wanted<T>(cache: &Cache<T>, range: Range, doc: u64, page: Guid, arrived: u64) -> Vec<Key> {
    range
        .by_distance(doc, page)
        .into_iter()
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

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "tiles_seam_tests.rs"]
mod seam_tests;
