//! Draws the canvas from cached tiles (`tiles.rs`) and asks the render worker for the
//! missing ones, nearest the view centre first.

use egui::{Color32, Pos2, Rect};
use figstudio_render::Stats;
use web_time::Duration;

use crate::tab::Tab;
use crate::tiles::{self, Cache, TILE_BYTES};
use crate::worker::{RenderDone, RenderJob, Workers};

/// Budget for the tiles kept as textures (resident cache, gutters included): 256 MiB native,
/// 128 MiB in the browser. Tiles still being rendered or uploaded are not counted; at most
/// one batch per tab (half the cache) is outstanding.
#[cfg(not(target_arch = "wasm32"))]
const BUDGET_BYTES: usize = 256 << 20;
#[cfg(target_arch = "wasm32")]
const BUDGET_BYTES: usize = 128 << 20;

/// A rendered tile; its pixels live only in the texture.
pub struct Tile {
    texture: egui::TextureHandle,
    elapsed: Duration,
}

impl std::fmt::Debug for Tile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tile")
            .field("elapsed", &self.elapsed)
            .finish_non_exhaustive()
    }
}

/// The tiles of every open document, in one memory budget.
#[derive(Debug)]
pub struct Tiles {
    cache: Cache<Tile>,
}

impl Default for Tiles {
    fn default() -> Self {
        Tiles {
            cache: Cache::new(BUDGET_BYTES / TILE_BYTES),
        }
    }
}

impl Tiles {
    /// Drops the tiles of a closed document.
    pub fn forget(&mut self, doc: u64) {
        self.cache.retain(|k| k.doc != doc);
    }

    /// Uploads a finished tile and stores it.
    pub fn accept(&mut self, ctx: &egui::Context, tab: &mut Tab, done: RenderDone) {
        tab.tile_requests.retain(|k| *k != done.key);
        tab.tiles_received.push((done.key, done.images));
        let image = egui::ColorImage::from_rgba_premultiplied(done.size, &done.rgba_premultiplied);
        let texture = ctx.load_texture("tile", image, egui::TextureOptions::LINEAR);
        let images = (done.stats.images_loading > 0).then_some(done.images);
        let tile = Tile {
            texture,
            elapsed: done.elapsed,
        };
        self.cache.insert(done.key, tile, done.stats, images);
    }

    /// Paints the cached tiles of the tab's view onto `rect` (best fallback under missing
    /// ones) and requests the missing tiles plus a one-tile ring.
    pub fn show(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        tab: &mut Tab,
        workers: &mut Workers,
        ppp: f32,
    ) {
        let (doc, page, view) = (tab.doc.instance, tab.page, tab.view);
        let visible = tiles::visible_page(view.zoom, view.pan.into(), rect.size().into());
        let range = tiles::plan(
            tiles::level_for(view.zoom * ppp),
            visible,
            self.cache.budget(),
        );
        let level = range.level;
        self.cache.begin_frame();
        let uv = tiles::uv();
        let mut stats: Option<Stats> = None;
        let mut elapsed = Duration::ZERO;
        for key in self.cache.draw_order(doc, page, level, visible) {
            self.cache.touch(&key);
            let Some(entry) = self.cache.get(&key) else {
                continue;
            };
            let r = key.page_rect();
            let corner = |x: f64, y: f64| view.to_screen(rect.min, Pos2::new(x as f32, y as f32));
            painter.image(
                entry.value.texture.id(),
                Rect::from_min_max(corner(r.x0, r.y0), corner(r.x1, r.y1)),
                Rect::from_min_max(Pos2::new(uv[0], uv[0]), Pos2::new(uv[1], uv[1])),
                Color32::WHITE,
            );
            if key.level == level {
                stats.get_or_insert_default().max_with(&entry.stats);
                elapsed += entry.value.elapsed;
            }
        }
        // The status bar describes the sharp tiles on screen (each count is a lower bound: the
        // largest of any one tile); it keeps the last numbers while none has arrived.
        if let Some(stats) = stats {
            tab.stats = stats;
            tab.render_time = elapsed;
        }

        if rect.width() * ppp < 1.0 || rect.height() * ppp < 1.0 {
            return;
        }
        let images = workers.images_arrived();
        let wanted = tiles::wanted(&self.cache, range, doc, page, images);
        let Some(batch) = tiles::next_batch(wanted, &tab.tile_requests) else {
            return;
        };
        let job = RenderJob {
            tab: tab.id,
            doc: tab.doc.clone(),
            source: tab.source.clone(),
            root: page,
            background: Some(figstudio_render::page_background(&tab.doc, page)),
            images,
            // An empty batch still goes out: it cancels the tiles of the previous one.
            tiles: batch.iter().copied().collect(),
            received: std::mem::take(&mut tab.tiles_received),
        };
        tab.tile_requests = if workers.render(job).is_ok() {
            batch
        } else {
            Vec::new()
        };
    }
}
