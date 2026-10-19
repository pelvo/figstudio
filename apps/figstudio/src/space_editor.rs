//! Arc-style space popovers, opened from a space dot's menu, the Space menu or the command
//! bar: the icon picker, renaming, and the theme colour picker (a colour pad with a handle,
//! the presets, and the appearance). Changes show at once; closing the popover (a click
//! outside, Escape, Enter when renaming) saves them for the file (`spaces::Store`).

use egui::{
    Align2, Color32, CornerRadius, Id, Margin, Mesh, Rect, RichText, Sense, Shape, Stroke,
    WidgetInfo, WidgetType, pos2, vec2,
};

use crate::anim::{self, Spring};
use crate::chrome::{card_shadow, paint_badge};
use crate::icons::{self, feedback};
use crate::menu::{Action, Appearance};
use crate::spaces::{self, Style, Symbol};
use crate::theme::{self, Colour};
use crate::{App, tab::Tab};

/// Which popover is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Popover {
    Icon,
    Rename,
    Colour,
}

/// An open space popover.
#[derive(Debug)]
pub struct SpaceEditor {
    /// The tab (`Tab::id`) whose space this is.
    pub tab: u64,
    pub kind: Popover,
    /// The name being typed (Rename).
    name: String,
    /// The emoji being typed (Icon).
    emoji: String,
    /// The style when the popover opened: Escape while renaming goes back to it.
    before: Style,
    /// Give the text field focus on the next frame.
    focus: bool,
    /// The frame it opened in: that frame's click does not close it.
    opened: u64,
}

impl SpaceEditor {
    pub fn new(tab: &Tab, kind: Popover, ctx: &egui::Context) -> Self {
        anim::reset(ctx, Id::new(("space-popover", kind)), 0.0);
        Self {
            tab: tab.id,
            kind,
            name: tab.style.name.clone().unwrap_or_default(),
            emoji: match &tab.style.symbol {
                Some(Symbol::Emoji(e)) => e.clone(),
                _ => String::new(),
            },
            before: tab.style.clone(),
            focus: true,
            opened: ctx.cumulative_frame_nr(),
        }
    }
}

/// How a popover ends this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Close {
    Keep,
    Save,
    Revert,
}

/// A round colour swatch; `selected` gets a ring.
fn swatch(ui: &mut egui::Ui, fill: Color32, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    let p = theme::of(ui);
    let (hover, press) = feedback(ui, &response);
    let c = rect.center();
    ui.painter()
        .circle_filled(c, 8.5 + hover * 1.5 - press * 1.5, fill);
    if selected {
        ui.painter()
            .circle_stroke(c, 12.0, Stroke::new(2.0, p.text));
    }
    response.on_hover_text(label)
}

/// A square button showing an emoji; `selected` gets a filled background.
fn emoji_button(ui: &mut egui::Ui, emoji: &str, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    let p = theme::of(ui);
    let (hover, press) = feedback(ui, &response);
    let r = rect.shrink(press * 1.5);
    let fill = if selected {
        p.selected
    } else {
        p.hover.gamma_multiply(hover * 1.5)
    };
    ui.painter().rect_filled(r, CornerRadius::same(8), fill);
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        emoji,
        egui::FontId::proportional(17.0),
        p.text,
    );
    response
}

/// A small pill button for a choice (Auto, System / Light / Dark).
fn chip(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let p = theme::of(ui);
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), p.text);
    let size = vec2(galley.size().x + 20.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    let (hover, press) = feedback(ui, &response);
    let r = rect.shrink(press);
    let fill = if selected {
        p.selected
    } else {
        p.hover.gamma_multiply(1.0 + hover)
    };
    ui.painter().rect_filled(r, CornerRadius::same(12), fill);
    let at = r.center() - galley.size() / 2.0;
    ui.painter().galley(at, galley, p.text);
    response
}

/// The colour at `(u, v)` of the colour pad: hue across, from vivid at the top to soft at
/// the bottom.
fn pad_colour(u: f32, v: f32) -> Color32 {
    egui::ecolor::Hsva::new(u.clamp(0.0, 1.0), 1.0 - 0.8 * v.clamp(0.0, 1.0), 0.95, 1.0).into()
}

/// Where `c` sits on the colour pad (the inverse of [`pad_colour`], near enough).
fn pad_place(c: Color32) -> (f32, f32) {
    let hsva = egui::ecolor::Hsva::from(c);
    (hsva.h, ((1.0 - hsva.s) / 0.8).clamp(0.0, 1.0))
}

/// The colour pad: drag the handle to pick any colour. Returns the colour picked this frame.
fn colour_pad(ui: &mut egui::Ui, current: Color32) -> Option<Color32> {
    let (rect, response) = ui.allocate_exact_size(vec2(276.0, 132.0), Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Slider, true, "Colour pad"));
    let p = theme::of(ui);
    const COLS: u32 = 24;
    const ROWS: u32 = 6;
    let mut mesh = Mesh::default();
    for row in 0..=ROWS {
        for col in 0..=COLS {
            let (u, v) = (col as f32 / COLS as f32, row as f32 / ROWS as f32);
            mesh.colored_vertex(rect.lerp_inside(vec2(u, v)), pad_colour(u, v));
        }
    }
    let at = |row: u32, col: u32| row * (COLS + 1) + col;
    for row in 0..ROWS {
        for col in 0..COLS {
            mesh.add_triangle(at(row, col), at(row, col + 1), at(row + 1, col));
            mesh.add_triangle(at(row, col + 1), at(row + 1, col + 1), at(row + 1, col));
        }
    }
    let painter = ui.painter();
    painter.add(Shape::mesh(mesh));
    // Round the pad's corners with the popover's own fill.
    painter.rect_stroke(
        rect.expand(5.0),
        CornerRadius::same(15),
        Stroke::new(10.0, ui.visuals().window_fill),
        egui::StrokeKind::Inside,
    );
    let picked = response
        .interact_pointer_pos()
        .filter(|_| response.is_pointer_button_down_on() || response.clicked())
        .map(|pos| {
            let u = (pos.x - rect.left()) / rect.width();
            let v = (pos.y - rect.top()) / rect.height();
            pad_colour(u, v)
        });
    let (u, v) = pad_place(picked.unwrap_or(current));
    let ctx = ui.ctx();
    let x = anim::spring(ctx, response.id.with("x"), u, Spring::SMOOTH);
    let y = anim::spring(ctx, response.id.with("y"), v, Spring::SMOOTH);
    let handle = rect.lerp_inside(vec2(x, y));
    let grab = if response.is_pointer_button_down_on() {
        1.2
    } else {
        1.0
    };
    let painter = ui.painter();
    painter.add(card_shadow(&p, 8).as_shape(
        Rect::from_center_size(handle, vec2(18.0, 18.0) * grab),
        CornerRadius::same(9),
    ));
    painter.circle_filled(handle, 9.0 * grab, Color32::WHITE);
    painter.circle_filled(handle, 6.5 * grab, picked.unwrap_or(current));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    picked
}

impl App {
    /// Saves and closes an open space popover: when it closes, when another one opens, when
    /// its file closes and when the app quits, so a change is never only in memory.
    pub(crate) fn commit_space_editor(&mut self) {
        let Some(editor) = self.space_editing.take() else {
            return;
        };
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == editor.tab) {
            tab.style = std::mem::take(&mut tab.style).tidy();
            let title = tab.title();
            self.spaces
                .remember(crate::spaces::FileId::of(tab, &title), &tab.style);
        }
    }

    /// Shows the open space popover; saves the style when it closes.
    pub(crate) fn space_popover(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.space_editing else {
            return;
        };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == editor.tab) else {
            self.space_editing = None;
            return;
        };
        let p = theme::palette(ctx);
        let dot = self
            .space_dots
            .iter()
            .find(|(id, _)| *id == editor.tab)
            .map(|(_, r)| *r)
            .filter(|_| self.show_left);
        // Above the space's dot at the bottom of the sidebar, else at the canvas's top left.
        let (pivot, at) = match dot {
            Some(dot) => (Align2::LEFT_BOTTOM, pos2(dot.left() - 6.0, dot.top() - 8.0)),
            None => (
                Align2::LEFT_TOP,
                self.card_rect
                    .map_or(pos2(16.0, 52.0), |c| c.min + vec2(16.0, 16.0)),
            ),
        };
        let kind = editor.kind;
        let pop = anim::spring_from(
            ctx,
            Id::new(("space-popover", kind)),
            1.0,
            Some(0.0),
            Spring::BOUNCY,
        );
        let mut close = Close::Keep;
        let mut appearance = None;
        let area = egui::Area::new(Id::new("space-popover"))
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(at)
            .constrain(true)
            .show(ctx, |ui| {
                ui.multiply_opacity(pop.clamp(0.0, 1.0));
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .corner_radius(CornerRadius::same(14))
                    .stroke(Stroke::new(1.0, p.card_stroke))
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 36,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 140 } else { 48 }),
                    })
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| match kind {
                        Popover::Icon => close = icon_picker(ui, editor, tab),
                        Popover::Rename => close = rename(ui, editor, tab),
                        Popover::Colour => {
                            appearance = colour_picker(ui, tab, self.appearance);
                        }
                    });
            });
        let layer = area.response.layer_id;
        ctx.move_to_top(layer);
        let corner = if dot.is_some() {
            area.response.rect.left_bottom()
        } else {
            area.response.rect.left_top()
        };
        let scale = 0.92 + 0.08 * pop;
        ctx.set_transform_layer(
            layer,
            egui::emath::TSTransform::new(corner.to_vec2() * (1.0 - scale), scale),
        );

        let fresh = ctx.cumulative_frame_nr() <= editor.opened + 1;
        let pressed_outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx
                .pointer_interact_pos()
                .is_some_and(|pos| !area.response.rect.contains(pos));
        if close == Close::Keep && !fresh && pressed_outside {
            close = Close::Save;
        }
        if close == Close::Keep
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            close = if kind == Popover::Rename {
                Close::Revert
            } else {
                Close::Save
            };
        }
        if close == Close::Revert {
            tab.style = editor.before.clone();
        }
        if close != Close::Keep {
            self.commit_space_editor();
        }
        if let Some(a) = appearance {
            self.apply(Action::Appearance(a), ctx);
        }
    }
}

fn title(ui: &mut egui::Ui, text: &str) {
    let p = theme::of(ui);
    ui.label(
        RichText::new(text)
            .font(theme::semibold(12.0))
            .color(p.text_faint),
    );
    ui.add_space(2.0);
}

/// Emoji, icons, none, or a typed emoji. Picking one closes the popover.
fn icon_picker(ui: &mut egui::Ui, editor: &mut SpaceEditor, tab: &mut Tab) -> Close {
    let mut close = Close::Keep;
    ui.set_width(8.0 * 32.0);
    title(ui, "Space icon");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for e in spaces::EMOJI {
            let on = tab.style.symbol == Some(Symbol::Emoji(e.into()));
            if emoji_button(ui, e, e, on).clicked() {
                tab.style.symbol = Some(Symbol::Emoji(e.into()));
                close = Close::Save;
            }
        }
        for (name, icon) in spaces::ICONS {
            let on = tab.style.symbol == Some(Symbol::Icon(name.into()));
            if icons::sized_button(ui, icon, name, on, 30.0).clicked() {
                tab.style.symbol = Some(Symbol::Icon(name.into()));
                close = Close::Save;
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let typed = ui.add(
            egui::TextEdit::singleline(&mut editor.emoji)
                .hint_text(format!("Type an emoji {}", spaces::EMOJI_HINT))
                .desired_width(150.0),
        );
        if typed.changed() {
            let e = editor.emoji.trim();
            tab.style.symbol = (!e.is_empty()).then(|| Symbol::Emoji(e.into()));
            // What the badge will show: the first emoji typed.
            tab.style = std::mem::take(&mut tab.style).tidy();
            if let Some(Symbol::Emoji(e)) = &tab.style.symbol {
                editor.emoji.clone_from(e);
            }
        }
        if typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            close = Close::Save;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if chip(ui, "No icon", tab.style.symbol.is_none()).clicked() {
                tab.style.symbol = None;
                close = Close::Save;
            }
        });
    });
    close
}

/// The name, typed in place; Enter keeps it, Escape goes back.
fn rename(ui: &mut egui::Ui, editor: &mut SpaceEditor, tab: &mut Tab) -> Close {
    let mut close = Close::Keep;
    ui.set_width(240.0);
    title(ui, "Space name");
    ui.horizontal(|ui| {
        let tint = theme::space_tint(tab.colour(), ui.visuals().dark_mode);
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        let p = theme::of(ui);
        paint_badge(
            ui.painter(),
            r,
            tab.style.symbol.as_ref(),
            tint.accent,
            p.text,
        );
        let field = ui.add(
            egui::TextEdit::singleline(&mut editor.name)
                .hint_text(tab.title())
                .font(egui::FontId::proportional(15.0))
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut editor.focus) {
            field.request_focus();
        }
        if field.changed() {
            tab.style.name = Some(editor.name.clone());
        }
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            close = Close::Save;
        }
    });
    close
}

/// The theme colour: the pad, the presets and Auto, and the appearance. Returns an
/// appearance picked this frame.
fn colour_picker(ui: &mut egui::Ui, tab: &mut Tab, appearance: Appearance) -> Option<Appearance> {
    let dark = ui.visuals().dark_mode;
    let mut picked = None;
    ui.set_width(276.0);
    title(ui, "Theme colour");
    let current = theme::space_tint(tab.colour(), dark).accent;
    let shown = match tab.style.colour {
        Some(Colour::Custom([r, g, b])) => Color32::from_rgb(r, g, b),
        _ => current,
    };
    if let Some(c) = colour_pad(ui, shown) {
        tab.style.colour = Some(Colour::Custom([c.r(), c.g(), c.b()]));
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 4.0);
        for (i, name) in theme::TINT_NAMES.iter().enumerate() {
            let on = tab.style.colour == Some(Colour::Preset(i));
            if swatch(ui, theme::tint(i, dark).accent, name, on).clicked() {
                tab.style.colour = Some(Colour::Preset(i));
            }
        }
        ui.add_space(4.0);
        if chip(ui, "Auto", tab.style.colour.is_none())
            .on_hover_text("The colour picked from the file name")
            .clicked()
        {
            tab.style.colour = None;
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (label, a) in [
            ("System", Appearance::System),
            ("Light", Appearance::Light),
            ("Dark", Appearance::Dark),
        ] {
            if chip(ui, label, appearance == a).clicked() {
                picked = Some(a);
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_pad_finds_its_own_colours_again() {
        for (u, v) in [(0.0, 0.0), (0.3, 0.5), (0.62, 0.9), (0.95, 0.2)] {
            let (pu, pv) = pad_place(pad_colour(u, v));
            assert!(
                (pu - u).abs() < 0.02 && (pv - v).abs() < 0.03,
                "{u},{v} -> {pu},{pv}"
            );
        }
    }
}
