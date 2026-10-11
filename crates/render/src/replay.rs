//! A page walked once at one zoom and replayed for each tile of that zoom (feature
//! `parallel`). The walk dominates a tile's cost on a large page; a replay only rasterizes the
//! recorded commands whose output area touches the tile.

use vello_cpu::kurbo::{Affine, Rect};
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use crate::images::ImageRequest;
use crate::record::Recorder;
use crate::{Frame, MAX_RENDER_WORK, Renderer, Stats, Target, Viewport};

/// The drawing of a page at one page-to-pixel transform, kept for replays.
///
/// Only what can reach the recorded pixel `area` is recorded (culled as a render of that area
/// would be), so a viewport inside it replays like a direct render. Not the whole page: that
/// would decode and hold every image of the page at this zoom. Each replay reports
/// [`Recording::stats`]: the counts of the whole area, not of the replayed viewport.
pub struct Recording {
    rec: Recorder,
    stats: Stats,
    transform: Affine,
    area: Rect,
}

impl std::fmt::Debug for Recording {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recording")
            .field("bytes", &self.rec.bytes())
            .field("stats", &self.stats)
            .field("area", &self.area)
            .finish_non_exhaustive()
    }
}

impl Recording {
    /// Whether recorded pixels `window` lie inside the recorded area.
    pub fn covers(&self, window: Rect) -> bool {
        let a = self.area;
        window.x0 >= a.x0 && window.y0 >= a.y0 && window.x1 <= a.x1 && window.y1 <= a.y1
    }

    /// What the recording walk drew, approximated and skipped (work budget included).
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Estimated bytes held (commands and paths; shared image pixels not counted).
    pub fn bytes(&self) -> usize {
        self.rec.bytes()
    }

    /// Renders `viewport` from the recording, like [`Renderer::render`] would. Returns the
    /// frame (its `stats` are [`Recording::stats`], its `wanted` empty) and how many fills
    /// were replayed, or `None` when the viewport is not the recorded transform moved by a
    /// translation (another zoom, rotation or skew) or reaches outside the recorded area.
    pub fn render(&self, viewport: &Viewport) -> Option<(Frame, usize)> {
        let [a, b, c, d, tx, ty] = viewport.transform.as_coeffs();
        let [ra, rb, rc_, rd, rtx, rty] = self.transform.as_coeffs();
        if [a, b, c, d] != [ra, rb, rc_, rd] {
            return None;
        }
        let (dx, dy) = (tx - rtx, ty - rty);
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        // The viewport's pixels, in recorded pixels.
        let window = Rect::new(-dx, -dy, f64::from(w) - dx, f64::from(h) - dy);
        if !self.covers(window) {
            return None;
        }
        let mut rc = RenderContext::new_with(w, h, RenderSettings::default());
        if let Some(bg) = viewport.background {
            rc.set_paint(bg);
            rc.fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        }
        let fills = self
            .rec
            .replay(&mut rc, window, Affine::translate((dx, dy)));
        rc.flush();
        let mut pixmap = Pixmap::new(w, h);
        rc.render(&mut pixmap, &mut Resources::new());
        let frame = Frame {
            pixmap,
            stats: self.stats.clone(),
            wanted: Vec::new(),
        };
        Some((frame, fills))
    }
}

impl Renderer {
    /// Walks `root` once with page-to-pixel `transform`, recording what can reach the output
    /// pixels `area`, within the usual work budget ([`MAX_RENDER_WORK`]).
    ///
    /// Returns the images the walk asked for (read them as for a [`Frame`]) and the recording,
    /// or `None` when it would hold over about `limit` bytes; render directly then.
    pub fn record(
        &mut self,
        doc: &figstudio_fig::Document,
        root: figstudio_fig::Guid,
        transform: Affine,
        area: Rect,
        limit: usize,
    ) -> (Option<Recording>, Vec<ImageRequest>) {
        let mut rec = Recorder::with_limit(limit);
        let (stats, wanted) = self.walk(
            doc,
            Target::Node(root),
            transform,
            area,
            &mut rec,
            MAX_RENDER_WORK,
            0,
        );
        let recording = (!rec.over()).then_some(Recording {
            rec,
            stats,
            transform,
            area,
        });
        (recording, wanted)
    }
}
