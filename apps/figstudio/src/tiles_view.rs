//! Draws the canvas from cached tiles (`tiles.rs`) and asks the render worker for the
//! missing ones, nearest the view centre first.

use egui::{Color32, Pos2, Rect};
use figstudio_render::Stats;
use web_time::Duration;

use crate::recordings::Pixels;
use crate::tab::Tab;
use crate::tiles::{self, Cache, TILE_BYTES};
use crate::worker::{RenderDone, RenderJob, Workers};

/// Budget for the tiles kept as textures (resident cache, gutters included): 256 MiB native,
/// 128 MiB in the browser. Tiles still being rendered or uploaded are not counted: at most
/// one batch per tab (half the cache) is queued in the worker, plus, in the browser, at most
/// `gpu_scenes::MAX_PENDING` scenes waiting for the main thread, all scenes alive within
/// `gpu_credits::BUDGET` bytes.
#[cfg(not(target_arch = "wasm32"))]
const BUDGET_BYTES: usize = 256 << 20;
#[cfg(target_arch = "wasm32")]
const BUDGET_BYTES: usize = 128 << 20;

/// A rendered tile; its pixels live only in the texture (egui's, or the GPU's own).
pub struct Tile {
    texture: Texture,
    elapsed: Duration,
}

enum Texture {
    Cpu(egui::TextureHandle),
    Gpu(crate::gpu_registry::NativeTexture),
}

impl Texture {
    fn id(&self) -> egui::TextureId {
        match self {
            Texture::Cpu(t) => t.id(),
            Texture::Gpu(t) => t.id,
        }
    }
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
    /// Tiles asked for before this many images had arrived were drawn with old fonts or old
    /// shader code: dropped when they come back, and asked for again.
    font_stamp: u64,
    /// Registers GPU tiles with egui (desktop with the GPU canvas).
    #[cfg(not(target_arch = "wasm32"))]
    pub gpu: Option<crate::gpu_tiles::Registry>,
    /// Renders the worker's scenes on the main thread (browser with the GPU canvas).
    #[cfg(target_arch = "wasm32")]
    pub gpu: Option<crate::gpu_tiles_web::WebGpu>,
}

impl Default for Tiles {
    fn default() -> Self {
        Tiles {
            cache: Cache::new(BUDGET_BYTES / TILE_BYTES),
            font_stamp: 0,
            gpu: None,
        }
    }
}

impl Tiles {
    /// Drops the tiles of a closed document.
    pub fn forget(&mut self, doc: u64) {
        self.cache.retain(|k| k.doc != doc);
        #[cfg(target_arch = "wasm32")]
        if let Some(gpu) = &mut self.gpu {
            gpu.forget(doc);
        }
    }

    /// Scenes wait for the main thread (browser): keep repainting.
    #[cfg(target_arch = "wasm32")]
    pub fn gpu_busy(&self) -> bool {
        self.gpu
            .as_ref()
            .is_some_and(crate::gpu_tiles_web::WebGpu::busy)
    }

    /// Fonts or shader code changed at `stamp` (`Workers::images_arrived` after
    /// `Workers::fonts_changed`).
    pub fn fonts_changed(&mut self, stamp: u64) {
        self.font_stamp = stamp;
    }

    /// Renders queued browser scenes; failed tiles are requested again on the CPU.
    #[cfg(target_arch = "wasm32")]
    pub fn pump_gpu(&mut self, tabs: &mut [Tab]) {
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        let failed = gpu.take_failed();
        self.cache.retain(|k| !failed.contains(k));
        let cache = &mut self.cache;
        gpu.pump(cache.plan(), |done, texture| {
            let Some(tab) = tabs.iter_mut().find(|t| t.id == done.tab) else {
                return;
            };
            if done.images < self.font_stamp {
                tab.tiles_received.push((done.key, done.images));
                return;
            }
            received(tab, &done);
            if let Some(texture) = texture {
                store(cache, &done, Texture::Gpu(texture));
            }
        });
    }

    /// Uploads a finished tile and stores it.
    pub fn accept(&mut self, ctx: &egui::Context, tab: &mut Tab, done: RenderDone) {
        if done.images < self.font_stamp {
            tab.tiles_received.push((done.key, done.images));
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let (Pixels::Gpu(_), Some(gpu)) = (&done.pixels, &mut self.gpu) {
            // Still requested and not received until the main thread renders (or drops) it.
            gpu.push(done, self.cache.plan(), |d| received(tab, &d));
            return;
        }
        received(tab, &done);
        let texture = match &done.pixels {
            Pixels::Cpu { size, rgba } => {
                let image = egui::ColorImage::from_rgba_premultiplied(*size, rgba);
                Texture::Cpu(ctx.load_texture("tile", image, egui::TextureOptions::LINEAR))
            }
            #[cfg(not(target_arch = "wasm32"))]
            Pixels::Gpu(texture) => match &self.gpu {
                Some(registry) => Texture::Gpu(registry.register(texture.clone())),
                // Not reached: GPU tiles come only from workers given the registry's device.
                None => return,
            },
            // Not reached: scenes come only from workers given the main thread's renderer.
            #[cfg(target_arch = "wasm32")]
            Pixels::Gpu(_) => return,
        };
        store(&mut self.cache, &done, texture);
    }

    /// Paints the cached tiles of the tab's view onto `rect` (best fallback under missing
    /// ones), cut to its rounded corners of `radius`, and requests the missing tiles plus a
    /// one-tile ring.
    pub fn show(
        &mut self,
        painter: &egui::Painter,
        (rect, radius): (Rect, f32),
        tab: &mut Tab,
        target: crate::tab::View,
        workers: &mut Workers,
        ppp: f32,
    ) {
        // Drawn: what `tab.view` shows (an ease in progress, maybe). Planned and requested:
        // the `target` view it is heading for.
        let (doc, page, view) = (tab.doc.instance, tab.page, tab.view);
        let visible = tiles::visible_page(view.zoom, view.pan.into(), rect.size().into());
        let planned = tiles::visible_page(target.zoom, target.pan.into(), rect.size().into());
        let range = tiles::plan(
            tiles::level_for(target.zoom * ppp),
            planned,
            self.cache.budget(),
        );
        let level = range.level;
        self.cache.begin_frame(doc, page, range);
        let mut stats: Option<Stats> = None;
        let mut elapsed = Duration::ZERO;
        let mut fallbacks = 0;
        for key in self.cache.draw_order(doc, page, level, visible) {
            self.cache.touch(&key);
            let Some(entry) = self.cache.get(&key) else {
                continue;
            };
            paint_key(
                painter,
                &key,
                entry.value.texture.id(),
                view,
                (rect, radius),
            );
            if key.level == level {
                fallbacks += entry.stats.gpu_fallback_tiles;
                stats.get_or_insert_default().max_with(&entry.stats);
                elapsed += entry.value.elapsed;
            }
        }
        // The status bar describes the sharp tiles on screen (each count is a lower bound: the
        // largest of any one tile); it keeps the last numbers while none has arrived.
        if let Some(mut stats) = stats {
            // Added up, unlike the other counts: how many sharp tiles on screen are CPU tiles.
            stats.gpu_fallback_tiles = fallbacks;
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

impl Tiles {
    /// Paints the cached tiles of `doc`'s `page` where `view` puts them, coarser ones first,
    /// without asking for more: the picture another file or page fades in over
    /// (`chrome::Crossfade`). Returns the tiles painted, for [`Tiles::keep_drawn`].
    pub fn paint_cached(
        &self,
        painter: &egui::Painter,
        (rect, radius): (Rect, f32),
        (doc, page): (u64, figstudio_fig::Guid),
        view: crate::tab::View,
        ppp: f32,
    ) -> Vec<tiles::Key> {
        let visible = tiles::visible_page(view.zoom, view.pan.into(), rect.size().into());
        let level = tiles::level_for(view.zoom * ppp);
        let keys = self.cache.draw_order(doc, page, level, visible);
        for key in &keys {
            if let Some(entry) = self.cache.get(key) {
                paint_key(painter, key, entry.value.texture.id(), view, (rect, radius));
            }
        }
        keys
    }

    /// Marks tiles painted outside [`Tiles::show`] as on screen this frame. Called after it,
    /// once the frame's plan is set: a tile last drawn in an earlier frame is evicted first.
    pub fn keep_drawn(&mut self, keys: &[tiles::Key]) {
        for key in keys {
            self.cache.touch(key);
        }
    }

    /// How many tiles of `doc` are cached, and how many of them this frame drew.
    #[cfg(test)]
    pub fn count(&self, doc: u64) -> (usize, usize) {
        self.cache.count(doc)
    }
}

/// Paints the tile `key` where `view` puts it on the card `rect`.
fn paint_key(
    painter: &egui::Painter,
    key: &tiles::Key,
    texture: egui::TextureId,
    view: crate::tab::View,
    (rect, radius): (Rect, f32),
) {
    let r = key.page_rect();
    let corner = |x: f64, y: f64| view.to_screen(rect.min, Pos2::new(x as f32, y as f32));
    let uv = tiles::uv();
    paint_tile(
        painter,
        texture,
        Rect::from_min_max(corner(r.x0, r.y0), corner(r.x1, r.y1)),
        Rect::from_min_max(Pos2::new(uv[0], uv[0]), Pos2::new(uv[1], uv[1])),
        (rect, radius),
    );
}

/// Paints a tile at `screen`; where it reaches into a rounded corner of `card`, only the part
/// inside the corner is drawn (a textured polygon), so the canvas card keeps round corners.
fn paint_tile(
    painter: &egui::Painter,
    texture: egui::TextureId,
    screen: Rect,
    uv: Rect,
    (card, radius): (Rect, f32),
) {
    let polygon = rounded_clip(screen, card, radius);
    let Some(polygon) = polygon else {
        painter.image(texture, screen, uv, Color32::WHITE);
        return;
    };
    if polygon.len() < 3 {
        return;
    }
    let mut mesh = egui::Mesh::with_texture(texture);
    for p in &polygon {
        let at = egui::vec2(
            (p.x - screen.left()) / screen.width(),
            (p.y - screen.top()) / screen.height(),
        );
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *p,
            uv: uv.lerp_inside(at),
            color: Color32::WHITE,
        });
    }
    for i in 1..polygon.len() as u32 - 1 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// The rounded rectangle `card` (corner `radius`) as a clockwise polygon, 8 segments a corner.
fn rounded_polygon(card: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.min(card.width() / 2.0).min(card.height() / 2.0);
    [
        (Pos2::new(card.left() + r, card.top() + r), 180.0_f32),
        (Pos2::new(card.right() - r, card.top() + r), 270.0),
        (Pos2::new(card.right() - r, card.bottom() - r), 0.0),
        (Pos2::new(card.left() + r, card.bottom() - r), 90.0),
    ]
    .into_iter()
    .flat_map(|(centre, start)| {
        (0..=8).map(move |step| {
            let a = (start + step as f32 * 90.0 / 8.0).to_radians();
            centre + egui::vec2(a.cos(), a.sin()) * r
        })
    })
    .collect()
}

/// `rect` cut to the rounded `card`, or `None` when it misses every rounded corner (then it
/// is drawn whole). Sutherland–Hodgman against the convex corner polygon.
fn rounded_clip(rect: Rect, card: Rect, radius: f32) -> Option<Vec<Pos2>> {
    let r = radius.min(card.width() / 2.0).min(card.height() / 2.0);
    if r <= 0.0 {
        return None;
    }
    let size = egui::vec2(r, r);
    let corners = [
        Rect::from_min_size(card.left_top(), size),
        Rect::from_min_size(card.right_top() - egui::vec2(r, 0.0), size),
        Rect::from_min_size(card.right_bottom() - size, size),
        Rect::from_min_size(card.left_bottom() - egui::vec2(0.0, r), size),
    ];
    if !corners.iter().any(|c| c.intersects(rect)) {
        return None;
    }
    let clip = rounded_polygon(card, radius);
    let mut out = vec![
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let inside = |a: Pos2, b: Pos2, p: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x >= 0.0;
    for (i, &a) in clip.iter().enumerate() {
        let b = clip[(i + 1) % clip.len()];
        let input = std::mem::take(&mut out);
        for (j, &p) in input.iter().enumerate() {
            let q = input[(j + 1) % input.len()];
            let (p_in, q_in) = (inside(a, b, p), inside(a, b, q));
            if p_in {
                out.push(p);
            }
            if p_in != q_in {
                // Where p→q crosses the line a→b.
                let d = q - p;
                let e = b - a;
                let denom = d.x * e.y - d.y * e.x;
                if denom.abs() > f32::EPSILON {
                    let t = ((a - p).x * e.y - (a - p).y * e.x) / denom;
                    out.push(p + d * t);
                }
            }
        }
        if out.is_empty() {
            break;
        }
    }
    Some(out)
}

/// The UI has the tile (or dropped it): no longer requested, and the worker hears it was
/// received, so a later request for it is rendered again (`tile_queue::Queue`).
pub(crate) fn received(tab: &mut Tab, done: &RenderDone) {
    tab.tile_requests.retain(|k| *k != done.key);
    tab.tiles_received.push((done.key, done.images));
}

/// Stores `done`'s tile, drawn as `texture` (its pixels are not used).
fn store(cache: &mut Cache<Tile>, done: &RenderDone, texture: Texture) {
    let images = (done.stats.images_loading > 0).then_some(done.images);
    let tile = Tile {
        texture,
        elapsed: done.elapsed,
    };
    cache.insert(done.key, tile, done.stats.clone(), images);
}

#[cfg(test)]
#[path = "tiles_font_tests.rs"]
mod font_tests;

#[cfg(test)]
mod rounded_tests {
    use super::*;

    fn area(points: &[Pos2]) -> f32 {
        let n = points.len();
        (0..n)
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % n]);
                a.x * b.y - b.x * a.y
            })
            .sum::<f32>()
            .abs()
            / 2.0
    }

    #[test]
    fn tiles_in_a_rounded_corner_are_cut_and_others_are_drawn_whole() {
        let card = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let middle = Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(100.0, 100.0));
        assert!(rounded_clip(middle, card, 12.0).is_none());
        assert!(rounded_clip(card, card, 0.0).is_none(), "square corners");
        let corner = Rect::from_min_size(Pos2::new(-50.0, -50.0), egui::vec2(100.0, 100.0));
        let cut = rounded_clip(corner, card, 12.0).expect("cut");
        // The 50×50 part on the card, less the corner outside a radius-12 arc.
        let expected = 50.0 * 50.0 - (4.0 - std::f32::consts::PI) / 4.0 * 144.0;
        assert!(
            (area(&cut) - expected).abs() < 2.0,
            "{} vs {expected}",
            area(&cut)
        );
        let whole = rounded_clip(card.expand(10.0), card, 12.0).expect("cut");
        let expected = 400.0 * 300.0 - (4.0 - std::f32::consts::PI) * 144.0;
        assert!(
            (area(&whole) - expected).abs() < 8.0,
            "{} vs {expected}",
            area(&whole)
        );
        let outside = Rect::from_min_size(Pos2::new(-30.0, -30.0), egui::vec2(20.0, 20.0));
        assert!(rounded_clip(outside, card, 12.0).is_none_or(|p| area(&p) < 0.01));
    }
}
