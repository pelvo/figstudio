//! Prototype Preview on the canvas: one screen alone, fitted; a click on a link follows it.
//! The tiles draw `Tab::root` (the screen), so CPU, GPU and the browser work unchanged.
use egui::{Pos2, Sense, Stroke};
use figstudio_render::kurbo::Point;
use figstudio_render::prototype::Pointer;

use crate::canvas::to_color32;
use crate::tab::{Tab, View};
use crate::theme;
use crate::tiles_view::Tiles;
use crate::worker::Workers;

/// How long the links flash after a click that hit none, in seconds.
const FLASH: f64 = 0.8;

/// How the screen is sized in the window: Figma's Present and Preview options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scaling {
    /// The whole screen in view; a smaller screen keeps its size (Figma's "scale down").
    #[default]
    Fit,
    /// The screen's width fills the window, never enlarged; a taller screen scrolls.
    FitWidth,
    /// The screen covers the window; what sticks out scrolls.
    Fill,
    /// One screen pixel per point; a larger screen scrolls.
    ActualSize,
}

impl Scaling {
    pub const ALL: [Scaling; 4] = [
        Scaling::Fit,
        Scaling::FitWidth,
        Scaling::Fill,
        Scaling::ActualSize,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Scaling::Fit => "Fit",
            Scaling::FitWidth => "Fit width",
            Scaling::Fill => "Fill",
            Scaling::ActualSize => "Actual size",
        }
    }
}

/// Figma's Fit keeps this much room around the screen (points; measured in Figma's Present).
const FIT_MARGIN: egui::Vec2 = egui::vec2(48.0, 60.0);

/// The pan that keeps page-space box `screen` on a `window`-sized canvas at `view`'s zoom:
/// centred along an axis where it fits, its edges at most at the window's where it is larger.
pub fn keep_on_window(screen: egui::Rect, window: egui::Vec2, view: View) -> egui::Vec2 {
    let axis = |pan: f32, min: f32, len: f32, win: f32| {
        let size = len * view.zoom;
        let at = pan + min * view.zoom;
        let at = if size <= win {
            (win - size) / 2.0
        } else {
            at.clamp(win - size, 0.0)
        };
        at - min * view.zoom
    };
    egui::vec2(
        axis(view.pan.x, screen.min.x, screen.width(), window.x),
        axis(view.pan.y, screen.min.y, screen.height(), window.y),
    )
}

/// The view that shows page-space box `screen` in `window` (points) by `scaling`: centred
/// where it is smaller than the window, from its top left where it is larger.
pub fn view_for(screen: egui::Rect, window: egui::Vec2, scaling: Scaling) -> View {
    let size = screen.size().max(egui::Vec2::splat(1.0));
    let room = (window - FIT_MARGIN * 2.0).max(egui::Vec2::splat(1.0));
    let (fit_x, fit_y) = (window.x / size.x, window.y / size.y);
    let zoom = match scaling {
        Scaling::Fit => (room.x / size.x).min(room.y / size.y).min(1.0),
        Scaling::FitWidth => fit_x.min(1.0),
        Scaling::Fill => fit_x.max(fit_y),
        Scaling::ActualSize => 1.0,
    }
    .clamp(crate::tab::MIN_ZOOM, crate::tab::MAX_ZOOM);
    let shown = size * zoom;
    let offset = |win: f32, len: f32| if len < win { (win - len) / 2.0 } else { 0.0 };
    let at = egui::vec2(offset(window.x, shown.x), offset(window.y, shown.y));
    View {
        zoom,
        pan: at - screen.min.to_vec2() * zoom,
    }
}

/// Draws the Preview screen and handles its input. Returns the page-space cursor position.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    workers: &mut Workers,
    tiles: &mut Tiles,
    radius: f32,
    scaling: Scaling,
) -> Option<Pos2> {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let origin = rect.min;
    tab.canvas_rect = Some(rect);
    // A field that had the keys before Preview started (the link dialog's, say) would keep
    // them and block the arrows: Preview takes the keyboard when it starts.
    if tab
        .preview
        .as_ref()
        .is_some_and(|p| p.fitted_size.is_none())
    {
        ui.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
    // Fit again when the canvas changes size too: Present hides the panels as it starts.
    let resized = tab.preview.as_mut().is_some_and(|p| {
        let changed = p.fitted_size.is_some_and(|s| s != rect.size());
        p.fitted_size = Some(rect.size());
        changed
    });
    if resized && tab.fit_node.is_none() {
        tab.fit_node = tab.preview.as_ref().map(|p| p.player.screen());
    }
    if rect.width() > 10.0
        && let Some(id) = tab.fit_node.take()
        && let Some(b) = figstudio_render::node_bounds(&tab.doc, id)
    {
        let screen = egui::Rect::from_min_max(
            Pos2::new(b.x0 as f32, b.y0 as f32),
            Pos2::new(b.x1 as f32, b.y1 as f32),
        );
        tab.view = view_for(screen, rect.size(), scaling);
    }
    tab.fit_pending = false;

    // A screen taller or wider than the window scrolls; pinch and ⌘-scroll zoom.
    if response.dragged() {
        tab.view.pan += response.drag_delta();
    }
    if response.hovered() {
        let (scroll, zoom, modifiers, pointer) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
            )
        });
        let anchor = pointer.unwrap_or(rect.center());
        if zoom != 1.0 {
            tab.view.zoom_at(origin, anchor, zoom);
        } else if modifiers.command || modifiers.ctrl {
            tab.view.zoom_at(origin, anchor, (scroll.y / 200.0).exp());
        } else if scroll != egui::Vec2::ZERO {
            // A frame that scrolls its content takes the wheel; elsewhere the view pans.
            let at = tab.view.to_page(origin, anchor);
            let delta = -scroll / tab.view.zoom;
            let event = Pointer::Scroll(
                Point::new(f64::from(at.x), f64::from(at.y)),
                figstudio_render::kurbo::Vec2::new(f64::from(delta.x), f64::from(delta.y)),
            );
            let now = ui.input(|i| i.time);
            if !tab.preview_scroll(event, now) {
                tab.view.pan += scroll;
            }
        }
    }
    // As in Figma's Present, the screen never moves off the window: along an axis where it
    // fits it stays where scaling put it, where it is larger it pans only up to its edges.
    if let Some(b) = tab
        .preview
        .as_ref()
        .and_then(|p| figstudio_render::node_bounds(&tab.doc, p.player.screen()))
    {
        let screen = egui::Rect::from_min_max(
            Pos2::new(b.x0 as f32, b.y0 as f32),
            Pos2::new(b.x1 as f32, b.y1 as f32),
        );
        tab.view.pan = keep_on_window(screen, rect.size(), tab.view);
    }
    tab.shown = tab.view;

    // As in Figma: → and ← step through the page's frames, Backspace goes back, R restarts.
    if !ui.ctx().egui_wants_keyboard_input() {
        let key = |i: &mut egui::InputState, k| i.consume_key(egui::Modifiers::NONE, k);
        let (next, previous, back, restart) = ui.input_mut(|i| {
            (
                key(i, egui::Key::ArrowRight),
                key(i, egui::Key::ArrowLeft),
                key(i, egui::Key::Backspace),
                key(i, egui::Key::R),
            )
        });
        if next || previous {
            tab.preview_step(next);
        } else if back {
            tab.preview_go(Some(figstudio_render::prototype::Step::Back));
        } else if restart {
            tab.preview_go(None);
        }
    }

    let cursor = response.hover_pos().map(|p| tab.view.to_page(origin, p));
    let active = ui.input(|i| {
        i.pointer.any_down()
            || i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerMoved(_) | egui::Event::MouseWheel { .. }
                )
            })
    });
    if active {
        let now = ui.input(|i| i.time);
        if let Some(preview) = tab.preview.as_mut() {
            preview.active_at = now;
        }
    }
    let page_point = |p: Pos2| Point::new(f64::from(p.x), f64::from(p.y));
    let now = ui.input(|i| i.time);
    // The player gets the pointer as Figma's prototypes do: hover, press, release (a click is
    // a press and a release on the same layer), and leaving the screen.
    let (pressed, released) =
        ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released()));
    let event = match cursor {
        Some(p) if pressed && response.hovered() => Some(Pointer::Down(page_point(p))),
        Some(p) if released => Some(Pointer::Up(page_point(p))),
        Some(p) => Some(Pointer::Move(page_point(p))),
        None => Some(Pointer::Leave),
    };
    let mut acted = false;
    if let Some(event) = event {
        // Hover is tracked before a press, so "while hovering" and a click on the same layer
        // both apply.
        if let Pointer::Down(p) | Pointer::Up(p) = event {
            tab.preview_pointer(Pointer::Move(p), now);
        }
        acted = tab.preview_pointer(event, now);
    }
    if let Some(due) = tab.preview_tick(now) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.0)));
    }
    let hit = |tab: &Tab| {
        let p = cursor?;
        let screen = &tab.preview.as_ref()?.screen;
        screen.hit(page_point(p)).cloned()
    };
    let hovered = hit(tab);
    if released && !acted && response.hovered() && !response.dragged() {
        let on_link = hovered.as_ref().is_some_and(|l| l.step.is_some());
        if let Some(preview) = tab.preview.as_mut().filter(|_| !on_link) {
            preview.missed = Some(now);
        }
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, radius, to_color32(tab.background()));
    let view = tab.view;
    // As in Figma's prototypes, a screen is cut at its frame even when it does not clip its
    // content.
    let screen = tab.preview.as_ref().map(|p| p.player.screen());
    let frame = screen
        .and_then(|id| figstudio_render::node_bounds(&tab.doc, id))
        .map(|b| {
            egui::Rect::from_min_max(
                view.to_screen(origin, Pos2::new(b.x0 as f32, b.y0 as f32)),
                view.to_screen(origin, Pos2::new(b.x1 as f32, b.y1 as f32)),
            )
        });
    let tiles_painter = frame.map_or_else(
        || painter.clone(),
        |f| painter.with_clip_rect(f.intersect(rect)),
    );
    tiles.show(
        &tiles_painter,
        (rect, radius),
        tab,
        view,
        workers,
        ui.pixels_per_point(),
    );

    // The layer's own shape, so a rotated link is outlined as it is drawn.
    let to_screen = |area: &figstudio_render::prototype::Area| {
        area.corners()
            .map(|p| {
                tab.view
                    .to_screen(origin, Pos2::new(p.x as f32, p.y as f32))
            })
            .to_vec()
    };
    let flash = tab
        .preview
        .as_ref()
        .and_then(|p| p.missed)
        .map_or(0.0, |at| (1.0 - (now - at) / FLASH).max(0.0) as f32);
    if flash > 0.0 {
        ui.ctx().request_repaint();
        if let Some(preview) = &tab.preview {
            for link in preview.screen.links.iter().filter(|l| l.step.is_some()) {
                painter.add(egui::Shape::convex_polygon(
                    to_screen(&link.area),
                    theme::ACCENT.gamma_multiply(0.25 * flash),
                    Stroke::NONE,
                ));
            }
        }
    }
    // As in Figma, a link shows only as the pointing hand: no outline over what it hovers.
    let playable = hovered.as_ref().is_some_and(|l| l.step.is_some());
    if response.hovered() {
        ui.ctx().set_cursor_icon(if playable {
            egui::CursorIcon::PointingHand
        } else if response.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Default
        });
    }
    cursor
}
