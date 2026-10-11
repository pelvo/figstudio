//! Page recordings for the render worker: a page is walked once per zoom level for the area
//! of a batch of tiles, and each tile of that area is replayed from the recording
//! (`figstudio_render::Recording`) instead of walking the document again.
//!
//! A replayed tile reports the recording's `Stats`: the counts of its whole area (the batch's
//! tiles plus one more ring), so with `Stats::max_with` the status bar describes that
//! neighbourhood of the view. A recording over its size limit or the work budget is not
//! used: its tiles are rendered directly as before (their own stats, budget truncation
//! included) and counted in `Stats::unrecorded_tiles`.

use figstudio_fig::Guid;
use figstudio_render::kurbo::{Affine, Rect};
use figstudio_render::{Frame, Recording, Renderer, Viewport};

use crate::tiles::{self, GUTTER, Key, TEXTURE, TILE};
use crate::worker::RenderJob;

/// Most bytes one recording may hold (commands and paths; image pixels are shared). All
/// recordings kept hold at most twice this, and at most [`ENTRIES`] are kept.
#[cfg(not(target_arch = "wasm32"))]
const LIMIT: usize = 64 << 20;
#[cfg(target_arch = "wasm32")]
const LIMIT: usize = 32 << 20;
/// Recordings kept (levels of the current page, and other tabs'), least recently used out.
const ENTRIES: usize = 4;

fn bytes(e: &Entry) -> usize {
    e.rec.as_ref().map_or(0, Recording::bytes)
}

struct Entry {
    doc: u64,
    page: Guid,
    level: i32,
    /// `Workers::images_arrived` when recorded.
    images: u64,
    area: Rect,
    /// `None`: over the size limit or the work budget; render the area's tiles directly.
    rec: Option<Recording>,
    used: u64,
}

impl Entry {
    /// Whether the tile `key` (texture pixels `window`) can come from this entry when
    /// `images` images have arrived. A recording with placeholders is redone once more
    /// images arrive; one without still draws every image it can.
    fn serves(&self, key: &Key, window: Rect, images: u64) -> bool {
        let a = self.area;
        let inside =
            window.x0 >= a.x0 && window.y0 >= a.y0 && window.x1 <= a.x1 && window.y1 <= a.y1;
        let fresh = self.images == images
            || self
                .rec
                .as_ref()
                .is_none_or(|r| r.stats().images_loading == 0);
        (self.doc, self.page, self.level) == (key.doc, key.page, key.level) && inside && fresh
    }
}

pub struct Recordings {
    entries: Vec<Entry>,
    clock: u64,
    limit: usize,
    /// Document walks made to record (for tests and profiling).
    pub walks: usize,
}

impl Default for Recordings {
    fn default() -> Self {
        Recordings {
            entries: Vec::new(),
            clock: 0,
            limit: LIMIT,
            walks: 0,
        }
    }
}

/// The tile's texture pixels in the level's recorded pixels (page origin at pixel 0, 0).
fn window(key: &Key) -> Rect {
    let (t, g) = (f64::from(TILE), f64::from(GUTTER));
    let (x, y) = (f64::from(key.x) * t - g, f64::from(key.y) * t - g);
    let side = f64::from(TEXTURE);
    Rect::new(x, y, x + side, y + side)
}

/// The area to record for `key`: every tile of its batch at its level, plus one more ring.
fn area(job: &RenderJob, key: &Key) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (key.x, key.y, key.x, key.y);
    let same = |k: &&Key| (k.doc, k.page, k.level) == (key.doc, key.page, key.level);
    for k in job.tiles.iter().filter(same) {
        (x0, y0, x1, y1) = (x0.min(k.x), y0.min(k.y), x1.max(k.x), y1.max(k.y));
    }
    let (t, g) = (f64::from(TILE), f64::from(GUTTER));
    let at = |n: i32, end: bool| f64::from(n) * t + if end { g } else { -g };
    Rect::new(
        at(x0.saturating_sub(1), false),
        at(y0.saturating_sub(1), false),
        at(x1.saturating_add(2), true),
        at(y1.saturating_add(2), true),
    )
}

/// Renders a tile by walking the document (the path without a usable recording).
fn direct(renderer: &mut Renderer, job: &RenderJob, viewport: &Viewport) -> Frame {
    #[cfg(target_arch = "wasm32")]
    {
        renderer.render_parallel(&job.doc, job.root, viewport)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        renderer.render(&job.doc, job.root, viewport)
    }
}

impl Recordings {
    /// Drops the recordings of a closed document.
    pub fn forget(&mut self, doc: u64) {
        self.entries.retain(|e| e.doc != doc);
    }

    /// Renders tile `key` of `job`: replayed from a recording of its level, recorded first
    /// if none covers it. The frame's `wanted` lists the images the recording walk asked for.
    pub fn draw(&mut self, renderer: &mut Renderer, job: &RenderJob, key: &Key) -> Frame {
        let viewport = Viewport {
            width: TEXTURE,
            height: TEXTURE,
            transform: key.transform(),
            background: job.background,
        };
        let win = window(key);
        let mut wanted = Vec::new();
        let found = self
            .entries
            .iter()
            .position(|e| e.serves(key, win, job.images));
        let at = match found {
            Some(i) => i,
            None => {
                let (i, w) = self.record(renderer, job, key);
                wanted = w;
                i
            }
        };
        self.clock += 1;
        let entry = &mut self.entries[at];
        entry.used = self.clock;
        let replayed = entry.rec.as_ref().and_then(|r| r.render(&viewport));
        let mut frame = match replayed {
            Some((frame, _)) => frame,
            None => {
                let mut frame = direct(renderer, job, &viewport);
                frame.stats.unrecorded_tiles = 1;
                frame
            }
        };
        // Asked for by the recording walk: sent on, or they would stay "in flight" for ever.
        frame.wanted.extend(wanted);
        frame
    }

    /// Records `key`'s level for its batch's area; returns the entry's index and the images
    /// the walk asked for.
    fn record(
        &mut self,
        renderer: &mut Renderer,
        job: &RenderJob,
        key: &Key,
    ) -> (usize, Vec<figstudio_render::images::ImageRequest>) {
        let area = area(job, key);
        let transform = Affine::scale(tiles::scale(key.level));
        self.walks += 1;
        let (rec, wanted) = renderer.record(&job.doc, job.root, transform, area, self.limit);
        // A truncated walk: tiles rendered directly show today's per-tile truncation.
        let rec = rec.filter(|r| r.stats().budget_exhausted == 0);
        // Replaces this level's old recording and drops other pages of the document.
        self.entries
            .retain(|e| e.doc != key.doc || (e.page == key.page && e.level != key.level));
        let new = rec.as_ref().map_or(0, Recording::bytes);
        while !self.entries.is_empty()
            && (self.entries.len() >= ENTRIES
                || self.entries.iter().map(bytes).sum::<usize>() + new > 2 * self.limit)
            && let Some(oldest) = (0..self.entries.len()).min_by_key(|&i| self.entries[i].used)
        {
            self.entries.swap_remove(oldest);
        }
        self.entries.push(Entry {
            doc: key.doc,
            page: key.page,
            level: key.level,
            images: job.images,
            area,
            rec,
            used: self.clock,
        });
        (self.entries.len() - 1, wanted)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "recordings_tests.rs"]
mod tests;
