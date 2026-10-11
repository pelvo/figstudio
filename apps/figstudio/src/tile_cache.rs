//! The tile cache: least-recently-used tiles within a budget, never dropping tiles on screen
//! while others can go. Pure; see `tiles.rs` for the grid.

use figstudio_fig::Guid;
use figstudio_render::kurbo::Rect;

use crate::tiles::Key;

/// A cached tile: its texture (or anything, in tests) and how fresh it is.
#[derive(Debug)]
pub struct Entry<T> {
    pub value: T,
    pub stats: figstudio_render::Stats,
    /// `Some(n)`: drawn with placeholders while images loaded, requested when `n` images had
    /// arrived; such a tile is drawn but rendered again once more images arrive.
    pub images: Option<u64>,
    used: u64,
    /// The last frame that drew it (see [`Cache::begin_frame`]).
    frame: u64,
}

impl<T> Entry<T> {
    pub fn fresh(&self, images_arrived: u64) -> bool {
        self.images.is_none_or(|n| n == images_arrived)
    }
}

/// Least-recently-used tiles, at most `budget` of them: a resident-memory budget for the
/// uploaded tiles only (results on their way from the worker are not counted).
#[derive(Debug)]
pub struct Cache<T> {
    tiles: std::collections::HashMap<Key, Entry<T>>,
    budget: usize,
    clock: u64,
    frame: u64,
}

impl<T> Cache<T> {
    pub fn new(budget: usize) -> Self {
        Cache {
            tiles: std::collections::HashMap::new(),
            budget: budget.max(1),
            clock: 0,
            frame: 0,
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

    /// Starts drawing a frame: tiles touched from now on are on screen until the next call.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Marks a tile as drawn this frame: evicted last, and not at all while on screen.
    pub fn touch(&mut self, key: &Key) {
        self.clock += 1;
        if let Some(e) = self.tiles.get_mut(key) {
            e.used = self.clock;
            e.frame = self.frame;
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
                frame: 0,
            },
        );
        while self.tiles.len() > self.budget {
            // Tiles on screen go last; only when they alone pass the budget is one dropped.
            let frame = self.frame;
            let victim = |on_screen: bool| {
                self.tiles
                    .iter()
                    .filter(|(_, e)| on_screen || e.frame != frame)
                    .min_by_key(|(_, e)| e.used)
                    .map(|(k, _)| *k)
            };
            let Some(oldest) = victim(false).or_else(|| victim(true)) else {
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
