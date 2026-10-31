//! Motion preview: plays a layer's timeline in place on the canvas (Properties → Motion →
//! Play), with a bar under it to play or pause, scrub and close.
//!
//! Frames are drawn by a worker thread with its own renderer (`Renderer::playhead` set to the
//! preview's time). A frame is the page region under the layer, so what is behind it shows as
//! on the canvas, drawn at the canvas zoom. The canvas shows the newest finished frame and asks
//! for the next one, so a slow frame lowers the frame rate instead of blocking the UI.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Context, Pos2, Rect, TextureHandle};
use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::motion::Player;
use figstudio_render::Viewport;
use figstudio_render::images::Images;
use figstudio_render::kurbo::{self, Affine};
use figstudio_render::motion::Playhead;

use crate::source::Source;
use crate::tab::View;

/// Longest side of a preview frame, in pixels.
const MAX_SIDE: f32 = 2048.0;

/// A playing (or paused) timeline, shown over its layer on the canvas.
pub struct Preview {
    pub root: Guid,
    /// The layer's page and its box there, in page units: where frames are painted.
    page: Guid,
    region: Rect,
    /// Pixels per page unit to draw at (the canvas zoom, rounded to a quarter octave).
    scale: f32,
    /// Length of one pass and the loop mode (`once`, `loop`, `boomerang`).
    duration_us: f64,
    mode: &'static str,
    pub time_us: f64,
    pub playing: bool,
    /// egui time of the last advance while playing.
    last_tick: Option<f64>,
    /// Asks the worker for a frame at a time and scale; dropping it ends the worker.
    ask: Sender<(f64, f32)>,
    frames: Receiver<(f64, f32, egui::ColorImage)>,
    /// Time and scale of the frame asked for and not back yet.
    waiting: Option<(f64, f32)>,
    /// Time and scale of the frame shown.
    shown: Option<(f64, f32)>,
    texture: Option<TextureHandle>,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview")
            .field("root", &self.root)
            .field("time_us", &self.time_us)
            .field("playing", &self.playing)
            .finish_non_exhaustive()
    }
}

impl Preview {
    /// A preview of `root`'s timeline, playing from the start.
    /// `None` when `root` is not on a page.
    pub fn new(
        doc: Arc<Document>,
        root: Guid,
        source: Option<Source>,
        images: Images,
    ) -> Option<Self> {
        let page = doc.page_of(root)?;
        // shortcut: motion that moves past the layer's box is cut at it; use the content
        // bounds over the whole timeline if that matters.
        let bounds = figstudio_render::node_bounds(&doc, root)?;
        let player = Arc::new(Player::new(&doc));
        let (duration_us, mode) = player.timeline(&doc, root);
        let (ask, jobs) = channel();
        let (send, frames) = channel();
        crate::worker::spawn_task(move || {
            draw_frames(&doc, page, bounds, source, images, player, &jobs, &send)
        });
        Some(Preview {
            root,
            page,
            region: Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            ),
            scale: 1.0,
            duration_us,
            mode,
            time_us: 0.0,
            playing: true,
            last_tick: None,
            ask,
            frames,
            waiting: None,
            shown: None,
            texture: None,
        })
    }

    /// Whether a frame has been drawn and shown.
    #[cfg(test)]
    pub fn has_frame(&self) -> bool {
        self.texture.is_some()
    }

    /// Length of the scrub bar (µs): a boomerang plays there and back.
    pub fn period_us(&self) -> f64 {
        let d = self.duration_us.max(1.0);
        if self.mode == "boomerang" { 2.0 * d } else { d }
    }

    /// Moves the playhead by `elapsed` seconds of playing: a once timeline stops at its end,
    /// loops wrap.
    pub fn advance(&mut self, elapsed: f64) {
        let period = self.period_us();
        let t = self.time_us + elapsed.max(0.0) * 1e6;
        if self.mode == "once" {
            self.time_us = t.min(period);
            if t >= period {
                self.playing = false;
            }
        } else {
            self.time_us = t.rem_euclid(period);
        }
    }

    /// Restart from the beginning when play is pressed at the end of a once timeline.
    pub fn toggle(&mut self) {
        if !self.playing && self.mode == "once" && self.time_us >= self.period_us() {
            self.time_us = 0.0;
        }
        self.playing = !self.playing;
        self.last_tick = None;
    }
}

/// The worker: draws the page region of the layer for the newest time and scale asked for,
/// until the preview is closed.
#[expect(
    clippy::too_many_arguments,
    reason = "the worker's inputs, moved into its thread"
)]
fn draw_frames(
    doc: &Document,
    page: Guid,
    region: kurbo::Rect,
    source: Option<Source>,
    images: Images,
    player: Arc<Player>,
    jobs: &Receiver<(f64, f32)>,
    send: &Sender<(f64, f32, egui::ColorImage)>,
) {
    let mut renderer = crate::renderer(images);
    let background = Some(figstudio_render::page_background(doc, page));
    while let Ok(mut job) = jobs.recv() {
        // Only the newest request matters.
        while let Ok(newer) = jobs.try_recv() {
            job = newer;
        }
        let (time_us, scale) = job;
        renderer.playhead = Some(Playhead {
            motion: player.clone(),
            time_us,
        });
        let s = f64::from(scale);
        let side = |v: f64| (v * s).ceil().clamp(1.0, f64::from(u16::MAX)) as u16;
        let viewport = Viewport {
            width: side(region.width()),
            height: side(region.height()),
            transform: Affine::scale(s) * Affine::translate((-region.x0, -region.y0)),
            background,
        };
        let frame = renderer.render_complete(doc, page, &viewport, |want| {
            source.as_ref().and_then(|s| s.read(&want.entry).ok())
        });
        let size = [
            usize::from(frame.pixmap.width()),
            usize::from(frame.pixmap.height()),
        ];
        let image =
            egui::ColorImage::from_rgba_premultiplied(size, frame.pixmap.data_as_u8_slice());
        if send.send((time_us, scale, image)).is_err() {
            break;
        }
    }
}

/// Runs the tab's preview, if one is open: advances the clock, takes finished frames and asks
/// for the next one.
pub fn tick(ctx: &Context, preview: &mut Option<Preview>) {
    let Some(p) = preview.as_mut() else {
        return;
    };
    let now = ctx.input(|i| i.time);
    let was_playing = p.playing;
    if p.playing {
        if let Some(last) = p.last_tick {
            p.advance(now - last);
        }
        p.last_tick = Some(now);
    } else {
        p.last_tick = None;
    }
    while let Ok((time, scale, image)) = p.frames.try_recv() {
        p.waiting = None;
        match &mut p.texture {
            Some(t) => t.set(image, egui::TextureOptions::LINEAR),
            None => {
                p.texture =
                    Some(ctx.load_texture("motion-preview", image, egui::TextureOptions::LINEAR));
            }
        }
        p.shown = Some((time, scale));
    }
    let want = (p.time_us, p.scale);
    if p.waiting.is_none() && p.shown != Some(want) && p.ask.send(want).is_ok() {
        p.waiting = Some(want);
    }
    // One more frame after the clock stops, so the bar shows Play again.
    if p.playing || was_playing || p.waiting.is_some() {
        ctx.request_repaint();
    }
}

impl Preview {
    /// The layer's box on screen, when its page is the one shown.
    pub fn on_screen(&self, page: Guid, view: View, origin: Pos2) -> Option<Rect> {
        (page == self.page).then(|| {
            Rect::from_min_max(
                view.to_screen(origin, self.region.min),
                view.to_screen(origin, self.region.max),
            )
        })
    }

    /// Paints the newest frame over the layer, and asks for frames at the canvas zoom (`ppp`:
    /// pixels per point).
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        page: Guid,
        view: View,
        origin: Pos2,
        ppp: f32,
    ) {
        let Some(screen) = self.on_screen(page, view, origin) else {
            return;
        };
        let longest = self.region.width().max(self.region.height()).max(1.0);
        let scale = (view.zoom * ppp).min(MAX_SIDE / longest).max(1e-3);
        // Quarter octaves, so a zoom does not ask for a new frame at every step.
        self.scale = ((scale.log2() * 4.0).round() / 4.0).exp2();
        if let Some(texture) = &self.texture {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(texture.id(), screen, uv, egui::Color32::WHITE);
        }
    }

    /// Moves the playhead to `time_us` and pauses (the scrub track).
    pub fn scrub(&mut self, time_us: f64) {
        self.time_us = time_us.clamp(0.0, self.period_us());
        self.playing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(mode: &'static str) -> Preview {
        let (ask, _) = channel();
        let (_, frames) = channel();
        Preview {
            root: Guid(0, 0),
            page: Guid(0, 1),
            region: Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 10.0)),
            scale: 1.0,
            duration_us: 1_000.0,
            mode,
            time_us: 0.0,
            playing: true,
            last_tick: None,
            ask,
            frames,
            waiting: None,
            shown: None,
            texture: None,
        }
    }

    #[test]
    fn a_once_timeline_stops_at_its_end_and_play_restarts_it() {
        let mut p = preview("once");
        p.advance(0.0006);
        assert!((p.time_us - 600.0).abs() < 1e-6 && p.playing);
        p.advance(0.0006);
        assert_eq!(p.time_us, 1_000.0);
        assert!(!p.playing);
        p.toggle();
        assert_eq!(p.time_us, 0.0);
        assert!(p.playing);
    }

    #[test]
    fn loops_wrap_and_a_boomerang_runs_there_and_back() {
        let mut p = preview("loop");
        p.advance(0.0013);
        assert!((p.time_us - 300.0).abs() < 1e-6);
        let mut b = preview("boomerang");
        assert_eq!(b.period_us(), 2_000.0);
        b.advance(0.0013);
        assert!((b.time_us - 1_300.0).abs() < 1e-6);
    }
}
