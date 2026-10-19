//! The marquee: a primary drag with the Select tool draws a rectangle in page coordinates;
//! right-click inside it opens the region menu (`region_copy.rs`).
//!
//! The geometry is pure (`drag_rect`, `big_enough`, `region_array`) and unit-tested; `update`,
//! `paint` and `on_context` are the egui glue `canvas.rs` calls.

use egui::{Pos2, Rect, Stroke, StrokeKind};
use figstudio_fig::Guid;

use crate::copy::Item;
use crate::tab::Tab;
use crate::theme;

/// A drag shorter than this many screen points on either side is a click, not a marquee.
const MIN_SCREEN_SIDE: f32 = 4.0;

/// The marquee of one tab.
#[derive(Clone, Debug, Default)]
pub struct Marquee {
    /// The rectangle in page coordinates.
    rect: Option<Rect>,
    /// The page it was drawn on; another page clears it.
    page: Option<Guid>,
    /// Page position where the running drag began.
    anchor: Option<Pos2>,
    /// The canvas context menu is the region menu (right-click landed inside the marquee).
    pub menu: bool,
}

impl Marquee {
    pub fn clear(&mut self) {
        *self = Marquee::default();
    }

    /// The rectangle in page coordinates, for the UI tests.
    #[cfg(test)]
    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }
}

/// The rectangle between two page points, whichever way the drag went. `None` if a point is
/// not finite.
pub fn drag_rect(a: Pos2, b: Pos2) -> Option<Rect> {
    (a.is_finite() && b.is_finite()).then(|| Rect::from_two_pos(a, b))
}

/// The marquee is big enough to keep: both sides span `MIN_SCREEN_SIDE` points at `zoom`.
pub fn big_enough(rect: Rect, zoom: f32) -> bool {
    rect.width() * zoom >= MIN_SCREEN_SIDE && rect.height() * zoom >= MIN_SCREEN_SIDE
}

/// `[x, y, w, h]` in page coordinates, the form links and `region::nodes_in` take.
pub fn region_array(rect: Rect) -> [f64; 4] {
    [rect.min.x, rect.min.y, rect.width(), rect.height()].map(f64::from)
}

/// Draws, extends and clears the marquee for this frame. `drawing` is true while a primary drag
/// may start one (Select tool, not panning).
pub fn update(
    ui: &egui::Ui,
    response: &egui::Response,
    tab: &mut Tab,
    origin: Pos2,
    drawing: bool,
) {
    let (zoom, page) = (tab.view.zoom, tab.page);
    let to_page = |p: Pos2| tab.view.to_page(origin, p);
    let m = &mut tab.marquee;
    let esc =
        ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input();
    if m.page.is_some_and(|p| p != page) || esc || response.clicked() {
        m.clear();
    }
    if !drawing {
        m.anchor = None;
        return;
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        let start = ui
            .input(|i| i.pointer.press_origin())
            .or(response.interact_pointer_pos());
        m.anchor = start.map(to_page);
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let (Some(a), Some(p)) = (m.anchor, response.interact_pointer_pos())
        && let Some(rect) = drag_rect(a, to_page(p))
    {
        m.rect = Some(rect);
        m.page = Some(page);
        m.menu = false;
    }
    if response.drag_stopped() {
        m.anchor = None;
        if !m.rect.is_some_and(|r| big_enough(r, zoom)) {
            m.clear();
        }
    }
}

/// A right-click inside the marquee opens the region menu instead of the node menu. Returns
/// true when it did, so the caller leaves the selection alone.
pub fn right_click(response: &egui::Response, tab: &mut Tab, origin: Pos2) -> bool {
    if !response.secondary_clicked() {
        return false;
    }
    let inside = response
        .interact_pointer_pos()
        .map(|p| tab.view.to_page(origin, p))
        .zip(tab.marquee.rect)
        .is_some_and(|(p, r)| r.contains(p));
    tab.marquee.menu = inside;
    inside
}

/// The region menu on `response`, once `right_click` chose it. A picked item goes to
/// `tab.pending_copy`.
pub fn context(response: &egui::Response, tab: &mut Tab) {
    let Some(rect) = tab.marquee.rect else { return };
    let mut picked: Option<Item> = None;
    crate::chrome::context_menu(response, |ui| {
        crate::region_copy::menu(ui, tab, rect, &mut picked);
    });
    tab.pending_copy = picked.or(tab.pending_copy.take());
}

pub fn paint(painter: &egui::Painter, tab: &Tab, origin: Pos2) {
    let Some(rect) = tab.marquee.rect else { return };
    let screen = Rect::from_two_pos(
        tab.view.to_screen(origin, rect.min),
        tab.view.to_screen(origin, rect.max),
    );
    painter.rect_filled(screen, 0.0, theme::ACCENT.gamma_multiply(0.12));
    painter.rect_stroke(
        screen,
        0.0,
        Stroke::new(1.0, theme::ACCENT),
        StrokeKind::Inside,
    );
}

#[cfg(test)]
#[path = "marquee_tests.rs"]
mod tests;
