//! The space editor (File → Edit space…, the sidebar's space header, a file dot's menu):
//! name, emoji or icon, and colour of the active file's space. Changes show at once; closing
//! the editor saves them for the file (`spaces::Store`).

use egui::{Color32, CornerRadius, Id, RichText, Sense, Stroke, WidgetInfo, WidgetType, vec2};

use crate::App;
use crate::chrome::paint_badge;
use crate::icons::{self, feedback};
use crate::spaces::{self, Style, Symbol};
use crate::theme::{self, Colour};

/// What the editor is editing, and its text fields.
#[derive(Debug)]
pub struct SpaceEditor {
    /// The tab (`Tab::id`) whose space this is.
    tab: u64,
    name: String,
    emoji: String,
    custom: [u8; 3],
}

impl SpaceEditor {
    pub fn new(tab: &crate::tab::Tab) -> Self {
        let p = theme::tint(tab.tint, false);
        Self {
            tab: tab.id,
            name: tab.style.name.clone().unwrap_or_default(),
            emoji: match &tab.style.symbol {
                Some(Symbol::Emoji(e)) => e.clone(),
                _ => String::new(),
            },
            custom: match tab.style.colour {
                Some(Colour::Custom(rgb)) => rgb,
                _ => [p.accent.r(), p.accent.g(), p.accent.b()],
            },
        }
    }
}

/// A round colour swatch; `selected` gets a ring.
fn swatch(ui: &mut egui::Ui, fill: Color32, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    let p = theme::of(ui);
    let (hover, press) = feedback(ui, &response);
    let c = rect.center();
    ui.painter()
        .circle_filled(c, 9.0 + hover * 1.5 - press * 1.5, fill);
    if selected {
        ui.painter()
            .circle_stroke(c, 12.5, Stroke::new(2.0, p.text));
    }
    response.on_hover_text(label)
}

/// The custom colour swatch: the picked colour inside a rainbow ring. No tooltip, so the
/// picker popup can open from it.
fn rainbow_swatch(ui: &mut egui::Ui, fill: Color32, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, "Custom colour"));
    let p = theme::of(ui);
    let (hover, press) = feedback(ui, &response);
    let c = rect.center();
    let ring = 10.0 + hover * 1.5 - press * 1.5;
    let segments = 24;
    for i in 0..segments {
        let hue = i as f32 / segments as f32;
        let colour: Color32 = egui::ecolor::Hsva::new(hue, 0.75, 0.95, 1.0).into();
        let a = |t: f32| std::f32::consts::TAU * (i as f32 + t) / segments as f32;
        let at = |t: f32| c + vec2(a(t).cos(), a(t).sin()) * ring;
        ui.painter()
            .line_segment([at(0.0), at(1.05)], Stroke::new(3.0, colour));
    }
    ui.painter().circle_filled(c, ring - 3.5, fill);
    if selected {
        ui.painter()
            .circle_stroke(c, 13.5, Stroke::new(2.0, p.text));
    }
    response
}

/// A square button showing an emoji; `selected` gets a filled background.
fn emoji_button(ui: &mut egui::Ui, emoji: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, emoji));
    let p = theme::of(ui);
    let (hover, press) = feedback(ui, &response);
    let r = rect.shrink(press * 1.5);
    let fill = if selected {
        p.selected
    } else {
        p.hover.gamma_multiply(hover)
    };
    ui.painter().rect_filled(r, CornerRadius::same(8), fill);
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        emoji,
        egui::FontId::proportional(17.0),
        p.text,
    );
    response
}

impl App {
    /// Shows the space editor while one is open; saves the style when it closes.
    pub(crate) fn space_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.space_editing else {
            return;
        };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == editor.tab) else {
            self.space_editing = None;
            return;
        };
        let p = theme::palette(ctx);
        let mut open = true;
        let mut done = false;
        let at = self
            .card_rect
            .map_or(egui::pos2(280.0, 60.0), |c| c.min + vec2(16.0, 16.0));
        egui::Window::new("Edit space")
            .id(Id::new("space-editor"))
            .collapsible(false)
            .resizable(false)
            .default_pos(at)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(316.0);
                let mut style = tab.style.clone();
                let tint = theme::space_tint(tab.colour(), p.dark);
                // Preview: the badge and the name as the sidebar shows them.
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                    paint_badge(ui.painter(), r, style.symbol.as_ref(), tint.accent, p.text);
                    ui.label(RichText::new(tab.space_name()).font(theme::semibold(16.0)));
                });
                ui.add_space(8.0);

                ui.label(
                    RichText::new("Name")
                        .font(theme::semibold(11.5))
                        .color(p.text_faint),
                );
                let typed = ui.add(
                    egui::TextEdit::singleline(&mut editor.name)
                        .hint_text(tab.title())
                        .desired_width(f32::INFINITY),
                );
                if typed.changed() {
                    style.name = Some(editor.name.clone());
                }
                ui.add_space(8.0);

                ui.label(
                    RichText::new("Icon")
                        .font(theme::semibold(11.5))
                        .color(p.text_faint),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    if emoji_button(ui, "·", style.symbol.is_none())
                        .on_hover_text("No icon: a plain dot")
                        .clicked()
                    {
                        style.symbol = None;
                        editor.emoji.clear();
                    }
                    for e in spaces::EMOJI {
                        let on = style.symbol == Some(Symbol::Emoji(e.into()));
                        if emoji_button(ui, e, on).clicked() {
                            style.symbol = Some(Symbol::Emoji(e.into()));
                            editor.emoji = e.into();
                        }
                    }
                    for (name, icon) in spaces::ICONS {
                        let on = style.symbol == Some(Symbol::Icon(name.into()));
                        if icons::sized_button(ui, icon, name, on, 30.0).clicked() {
                            style.symbol = Some(Symbol::Icon(name.into()));
                            editor.emoji.clear();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Or type one").color(p.text_dim));
                    let typed = ui.add(
                        egui::TextEdit::singleline(&mut editor.emoji)
                            .hint_text(spaces::EMOJI_HINT)
                            .desired_width(64.0),
                    );
                    if typed.changed() {
                        let e = editor.emoji.trim();
                        style.symbol = (!e.is_empty()).then(|| Symbol::Emoji(e.into()));
                    }
                });
                ui.add_space(8.0);

                ui.label(
                    RichText::new("Colour")
                        .font(theme::semibold(11.5))
                        .color(p.text_faint),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    let auto = style.colour.is_none();
                    for (i, name) in theme::TINT_NAMES.iter().enumerate() {
                        let fill = theme::tint(i, p.dark).accent;
                        let on = style.colour == Some(Colour::Preset(i)) || (auto && tab.tint == i);
                        if swatch(ui, fill, name, on).clicked() {
                            style.colour = Some(Colour::Preset(i));
                        }
                    }
                    let custom = matches!(style.colour, Some(Colour::Custom(_)));
                    let [r, g, b] = editor.custom;
                    let current = Color32::from_rgb(r, g, b);
                    let button = rainbow_swatch(ui, current, custom);
                    if button.clicked() && !custom {
                        style.colour = Some(Colour::Custom(editor.custom));
                    }
                    egui::Popup::menu(&button).show(|ui| {
                        let mut c = current;
                        let alpha = egui::color_picker::Alpha::Opaque;
                        if egui::color_picker::color_picker_color32(ui, &mut c, alpha) {
                            editor.custom = [c.r(), c.g(), c.b()];
                            style.colour = Some(Colour::Custom(editor.custom));
                        }
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Reset")
                        .on_hover_text("The file name, a plain dot and the colour picked for it")
                        .clicked()
                    {
                        style = Style::default();
                        editor.name.clear();
                        editor.emoji.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        done = ui.button("Done").clicked();
                    });
                });
                tab.style = style;
            });
        if !open || done {
            tab.style = std::mem::take(&mut tab.style).tidy();
            self.spaces
                .remember(tab.doc_key.as_deref(), &tab.title(), &tab.style);
            self.space_editing = None;
        }
    }
}
