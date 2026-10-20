//! The tile cache: least-recently-used tiles within a budget. Tiles of the planned level on
//! screen are dropped last; fallback tiles on screen give way to them, those already covered
//! by planned tiles first. Pure; see `tiles.rs` for the grid.

use figstudio_fig::Guid;
use figstudio_render::kurbo::Rect;

use crate::tiles::{Key, Range};

/// A fallback tile covering more planned tiles than this is never counted as redundant (the
/// check would be slow, and such a coarse tile is rarely fully covered).
const MAX_COVER_CHECK: usize = 64;

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
    /// What the last frame planned (see [`Cache::begin_frame`]); at most half the budget.
    plan: Option<(u64, Guid, Range)>,
}

impl<T> Cache<T> {
    pub fn new(budget: usize) -> Self {
        Cache {
            tiles: std::collections::HashMap::new(),
            budget: budget.max(1),
            clock: 0,
            frame: 0,
            plan: None,
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

    /// Starts drawing a frame whose planned tiles are `plan` (from `tiles::plan`, so at most
    /// half the budget): tiles touched from now on are on screen until the next call.
    pub fn begin_frame(&mut self, doc: u64, page: Guid, plan: Range) {
        self.frame += 1;
        self.plan = Some((doc, page, plan));
    }

    /// What the last frame planned: document, page and tiles (see [`Cache::begin_frame`]).
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the browser prunes its GPU scenes by it")
    )]
    pub fn plan(&self) -> Option<(u64, Guid, Range)> {
        self.plan
    }

    /// Marks a tile as drawn this frame: a fallback on screen is evicted after tiles that are
    /// not, but before planned tiles.
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
            let Some(victim) = self
                .tiles
                .iter()
                .min_by_key(|(k, e)| (self.keep_rank(k, e), e.used))
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.tiles.remove(&victim);
        }
    }

    /// Eviction order, lowest first: neither planned nor on screen; a fallback on screen
    /// already covered by cached planned tiles; another fallback on screen; a planned tile
    /// (of the planned range, drawn yet or not: the plan fits in the budget).
    fn keep_rank(&self, key: &Key, entry: &Entry<T>) -> u8 {
        let Some((doc, page, plan)) = self.plan else {
            return 0;
        };
        if key.doc == doc && key.page == page && plan.contains(key) {
            3
        } else if entry.frame != self.frame {
            0
        } else if self.covered(key, plan.level) {
            1
        } else {
            2
        }
    }

    /// Whether every tile of `level` under `key` is cached.
    fn covered(&self, key: &Key, level: i32) -> bool {
        // Counted before listing: a big level gap would otherwise list thousands of tiles
        // per candidate on every insert, only to refuse them.
        let under = Range::covering(level, key.page_rect());
        under.count() <= MAX_COVER_CHECK as u64
            && (under.y0..under.y1).all(|y| {
                (under.x0..under.x1).all(|x| {
                    let k = Key {
                        level,
                        x,
                        y,
                        ..*key
                    };
                    self.tiles.contains_key(&k)
                })
            })
    }

    /// How many tiles of `doc` are cached, and how many of them this frame drew.
    #[cfg(test)]
    pub fn count(&self, doc: u64) -> (usize, usize) {
        let of_doc = || self.tiles.iter().filter(move |(k, _)| k.doc == doc);
        let now = of_doc().filter(|(_, e)| e.frame == self.frame).count();
        (of_doc().count(), now)
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

#[cfg(test)]
#[path = "tile_cache_tests.rs"]
mod tests;
