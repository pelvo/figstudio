//! The window chrome in the Arc style: a tinted gradient behind everything, a sidebar sitting
//! on it (window controls, the command field, pages, layers, the file switcher), the canvas
//! as a rounded card with floating tool and status pills, and toasts.

use egui::epaint::{Shadow, Vertex};
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, Galley, Id, Layout, Margin, Mesh, Pos2, Rect, Response,
    RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};
use std::sync::Arc;

use crate::anim::{self, Spring};
use crate::canvas::{self, Tool};
use crate::icons::{self, Icon};
use crate::menu::Action;
use crate::theme::{self, CARD_RADIUS, GAP, Palette, Tint};
use crate::{APP_NAME, App, panels};

/// How far the sidebar's content slides when switching files.
const SLIDE: f32 = 36.0;
/// Room for the macOS traffic lights in the frameless window.
const TRAFFIC_LIGHTS: f32 = 70.0;
/// Height of the file switcher at the bottom of the sidebar.
const FOOTER: f32 = 40.0;
/// Space kept for each side of the Pages/Layers split, in points.
pub(crate) const MIN_PAGES: f32 = 60.0;
pub(crate) const MIN_LAYERS: f32 = 120.0;

/// Height of the page list for `share` of `avail`, keeping both sides usable.
pub(crate) fn split_height(avail: f32, share: f32) -> f32 {
    let max = (avail - MIN_LAYERS).max(MIN_PAGES);
    (avail * share).clamp(MIN_PAGES, max)
}

/// `text` on one line, cut with "…" to fit `width`.
pub(crate) fn elided(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping {
        max_width: width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// The soft drop shadow under cards and pills.
pub(crate) fn card_shadow(p: &Palette, blur: u8) -> Shadow {
    Shadow {
        offset: [0, 2],
        blur,
        spread: 0,
        color: if p.dark {
            Color32::from_black_alpha(90)
        } else {
            Color32::from_black_alpha(24)
        },
    }
}

/// Paints the window gradient for `tint` over `rect`: `top` at the top left, `bottom` at the
/// bottom right, a soft `glow` near the top. A grid of vertices keeps the glow round.
pub(crate) fn paint_gradient(painter: &egui::Painter, rect: Rect, tint: Tint) {
    const COLS: u32 = 12;
    const ROWS: u32 = 8;
    let glow_at = pos2(0.25, 0.0);
    let mut mesh = Mesh::default();
    for row in 0..=ROWS {
        for col in 0..=COLS {
            let u = col as f32 / COLS as f32;
            let v = row as f32 / ROWS as f32;
            let diagonal = ((u + v) / 2.0).clamp(0.0, 1.0);
            let base = tint.top.lerp_to_gamma(tint.bottom, diagonal);
            let d = vec2((u - glow_at.x) * rect.aspect_ratio(), v - glow_at.y).length();
            let glow = (1.0 - d / 0.9).clamp(0.0, 1.0).powi(2) * 0.8;
            mesh.vertices.push(Vertex {
                pos: rect.lerp_inside(vec2(u, v)),
                uv: egui::epaint::WHITE_UV,
                color: base.lerp_to_gamma(tint.glow, glow),
            });
        }
    }
    let at = |row: u32, col: u32| row * (COLS + 1) + col;
    for row in 0..ROWS {
        for col in 0..COLS {
            mesh.add_triangle(at(row, col), at(row, col + 1), at(row + 1, col));
            mesh.add_triangle(at(row, col + 1), at(row + 1, col + 1), at(row + 1, col));
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A rounded rectangle filled with a gradient between its corner colours (top left, top
/// right, bottom right, bottom left).
pub(crate) fn rounded_gradient(rect: Rect, radius: f32, colours: [Color32; 4]) -> Mesh {
    let colour_at = |p: Pos2| {
        let u = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let v = ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let top = colours[0].lerp_to_gamma(colours[1], u);
        let bottom = colours[3].lerp_to_gamma(colours[2], u);
        top.lerp_to_gamma(bottom, v)
    };
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let centres = [
        (pos2(rect.right() - r, rect.top() + r), -90.0_f32),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.center(), colour_at(rect.center()));
    for (centre, start) in centres {
        for step in 0..=8 {
            let a = (start + step as f32 * 90.0 / 8.0).to_radians();
            let p = centre + vec2(a.cos(), a.sin()) * r;
            mesh.colored_vertex(p, colour_at(p));
        }
    }
    let n = mesh.vertices.len() as u32 - 1;
    for i in 1..=n {
        mesh.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    mesh
}

/// A small section title in the sidebar ("Pages", "Layers") with an optional count.
fn section_label(ui: &mut Ui, title: &str, count: Option<usize>) {
    let p = theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let painter = ui.painter();
    painter.text(
        pos2(rect.left() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::semibold(11.0),
        p.text_faint,
    );
    if let Some(n) = count {
        painter.text(
            pos2(rect.right() - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            n.to_string(),
            egui::FontId::proportional(11.0),
            p.text_faint,
        );
    }
}

/// The draggable line between Pages and Layers. Dragging moves the split.
pub(crate) fn split_handle(ui: &mut Ui, share: &mut f32) {
    let p = theme::of(ui);
    let total = ui.max_rect().height().max(1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if response.dragged() {
        *share = (*share + response.drag_delta().y / total).clamp(0.05, 0.95);
    }
    let lit = ui.ctx().animate_bool_with_time(
        response.id.with("lit"),
        response.hovered() || response.dragged(),
        0.12,
    );
    let color = p.separator.lerp_to_gamma(theme::ACCENT, lit);
    ui.painter().hline(
        (rect.left() + 6.0)..=(rect.right() - 6.0),
        rect.center().y,
        Stroke::new(1.0 + lit, color),
    );
}

/// A rounded field with a magnifier and a clear button: the layer filter.
fn filter_field(ui: &mut Ui, text: &mut String) {
    let p = theme::of(ui);
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 24.0), Sense::hover());
                let icon = Rect::from_center_size(r.center(), vec2(13.0, 13.0));
                icons::paint(ui.painter(), Icon::Search, icon, p.text_faint);
                let clear = !text.is_empty();
                let width = ui.available_width() - if clear { 22.0 } else { 0.0 };
                ui.add_sized(
                    vec2(width, 24.0),
                    egui::TextEdit::singleline(text)
                        .hint_text("Filter layers")
                        .vertical_align(Align::Center)
                        .frame(egui::Frame::NONE)
                        .margin(Margin::symmetric(2, 0)),
                );
                if clear
                    && icons::sized_button(ui, Icon::Close, "Clear filter", false, 20.0).clicked()
                {
                    text.clear();
                }
            });
        });
}

/// A clickable pill drawn by hand inside the tool pill.
fn pill_button(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    (icon, label, tip): (Option<Icon>, &str, &str),
    active: bool,
    accent: Color32,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, active, label));
    let p = theme::of(ui);
    let (hover, press) = icons::feedback(ui, &response);
    let r = rect.shrink(1.0 + press * 1.5);
    if !active {
        ui.painter().rect_filled(
            r,
            CornerRadius::same(13),
            p.hover.gamma_multiply(hover * 1.6),
        );
    }
    let color = if active {
        accent
    } else {
        p.text_dim.lerp_to_gamma(p.text, hover)
    };
    match icon {
        Some(icon) => {
            let size = 16.0 * r.width() / rect.width();
            icons::paint(
                ui.painter(),
                icon,
                Rect::from_center_size(r.center(), vec2(size, size)),
                color,
            );
        }
        None => {
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                label,
                theme::semibold(12.0),
                color,
            );
        }
    }
    response.on_hover_text(tip)
}

impl App {
    /// Whether the frameless macOS window shows its traffic lights over our chrome.
    fn traffic_lights(&self, ctx: &egui::Context) -> bool {
        self.frameless && !ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
    }

    /// In the frameless window, empty chrome moves the window; a double click zooms it.
    fn drag_window(&self, ui: &Ui, response: &Response) {
        if !self.frameless {
            return;
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if response.double_clicked() {
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
    }

    /// The gradient of the active file, crossfading when the file changes.
    pub(crate) fn tint_now(&self, ctx: &egui::Context) -> Tint {
        let dark = theme::palette(ctx).dark;
        let index = self
            .tabs
            .get(self.active)
            .map_or(theme::HOME_TINT, |t| t.tint);
        let target = theme::tint(index, dark);
        let id = Id::new("window-tint");
        let fade = |n: &str, c: Color32| anim::fade(ctx, id.with(n), c, 0.45);
        Tint {
            top: fade("top", target.top),
            bottom: fade("bottom", target.bottom),
            glow: fade("glow", target.glow),
            accent: fade("accent", target.accent),
        }
    }

    /// Starts the sidebar slide when the active file changed since the last frame.
    pub(crate) fn note_active_file(&mut self, ctx: &egui::Context) {
        let now = self.tabs.get(self.active).map(|t| t.id);
        if now != self.shown_file {
            if let (Some(before), Some(now)) = (self.shown_file, now) {
                let pos = |id: u64| self.tabs.iter().position(|t| t.id == id);
                let dir = match (pos(before), pos(now)) {
                    (Some(a), Some(b)) if a > b => -1.0,
                    _ => 1.0,
                };
                anim::reset(ctx, Id::new("space-slide"), dir * SLIDE);
            }
            self.shown_file = now;
        }
    }

    /// The window controls row at the top of the sidebar (or of the title strip).
    fn window_row(&mut self, ui: &mut Ui, sidebar: bool) {
        let peek = sidebar && !self.show_left;
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            if self.traffic_lights(ui.ctx()) && !peek {
                let (_, r) =
                    ui.allocate_exact_size(vec2(TRAFFIC_LIGHTS, 28.0), Sense::click_and_drag());
                self.drag_window(ui, &r);
            }
            let (label, show) = match (sidebar, peek) {
                (true, false) => ("Hide sidebar", false),
                (true, true) => ("Keep sidebar open", true),
                (false, _) => ("Show sidebar", true),
            };
            if icons::button(ui, Icon::Sidebar, label, false).clicked() {
                self.show_left = show;
            }
            let menu = self.show_drawn_menu();
            let rest = ui.available_width() - if menu { 30.0 } else { 0.0 };
            let (rect, r) =
                ui.allocate_exact_size(vec2(rest.max(0.0), 28.0), Sense::click_and_drag());
            self.drag_window(ui, &r);
            if !sidebar && let Some(tab) = self.tabs.get(self.active) {
                let p = theme::of(ui);
                let page = tab
                    .doc
                    .node(tab.page)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                let title = format!("{}  ·  {page}", tab.title());
                let width = rect.width() - 24.0;
                let g = elided(ui, &title, theme::semibold(12.5), p.text_dim, width);
                let at = pos2(
                    rect.center().x - g.size().x / 2.0,
                    rect.center().y - g.size().y / 2.0,
                );
                ui.painter().galley(at, g, p.text_dim);
            }
            if menu {
                self.menu_button(ui);
            }
        });
    }

    /// While the sidebar is hidden, pointing at the window's left edge slides it out over the
    /// canvas (Arc's peek); it slides away when the pointer leaves it.
    pub(crate) fn peek_sidebar(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let width = 264.0;
        let card = Rect::from_min_size(
            pos2(screen.left() + GAP, screen.top() + 44.0),
            vec2(width, (screen.height() - 44.0 - GAP).max(100.0)),
        );
        if self.show_left {
            self.peek = false;
        } else if let Some(pointer) = ctx.pointer_hover_pos() {
            let busy = ctx.any_popup_open() || ctx.input(|i| i.pointer.any_down());
            if pointer.x <= screen.left() + 6.0 && pointer.y > screen.top() + 40.0 {
                self.peek = true;
            } else if self.peek && !busy && !card.expand(24.0).contains(pointer) {
                self.peek = false;
            }
        }
        let hidden = -(width + GAP + 24.0);
        let x = anim::spring(
            ctx,
            Id::new("peek"),
            if self.peek { 0.0 } else { hidden },
            Spring::SMOOTH,
        );
        if x <= hidden + 0.5 {
            return;
        }
        let card = card.translate(vec2(x, 0.0));
        let p = theme::palette(ctx);
        let tint = self.tint_now(ctx);
        egui::Area::new(Id::new("peek-sidebar"))
            .order(egui::Order::Foreground)
            .fixed_pos(card.min)
            .show(ctx, |ui| {
                let radius = CornerRadius::same(CARD_RADIUS);
                let painter = ui.painter();
                painter.add(card_shadow(&p, 28).as_shape(card, radius));
                painter.add(Shape::mesh(rounded_gradient(
                    card,
                    f32::from(CARD_RADIUS),
                    [tint.glow, tint.top, tint.bottom, tint.top],
                )));
                painter.rect_stroke(
                    card,
                    radius,
                    Stroke::new(1.0, p.card_stroke),
                    StrokeKind::Inside,
                );
                let inner = card.shrink2(vec2(8.0, 8.0));
                ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    ui.set_clip_rect(card);
                    self.sidebar(ui);
                });
            });
    }

    /// The thin strip shown instead of the sidebar while it is hidden: window controls, the
    /// sidebar button and the file's title.
    pub(crate) fn title_strip(&mut self, ui: &mut Ui) {
        self.window_row(ui, false);
    }

    /// The left sidebar.
    pub(crate) fn sidebar(&mut self, ui: &mut Ui) {
        self.window_row(ui, true);
        ui.add_space(6.0);
        self.address_field(ui);
        ui.add_space(10.0);
        let max = ui.max_rect();
        let body = Rect::from_min_max(ui.cursor().min, pos2(max.right(), max.bottom() - FOOTER));
        let ctx = ui.ctx().clone();
        let slide = anim::spring(&ctx, Id::new("space-slide"), 0.0, Spring::SMOOTH);
        ui.scope_builder(
            UiBuilder::new().max_rect(body.translate(vec2(slide, 0.0))),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(body.expand2(vec2(0.0, 4.0))));
                ui.multiply_opacity(1.0 - (slide.abs() / SLIDE).min(1.0) * 0.85);
                self.sidebar_body(ui);
            },
        );
        let footer = Rect::from_min_max(pos2(max.left(), max.bottom() - FOOTER), max.max);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(footer)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| self.file_switcher(ui),
        );
    }

    /// The field at the top of the sidebar: where you are, and the way into the command bar.
    fn address_field(&mut self, ui: &mut Ui) {
        let p = theme::of(ui);
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Command bar"));
        let (hover, press) = icons::feedback(ui, &response);
        let r = rect.shrink(press);
        let painter = ui.painter();
        if !p.dark {
            painter.add(card_shadow(&p, 6).as_shape(r, CornerRadius::same(10)));
        }
        painter.rect_filled(
            r,
            CornerRadius::same(10),
            p.field.lerp_to_gamma(p.selected, hover * 0.6),
        );
        let icon = Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(14.0, 14.0));
        icons::paint(painter, Icon::Search, icon, p.text_dim);
        let shortcut = ui.ctx().format_shortcut(&crate::command::SHORTCUT);
        let hint = elided(
            ui,
            &shortcut,
            egui::FontId::proportional(11.5),
            p.text_faint,
            60.0,
        );
        let hint_at = pos2(
            r.right() - 12.0 - hint.size().x,
            r.center().y - hint.size().y / 2.0,
        );
        let (text, color) = match self.tabs.get(self.active) {
            Some(tab) => (tab.title(), p.text),
            None => ("Search or open a file".to_owned(), p.text_faint),
        };
        let width = hint_at.x - (r.left() + 34.0) - 8.0;
        let g = elided(ui, &text, egui::FontId::proportional(13.0), color, width);
        let at = pos2(r.left() + 34.0, r.center().y - g.size().y / 2.0);
        painter.galley(at, g, color);
        painter.galley(hint_at, hint, p.text_faint);
        let response = response.on_hover_text("Search commands, files, pages and layers");
        if response.clicked() {
            self.command.show(ui.ctx());
        }
    }

    fn sidebar_body(&mut self, ui: &mut Ui) {
        let p = theme::of(ui);
        let Some(tab) = self.tabs.get_mut(self.active) else {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("No file open").color(p.text_faint));
            });
            return;
        };
        // Pages label, the split handle, the Layers label and the filter field.
        let fixed = 20.0 + 12.0 + 20.0 + 32.0 + 8.0;
        let pages_height = split_height(ui.available_height() - fixed, self.pages_share);
        section_label(ui, "Pages", Some(tab.doc.pages.len()));
        egui::ScrollArea::vertical()
            .id_salt("pages")
            .auto_shrink([false, false])
            .max_height(pages_height)
            .show(ui, |ui| panels::pages(ui, tab));
        split_handle(ui, &mut self.pages_share);
        section_label(ui, "Layers", None);
        filter_field(ui, &mut self.layer_filter);
        ui.add_space(4.0);
        panels::layers(ui, tab, &self.layer_filter);
    }

    /// The bottom of the sidebar: one dot per open file in its colour, like Arc's spaces.
    fn file_switcher(&mut self, ui: &mut Ui) {
        let p = theme::of(ui);
        let ctx = ui.ctx().clone();
        let dot = 26.0;
        let count = self.tabs.len() + self.loading.len();
        let plus = 30.0;
        ui.spacing_mut().item_spacing.x = 0.0;
        let free = ui.available_width() - plus - count as f32 * dot;
        ui.add_space((free / 2.0).max(4.0));
        let (mut pick, mut close) = (None, None);
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let (rect, response) = ui.allocate_exact_size(vec2(dot, dot), Sense::click());
            let title = tab.title();
            response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, active, &title));
            let (hover, press) = icons::feedback(ui, &response);
            let grow = anim::spring(
                &ctx,
                response.id.with("grow"),
                if active { 1.0 } else { 0.7 },
                Spring::BOUNCY,
            );
            let scale = grow + hover * 0.12 - press * 0.15;
            let tint = theme::tint(tab.tint, p.dark);
            let c = rect.center();
            let painter = ui.painter();
            painter.circle_filled(c, 6.5 * scale, tint.accent);
            if active {
                painter.circle_stroke(
                    c,
                    6.5 * scale + 3.0,
                    Stroke::new(1.5, tint.accent.gamma_multiply(0.55)),
                );
            }
            let response = response.on_hover_text(&title);
            if response.clicked() {
                pick = Some(i);
            }
            if response.middle_clicked() {
                close = Some(i);
            }
            response.context_menu(|ui| {
                if ui.button("Close").clicked() {
                    close = Some(i);
                    ui.close();
                }
            });
        }
        for _ in self.loading.iter() {
            let (rect, _) = ui.allocate_exact_size(vec2(dot, dot), Sense::hover());
            egui::Spinner::new().paint_at(ui, rect.shrink(7.0));
        }
        let mut open = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(2.0);
            open = icons::button(ui, Icon::Plus, "Open file…", false).clicked();
        });
        if let Some(i) = pick {
            self.active = i;
        }
        if let Some(i) = close {
            self.close(i);
        }
        if open {
            self.open_dialog(&ctx);
        }
    }

    /// The canvas card and everything floating on it. Returns the page-space cursor.
    pub(crate) fn canvas_card(&mut self, ui: &mut Ui, space_held: bool) -> Option<Pos2> {
        let ctx = ui.ctx().clone();
        let p = theme::of(ui);
        let card = ui.max_rect().shrink(GAP);
        self.card_rect = Some(card);
        let radius = CornerRadius::same(CARD_RADIUS);
        let painter = ui.painter();
        painter.add(card_shadow(&p, 16).as_shape(card, radius));
        painter.rect_filled(card, radius, p.card);
        let mut actions = Vec::new();
        let mut cursor = None;
        let mut open_clicked = false;
        ui.scope_builder(UiBuilder::new().max_rect(card), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(card));
            match self.tabs.get_mut(self.active) {
                Some(_) if self.render_worker_lost => {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Rendering is unavailable; restart the app.")
                                .color(p.warning),
                        );
                    });
                }
                Some(tab) => {
                    cursor = canvas::show(
                        ui,
                        tab,
                        &mut self.workers,
                        &mut self.tiles,
                        self.tool,
                        space_held,
                        f32::from(CARD_RADIUS),
                    );
                }
                None => open_clicked = self.empty_state(ui, card),
            }
            ui.painter().rect_stroke(
                card,
                radius,
                Stroke::new(1.0, p.card_stroke),
                StrokeKind::Inside,
            );
            if !self.render_worker_lost && self.tabs.get(self.active).is_some() {
                let tools = self.tool_pill(ui, card, &mut actions);
                self.status_pill(ui, card, tools.left() - 8.0);
                self.gaps_chip(ui, card);
            }
        });
        for action in actions {
            self.apply(action, &ctx);
        }
        if open_clicked {
            self.open_dialog(&ctx);
        }
        cursor
    }

    /// Select / Hand / Zoom with a highlight that slides between them, and the zoom controls.
    fn tool_pill(&mut self, ui: &mut Ui, card: Rect, actions: &mut Vec<Action>) -> Rect {
        let ctx = ui.ctx().clone();
        let p = theme::of(ui);
        let accent = self.tint_now(&ctx).accent;
        let tools = [
            (Tool::Select, Icon::Select, "Select tool", "Select (V)"),
            (Tool::Hand, Icon::Hand, "Hand tool", "Hand (H)"),
            (
                Tool::Zoom,
                Icon::Zoom,
                "Zoom tool",
                "Zoom (Z), Alt to zoom out",
            ),
        ];
        let (b, pad, zoom_w, sep) = (30.0, 4.0, 52.0, 9.0);
        let full = pad * 2.0 + b * 3.0 + sep + b * 2.0 + zoom_w;
        // A narrow card keeps only the tools (the zoom stays on the keys and the menus).
        let compact = card.width() < full + 32.0;
        let width = if compact { pad * 2.0 + b * 3.0 } else { full };
        let rect = Rect::from_center_size(
            pos2(card.center().x, card.bottom() - 32.0),
            vec2(width, 38.0),
        );
        let radius = CornerRadius::same(19);
        // Catches clicks between the buttons so they do not reach the canvas.
        ui.interact(rect, Id::new("tool-pill"), Sense::click());
        let painter = ui.painter();
        painter.add(card_shadow(&p, 14).as_shape(rect, radius));
        painter.rect_filled(rect, radius, p.glass);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, p.card_stroke),
            StrokeKind::Inside,
        );
        let slot = |i: f32| {
            Rect::from_min_size(
                pos2(rect.left() + pad + i * b, rect.top() + 4.0),
                vec2(b, b),
            )
        };
        let index = tools.iter().position(|t| t.0 == self.tool).unwrap_or(0) as f32;
        let x = anim::spring(&ctx, Id::new("tool-highlight"), index * b, Spring::BOUNCY);
        let highlight = slot(0.0).translate(vec2(x, 0.0)).shrink(1.0);
        ui.painter().rect_filled(
            highlight,
            CornerRadius::same(13),
            accent.gamma_multiply(0.16),
        );
        for (i, (tool, icon, label, tip)) in tools.into_iter().enumerate() {
            let id = Id::new(("tool", label));
            let active = self.tool == tool;
            if pill_button(
                ui,
                slot(i as f32),
                id,
                (Some(icon), label, tip),
                active,
                accent,
            )
            .clicked()
            {
                actions.push(Action::SelectTool(tool));
            }
        }
        if compact {
            return rect;
        }
        let sep_x = rect.left() + pad + 3.0 * b + sep / 2.0;
        ui.painter().vline(
            sep_x,
            (rect.top() + 10.0)..=(rect.bottom() - 10.0),
            Stroke::new(1.0, p.separator),
        );
        let left = sep_x + sep / 2.0;
        let out = Rect::from_min_size(pos2(left, rect.top() + 4.0), vec2(b, b));
        let zoom = Rect::from_min_size(pos2(out.right(), rect.top() + 4.0), vec2(zoom_w, b));
        let into = Rect::from_min_size(pos2(zoom.right(), rect.top() + 4.0), vec2(b, b));
        if pill_button(
            ui,
            out,
            Id::new("zoom-out"),
            (Some(Icon::Minus), "Zoom out", "Zoom out (-)"),
            false,
            accent,
        )
        .clicked()
        {
            actions.push(Action::ZoomBy(0.8));
        }
        let percent = self
            .tabs
            .get(self.active)
            .map_or(100.0, |t| t.view.zoom * 100.0);
        let response = ui.interact(zoom, Id::new("zoom-fit"), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Zoom to fit"));
        let (hover, press) = icons::feedback(ui, &response);
        ui.painter().rect_filled(
            zoom.shrink(1.0 + press),
            CornerRadius::same(13),
            p.hover.gamma_multiply(hover * 1.6),
        );
        ui.painter().text(
            zoom.center(),
            Align2::CENTER_CENTER,
            format!("{percent:.0}%"),
            theme::semibold(12.0),
            p.text,
        );
        if response.on_hover_text("Zoom to fit (⇧1)").clicked() {
            actions.push(Action::ZoomFit);
        }
        if pill_button(
            ui,
            into,
            Id::new("zoom-in"),
            (Some(Icon::Plus), "Zoom in", "Zoom in (+)"),
            false,
            accent,
        )
        .clicked()
        {
            actions.push(Action::ZoomBy(1.25));
        }
        rect
    }

    /// What draws the canvas, the cursor and the render time, at the bottom left.
    fn status_pill(&mut self, ui: &mut Ui, card: Rect, right: f32) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let p = theme::of(ui);
        let max = Rect::from_min_max(
            pos2(card.left() + 12.0, card.bottom() - 47.0),
            pos2(right, card.bottom() - 17.0),
        );
        if max.width() < 90.0 {
            return;
        }
        let cpu_tiles = match tab.stats.gpu_fallback_tiles {
            0 => String::new(),
            n => format!(" · {n} tiles on the CPU"),
        };
        let cursor = self
            .cursor
            .map(|c| format!(" · {:.0}, {:.0}", c.x, c.y))
            .unwrap_or_default();
        let loading = match tab.stats.images_loading {
            0 => String::new(),
            n => format!(" · loading {n} images…"),
        };
        let text = format!(
            "{}{cpu_tiles}{cursor} · {} layers drawn · {:.0} ms{loading}",
            self.renderer,
            tab.stats.nodes,
            tab.render_time.as_secs_f64() * 1000.0
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(max)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                egui::Frame::new()
                    .fill(p.glass)
                    .corner_radius(CornerRadius::same(15))
                    .stroke(Stroke::new(1.0, p.card_stroke))
                    .shadow(card_shadow(&p, 10))
                    .inner_margin(Margin::symmetric(11, 5))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        self.mcp.status(ui);
                        ui.add(
                            egui::Label::new(RichText::new(text).size(11.5).color(p.text_dim))
                                .truncate(),
                        )
                        .on_hover_text("What draws the canvas (FIGSTUDIO_RENDERER=cpu|gpu). Tiles on the CPU use something the GPU path cannot draw yet.");
                    });
            },
        );
    }

    /// What this page uses that figstudio does not draw exactly, at the top right.
    fn gaps_chip(&self, ui: &mut Ui, card: Rect) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let gaps = crate::region_png::gaps(&tab.stats);
        if gaps.is_empty() {
            return;
        }
        let p = theme::of(ui);
        let max = Rect::from_min_max(
            pos2(card.center().x, card.top() + 12.0),
            pos2(card.right() - 12.0, card.top() + 42.0),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(max)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                egui::Frame::new()
                    .fill(p.glass)
                    .corner_radius(CornerRadius::same(15))
                    .stroke(Stroke::new(1.0, p.warning.gamma_multiply(0.45)))
                    .shadow(card_shadow(&p, 10))
                    .inner_margin(Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        // Right to left: the text first, then the icon on its left.
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.add(
                            egui::Label::new(RichText::new(gaps).size(11.5).color(p.warning))
                                .truncate(),
                        )
                        .on_hover_text("Parts of this page use features figstudio does not draw yet, so it may differ from Figma.");
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icons::paint(ui.painter(), Icon::Warning, r, p.warning);
                    });
            },
        );
    }

    /// Shown in the canvas card when no file is open. Returns true when "Open file…" was
    /// clicked.
    fn empty_state(&self, ui: &mut Ui, card: Rect) -> bool {
        let ctx = ui.ctx().clone();
        let p = theme::of(ui);
        let tint = self.tint_now(&ctx);
        let c = card.center() - vec2(0.0, card.height() * 0.08);
        let painter = ui.painter();
        // The mark: a rounded tile in the tint's gradient with a frame glyph.
        let tile = Rect::from_center_size(c - vec2(0.0, 92.0), vec2(64.0, 64.0));
        painter.add(card_shadow(&p, 18).as_shape(tile, CornerRadius::same(18)));
        let colours = [
            tint.glow,
            tint.accent,
            tint.accent,
            tint.accent.lerp_to_gamma(tint.glow, 0.4),
        ];
        painter.add(Shape::mesh(rounded_gradient(tile, 18.0, colours)));
        icons::paint(
            painter,
            Icon::Frame,
            Rect::from_center_size(tile.center(), vec2(30.0, 30.0)),
            Color32::WHITE,
        );
        painter.text(
            c - vec2(0.0, 30.0),
            Align2::CENTER_CENTER,
            APP_NAME,
            theme::semibold(24.0),
            p.text,
        );
        painter.text(
            c - vec2(0.0, 4.0),
            Align2::CENTER_CENTER,
            "Open a Figma .fig local copy, or drop one here",
            egui::FontId::proportional(13.5),
            p.text_dim,
        );
        let button = Rect::from_center_size(c + vec2(0.0, 36.0), vec2(150.0, 38.0));
        let response = ui.interact(button, Id::new("open-file"), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Open file…"));
        let (hover, press) = icons::feedback(ui, &response);
        let r = button
            .translate(vec2(0.0, -hover * 1.5))
            .shrink(press * 2.0);
        let painter = ui.painter();
        painter
            .add(card_shadow(&p, (8.0 + hover * 10.0) as u8).as_shape(r, CornerRadius::same(12)));
        painter.rect_filled(
            r,
            CornerRadius::same(12),
            tint.accent.lerp_to_gamma(Color32::WHITE, hover * 0.1),
        );
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            "Open file…",
            theme::semibold(14.0),
            Color32::WHITE,
        );
        let shortcut = ctx.format_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::O,
        ));
        painter.text(
            c + vec2(0.0, 76.0),
            Align2::CENTER_CENTER,
            shortcut,
            egui::FontId::proportional(11.5),
            p.text_faint,
        );
        painter.text(
            card.center_bottom() - vec2(0.0, 28.0),
            Align2::CENTER_CENTER,
            "Figma → File → Save local copy… gives you a .fig",
            egui::FontId::proportional(11.5),
            p.text_faint,
        );
        if response.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response.clicked()
    }

    /// Files dragged over the window: the card lights up in the tint.
    pub(crate) fn drop_overlay(&self, ctx: &egui::Context, hovering: bool) {
        let t = ctx.animate_bool_with_time(Id::new("drop-overlay"), hovering, 0.15);
        if t <= 0.0 {
            return;
        }
        let p = theme::palette(ctx);
        let tint = self.tint_now(ctx);
        let card = self
            .card_rect
            .unwrap_or_else(|| ctx.content_rect().shrink(GAP));
        let painter =
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("drop")));
        let radius = CornerRadius::same(CARD_RADIUS);
        painter.rect_filled(card, radius, p.card.gamma_multiply(0.82 * t));
        let pulse = (ctx.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
        painter.rect_stroke(
            card.shrink(8.0),
            CornerRadius::same(CARD_RADIUS - 4),
            Stroke::new(2.0, tint.accent.gamma_multiply(t * (0.6 + 0.4 * pulse))),
            StrokeKind::Inside,
        );
        painter.text(
            card.center() - vec2(0.0, 10.0 * (1.0 - t)),
            Align2::CENTER_CENTER,
            "Drop to open",
            theme::semibold(22.0),
            p.text.gamma_multiply(t),
        );
        if hovering {
            ctx.request_repaint();
        }
    }

    /// Toasts stack up from the bottom right of the canvas card, slide in and fade out.
    pub(crate) fn toasts(&self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        let p = theme::palette(ctx);
        let now = ctx.input(|i| i.time);
        let reduce = anim::reduced(ctx);
        let card = self
            .card_rect
            .unwrap_or_else(|| ctx.content_rect().shrink(GAP));
        let mut y = card.bottom() - 62.0;
        let mut moving = false;
        for t in self.toasts.iter().rev() {
            let id = Id::new(("toast", t.born.to_bits()));
            let age = (now - t.born) as f32;
            let enter = if reduce {
                1.0
            } else {
                (age / 0.35).clamp(0.0, 1.0)
            };
            let leave = ((t.until - now) as f32 / 0.3).clamp(0.0, 1.0);
            moving |= enter < 1.0 || leave < 1.0;
            let height = ctx.memory(|m| m.area_rect(id)).map_or(34.0, |r| r.height());
            let area = egui::Area::new(id)
                .order(egui::Order::Foreground)
                .pivot(Align2::RIGHT_BOTTOM)
                .fixed_pos(pos2(card.right() - 14.0, y))
                .interactable(false)
                .show(ctx, |ui| {
                    ui.multiply_opacity(enter.min(leave));
                    egui::Frame::new()
                        .fill(p.glass)
                        .corner_radius(CornerRadius::same(12))
                        .stroke(Stroke::new(1.0, p.card_stroke))
                        .shadow(card_shadow(&p, 16))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_max_width(380.0);
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                if t.error {
                                    icons::paint(ui.painter(), Icon::Warning, r, p.warning);
                                } else {
                                    ui.painter().circle_filled(r.center(), 4.0, theme::ACCENT);
                                }
                                let text = RichText::new(&t.text).color(if t.error {
                                    p.warning
                                } else {
                                    p.text
                                });
                                ui.add(egui::Label::new(text).wrap());
                            });
                        });
                });
            let lift = (1.0 - egui::emath::easing::back_out(enter)) * 18.0;
            ctx.set_transform_layer(
                area.response.layer_id,
                egui::emath::TSTransform::from_translation(vec2(0.0, lift)),
            );
            y -= height + 8.0;
        }
        if moving {
            ctx.request_repaint();
        }
    }
}
