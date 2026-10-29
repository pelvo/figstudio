//! Motion preview: a window playing a layer's timeline (Properties → Motion → Play), with
//! play/pause and a scrub bar.
//!
//! Frames are drawn by a worker thread with its own renderer (`Renderer::playhead` set to the
//! preview's time); the window shows the newest finished frame and asks for the next one, so a
//! slow frame lowers the frame rate instead of blocking the UI.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Context, TextureHandle, Ui};
use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::motion::Player;
use figstudio_render::images::Images;
use figstudio_render::motion::Playhead;

use crate::source::Source;

/// Longest side of a preview frame, in pixels.
const MAX_SIDE: f64 = 720.0;

/// A playing (or paused) timeline and its window.
pub struct Preview {
    pub root: Guid,
    title: String,
    /// Length of one pass and the loop mode (`once`, `loop`, `boomerang`).
    duration_us: f64,
    mode: &'static str,
    pub time_us: f64,
    pub playing: bool,
    /// egui time of the last advance while playing.
    last_tick: Option<f64>,
    /// Asks the worker for a frame at a time; dropping it ends the worker.
    ask: Sender<f64>,
    frames: Receiver<Option<(f64, egui::ColorImage)>>,
    /// Time of the frame asked for and not back yet.
    waiting: Option<f64>,
    /// Time of the frame shown.
    shown: Option<f64>,
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
    pub fn new(doc: Arc<Document>, root: Guid, source: Option<Source>, images: Images) -> Self {
        let player = Arc::new(Player::new(&doc));
        let (duration_us, mode) = player.timeline(&doc, root);
        let title = doc
            .node(root)
            .map_or_else(|| root.to_string(), |n| n.name.clone());
        let (ask, jobs) = channel::<f64>();
        let (send, frames) = channel();
        crate::worker::spawn_task(move || {
            draw_frames(&doc, root, source, images, player, &jobs, &send)
        });
        Preview {
            root,
            title,
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
        }
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
    fn toggle(&mut self) {
        if !self.playing && self.mode == "once" && self.time_us >= self.period_us() {
            self.time_us = 0.0;
        }
        self.playing = !self.playing;
        self.last_tick = None;
    }
}

/// The worker: draws a frame for the newest time asked for until the preview is closed.
fn draw_frames(
    doc: &Document,
    root: Guid,
    source: Option<Source>,
    images: Images,
    player: Arc<Player>,
    jobs: &Receiver<f64>,
    send: &Sender<Option<(f64, egui::ColorImage)>>,
) {
    let mut renderer = crate::renderer(images);
    let scale = figstudio_render::node_bounds(doc, root).map_or(1.0, |b| {
        (MAX_SIDE / b.width().max(b.height()).max(1.0)).min(2.0)
    });
    let background = doc
        .page_of(root)
        .map(|page| figstudio_render::page_background(doc, page));
    while let Ok(mut time_us) = jobs.recv() {
        // Only the newest request matters.
        while let Ok(newer) = jobs.try_recv() {
            time_us = newer;
        }
        renderer.playhead = Some(Playhead {
            motion: player.clone(),
            time_us,
        });
        let frame = renderer.render_fit_complete(doc, root, scale, 0.0, background, |want| {
            source.as_ref().and_then(|s| s.read(&want.entry).ok())
        });
        let image = frame.map(|f| {
            let size = [
                usize::from(f.pixmap.width()),
                usize::from(f.pixmap.height()),
            ];
            (
                time_us,
                egui::ColorImage::from_rgba_premultiplied(size, f.pixmap.data_as_u8_slice()),
            )
        });
        if send.send(image).is_err() {
            break;
        }
    }
}

/// Shows the tab's motion preview, if one is open: advances the clock, keeps a frame coming
/// and draws the window. Closing the window ends the preview.
pub fn show(ctx: &Context, preview: &mut Option<Preview>) {
    let Some(p) = preview.as_mut() else {
        return;
    };
    let now = ctx.input(|i| i.time);
    if p.playing {
        if let Some(last) = p.last_tick {
            p.advance(now - last);
        }
        p.last_tick = Some(now);
    } else {
        p.last_tick = None;
    }
    while let Ok(done) = p.frames.try_recv() {
        p.waiting = None;
        if let Some((time, image)) = done {
            match &mut p.texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => {
                    p.texture = Some(ctx.load_texture(
                        "motion-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            p.shown = Some(time);
        }
    }
    if p.waiting.is_none() && p.shown != Some(p.time_us) && p.ask.send(p.time_us).is_ok() {
        p.waiting = Some(p.time_us);
    }
    let mut open = true;
    egui::Window::new(format!("Motion · {}", p.title))
        .id(egui::Id::new("motion-preview"))
        .open(&mut open)
        .resizable(true)
        .default_width(480.0)
        .show(ctx, |ui| window(ui, p));
    if p.playing || p.waiting.is_some() {
        ctx.request_repaint();
    }
    if !open {
        *preview = None;
    }
}

fn window(ui: &mut Ui, p: &mut Preview) {
    if let Some(texture) = &p.texture {
        let size = texture.size_vec2();
        let width = ui.available_width().max(64.0);
        let scale = (width / size.x).min(1.0);
        ui.image((texture.id(), size * scale));
    } else {
        ui.spinner();
    }
    ui.horizontal(|ui| {
        let label = if p.playing { "Pause" } else { "Play" };
        if ui.button(label).clicked() {
            p.toggle();
        }
        let period = p.period_us();
        let mut ms = p.time_us / 1e3;
        let slider = egui::Slider::new(&mut ms, 0.0..=period / 1e3)
            .show_value(false)
            .clamping(egui::SliderClamping::Always);
        if ui.add(slider).changed() {
            p.time_us = ms * 1e3;
            p.playing = false;
        }
        ui.label(format!("{:.0} / {:.0} ms", p.time_us / 1e3, period / 1e3));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(mode: &'static str) -> Preview {
        let (ask, _) = channel();
        let (_, frames) = channel();
        Preview {
            root: Guid(0, 0),
            title: String::new(),
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
