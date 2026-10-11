//! A canvas that remembers where whole-node shadow redraws drew, so a discarded one can be
//! erased exactly. Wraps whatever the renderer draws to (a Vello context or a `Recorder`).

use vello_cpu::PaintType;
use vello_cpu::filter_effects::Filter;
use vello_cpu::kurbo::{Affine, BezPath, Rect};
use vello_cpu::peniko::Fill;

use crate::paint::linear_scale;
use crate::record::Canvas;

pub(crate) struct Tracked<'a> {
    inner: &'a mut dyn Canvas,
    transform: Affine,
    /// Filter reach in output pixels at each open layer or clip (summed over enclosing filters).
    reach: Vec<f64>,
    /// One entry per active redraw (innermost last): the reach already open when it began, and
    /// the output bounds drawn since.
    drawn: Vec<(f64, Option<Rect>)>,
}

impl<'a> Tracked<'a> {
    pub(crate) fn new(inner: &'a mut dyn Canvas) -> Self {
        Tracked {
            inner,
            transform: Affine::IDENTITY,
            reach: Vec::new(),
            drawn: Vec::new(),
        }
    }

    /// Starts recording where a redraw draws (call it inside the redraw's own layer).
    pub(crate) fn begin_track(&mut self) {
        self.drawn
            .push((self.reach.last().copied().unwrap_or(0.0), None));
    }

    /// Ends the innermost recording: the output bounds of everything drawn in it, spread by
    /// the filters opened inside it (what its layer holds before its own filter runs).
    pub(crate) fn end_track(&mut self) -> Option<Rect> {
        self.drawn.pop().and_then(|(_, area)| area)
    }

    fn open(&mut self, extra: f64) {
        let outer = self.reach.last().copied().unwrap_or(0.0);
        self.reach.push(outer + extra);
    }
}

impl Canvas for Tracked<'_> {
    fn set_transform(&mut self, t: Affine) {
        self.transform = t;
        self.inner.set_transform(t);
    }
    fn set_paint(&mut self, paint: PaintType) {
        self.inner.set_paint(paint);
    }
    fn set_paint_transform(&mut self, t: Affine) {
        self.inner.set_paint_transform(t);
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        self.inner.set_fill_rule(rule);
    }
    fn fill_path(&mut self, path: &BezPath) {
        if !self.drawn.is_empty() {
            let open = self.reach.last().copied().unwrap_or(0.0);
            // Control points bound the curve.
            let mut bbox = self.transform.transform_rect_bbox(path.control_box());
            if !bbox.is_finite() {
                bbox = Rect::new(-1e9, -1e9, 1e9, 1e9);
            }
            // Every active redraw sees it, spread by the filters opened since that one began
            // (a nested redraw's own shadow layer shifts and blurs what it holds).
            for (base, drawn) in &mut self.drawn {
                let r = open - *base + 1.0;
                let area = bbox.inflate(r, r);
                *drawn = Some(drawn.map_or(area, |d| d.union(area)));
            }
        }
        self.inner.fill_path(path);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        self.open(0.0);
        self.inner.push_clip_layer(path);
    }
    fn push_clip_path(&mut self, path: &BezPath) {
        self.open(0.0);
        self.inner.push_clip_path(path);
    }
    fn pop_clip(&mut self) {
        self.reach.pop();
        self.inner.pop_clip();
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        self.open(0.0);
        self.inner.push_opacity_layer(opacity);
    }
    fn push_filter_layer(&mut self, filter: Filter, reach: f64) {
        self.open(reach.abs() * linear_scale(self.transform));
        self.inner.push_filter_layer(filter, reach);
    }
    fn pop_layer(&mut self) {
        self.reach.pop();
        self.inner.pop_layer();
    }
    fn clear_layer(&mut self, area: Rect) {
        // The erase is not drawing: it is not recorded.
        self.transform = Affine::IDENTITY;
        self.inner.clear_layer(area);
    }
}
