//! Settings (⌘, / Ctrl+,), laid out like Arc's: the tab's name as the title, tabs with an icon
//! above their name, and below them a list on the left with a short explanation on the right.
//! General, Canvas, Shortcuts, Agents (agent access, the MCP server) and About.

use egui::{
    Align, Align2, Color32, CornerRadius, Id, Key, KeyboardShortcut, Layout, Margin, Modifiers,
    Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::anim::{self, Spring};
use crate::icons::{self, Icon};
use crate::keymap::Binding;
use crate::menu::{Action, Appearance};
use crate::{APP_NAME, App, theme};

/// The tabs of the settings window, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    General,
    Canvas,
    Shortcuts,
    Agents,
    About,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::General,
        Tab::Canvas,
        Tab::Shortcuts,
        Tab::Agents,
        Tab::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Canvas => "Canvas",
            Tab::Shortcuts => "Shortcuts",
            Tab::Agents => "Agents",
            Tab::About => "About",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Tab::General => Icon::Gear,
            Tab::Canvas => Icon::Frame,
            Tab::Shortcuts => Icon::Keyboard,
            Tab::Agents => Icon::Sparkle,
            Tab::About => Icon::Info,
        }
    }
}

/// Which renderer draws the canvas, chosen in Settings → Canvas (applied at the next start).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Renderer {
    #[default]
    Automatic,
    Gpu,
    Cpu,
}

impl Renderer {
    /// What `FIGSTUDIO_RENDERER` would say for this choice.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the browser picks its renderer from the URL")
    )]
    pub fn env_value(self) -> Option<&'static str> {
        match self {
            Renderer::Automatic => None,
            Renderer::Gpu => Some("gpu"),
            Renderer::Cpu => Some("cpu"),
        }
    }
}

/// The settings window's state.
#[derive(Debug, Default)]
pub struct Settings {
    open: bool,
    pub tab: Tab,
    /// The shortcut list's search.
    pub(crate) filter: String,
    /// It opened this frame: that click does not close it.
    fresh: bool,
    /// The shortcut waiting for its new keys (the next key press).
    recording: Option<Binding>,
    /// What the last change did to another shortcut ("moved from …").
    notice: Option<String>,
}

impl Settings {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the window, on `tab` if given.
    pub fn show(&mut self, tab: Option<Tab>, ctx: &egui::Context) {
        if !self.open {
            anim::reset(ctx, Id::new("settings-pop"), 0.0);
        }
        self.open = true;
        self.fresh = true;
        if let Some(tab) = tab {
            self.tab = tab;
        }
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.recording = None;
    }

    /// The shortcut being recorded: until a key is pressed, keys go to it, not to the app.
    pub fn recording(&self) -> Option<Binding> {
        self.recording.filter(|_| self.open)
    }
}

/// Height of the title and the tab row.
const HEADER: f32 = 96.0;

/// Things done with the pointer or fixed keys: listed, not changeable.
const FIXED: [(&str, &str); 7] = [
    ("Clear the selection", "Esc"),
    (
        "Next or previous match while typing",
        "Enter or Shift+Enter",
    ),
    ("Zoom around the pointer", "⌘ or Ctrl + scroll, or pinch"),
    ("Pan", "Scroll, Space + drag, or middle drag"),
    ("Zoom to a layer", "Double-click it"),
    ("Switch spaces", "Two-finger swipe on the sidebar"),
    ("Peek at the hidden sidebar", "Point at the left edge"),
];

/// A keycap: the keys on a light rounded box, as Arc draws them, ending at `right`.
/// `recording` outlines it in the accent and asks for keys.
fn keycap(ui: &Ui, text: &str, right: f32, y: f32, recording: bool) -> Rect {
    let p = theme::of(ui);
    let ink = Color32::from_rgb(40, 40, 46);
    let shown = if recording { "Type keys…" } else { text };
    let galley = ui
        .painter()
        .layout_no_wrap(shown.to_owned(), theme::semibold(12.0), ink);
    let w = (galley.size().x + 20.0).max(54.0);
    let cap = Rect::from_min_size(pos2(right - w, y - 11.0), vec2(w, 22.0));
    let fill = if p.dark {
        Color32::from_gray(236)
    } else {
        Color32::WHITE
    };
    ui.painter().rect_filled(cap, CornerRadius::same(6), fill);
    let edge = if recording {
        Stroke::new(2.0, theme::MENU_HIGHLIGHT)
    } else {
        Stroke::new(1.0, Color32::from_black_alpha(30))
    };
    ui.painter()
        .rect_stroke(cap, CornerRadius::same(6), edge, egui::StrokeKind::Outside);
    ui.painter()
        .galley(cap.center() - galley.size() / 2.0, galley, ink);
    cap
}

/// A switch, macOS style: the knob slides, the track fills with the accent when on.
fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, *on, label));
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let p = theme::of(ui);
    let t = anim::spring(
        ui.ctx(),
        response.id.with("knob"),
        if *on { 1.0 } else { 0.0 },
        Spring::BOUNCY,
    );
    let off = if p.dark {
        Color32::from_gray(70)
    } else {
        Color32::from_gray(210)
    };
    let track = off.lerp_to_gamma(theme::MENU_HIGHLIGHT, t.clamp(0.0, 1.0));
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), track);
    let x = rect.left() + 10.0 + t * (rect.width() - 20.0);
    ui.painter().add(crate::chrome::card_shadow(&p, 4).as_shape(
        Rect::from_center_size(pos2(x, rect.center().y), vec2(16.0, 16.0)),
        CornerRadius::same(8),
    ));
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), 8.0, Color32::WHITE);
    response
}

/// A row of choices; the chosen one is lit. Returns the one clicked.
fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    current: T,
    choices: &[(&str, T)],
) -> Option<T> {
    let p = theme::of(ui);
    let mut picked = None;
    let width = 56.0;
    let (rect, _) = ui.allocate_exact_size(
        vec2(width * choices.len() as f32 + 4.0, 26.0),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), p.well);
    let index = choices.iter().position(|(_, v)| *v == current).unwrap_or(0) as f32;
    let x = anim::spring(
        ui.ctx(),
        Id::new((id, "lit")),
        index * width,
        Spring::SMOOTH,
    );
    let lit = Rect::from_min_size(
        pos2(rect.left() + 2.0 + x, rect.top() + 2.0),
        vec2(width, 22.0),
    );
    let fill = if p.dark {
        Color32::from_gray(78)
    } else {
        Color32::WHITE
    };
    ui.painter()
        .add(crate::chrome::card_shadow(&p, 4).as_shape(lit, CornerRadius::same(6)));
    ui.painter().rect_filled(lit, CornerRadius::same(6), fill);
    for (i, (label, value)) in choices.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.left() + 2.0 + i as f32 * width, rect.top() + 2.0),
            vec2(width, 22.0),
        );
        let response = ui.interact(r, Id::new((id, i)), Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::Button, true, *value == current, *label)
        });
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            egui::FontId::proportional(12.5),
            p.text,
        );
        if response.clicked() {
            picked = Some(*value);
        }
    }
    picked
}

/// A labelled row in a settings group: the label (and a note under it) left, `control` right.
fn setting(ui: &mut Ui, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
    let p = theme::of(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(if note.is_empty() { 34.0 } else { 46.0 });
        ui.vertical(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(label).size(13.0));
            if !note.is_empty() {
                ui.label(RichText::new(note).size(11.5).color(p.text_faint));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

/// A rounded group of settings rows.
fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    let p = theme::of(ui);
    if !title.is_empty() {
        ui.label(
            RichText::new(title)
                .font(theme::semibold(12.0))
                .color(p.text_faint),
        );
        ui.add_space(2.0);
    }
    egui::Frame::new()
        .fill(p.well)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
    ui.add_space(14.0);
}

/// The right-hand column: a large icon, a title and what the tab is for.
fn explain(ui: &mut Ui, icon: Icon, title: &str, text: &str) {
    let p = theme::of(ui);
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
    let tile = Rect::from_center_size(r.center(), vec2(64.0, 64.0));
    ui.painter()
        .add(crate::chrome::card_shadow(&p, 12).as_shape(tile, CornerRadius::same(16)));
    ui.painter()
        .rect_filled(tile, CornerRadius::same(16), theme::MENU_HIGHLIGHT);
    ui.painter().rect_stroke(
        tile.shrink(3.0),
        CornerRadius::same(13),
        Stroke::new(1.5, Color32::from_white_alpha(70)),
        egui::StrokeKind::Inside,
    );
    icons::paint(ui.painter(), icon, tile.shrink(17.0), Color32::WHITE);
    ui.add_space(14.0);
    ui.label(RichText::new(title).font(theme::semibold(17.0)));
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(13.0).color(p.text_dim));
}

/// The body split as Arc's settings: a wide list on the left, a narrower explanation right.
fn columns(ui: &mut Ui) -> (Ui, Ui) {
    let r = ui.max_rect();
    let gap = 28.0;
    let left = (r.width() - gap) * 0.62;
    let layout = Layout::top_down(Align::Min);
    let l = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(r.min, vec2(left, r.height())))
            .layout(layout),
    );
    let rr = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(r.left() + left + gap, r.top()),
                r.max,
            ))
            .layout(layout),
    );
    (l, rr)
}

impl App {
    /// Draws the settings window while it is open.
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            return;
        }
        // A shortcut being recorded takes the next key press.
        let mut recorded = false;
        if let Some(binding) = self.settings.recording {
            let pressed = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, modifiers)) = pressed {
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. })));
                let plain =
                    !(modifiers.command || modifiers.ctrl || modifiers.alt || modifiers.mac_cmd);
                self.settings.notice = None;
                match key {
                    Key::Escape if plain => {}
                    Key::Backspace | Key::Delete if plain => {
                        self.prefs.keymap.set(binding, None);
                    }
                    _ => {
                        let keys = KeyboardShortcut::new(modifiers, key);
                        if let Some(other) = self.prefs.keymap.set(binding, Some(keys)) {
                            self.settings.notice = Some(format!(
                                "{} moved here from “{}”, which now has none.",
                                ctx.format_shortcut(&keys),
                                other.label()
                            ));
                        }
                    }
                }
                self.settings.recording = None;
                recorded = true;
            }
        }
        let p = theme::palette(ctx);
        let pop = anim::spring_from(ctx, Id::new("settings-pop"), 1.0, Some(0.0), Spring::BOUNCY);
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 32.0).min(700.0),
            (screen.height() - 48.0).min(560.0),
        );
        let mut close = !recorded
            && self.settings.recording.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        egui::Area::new(Id::new("settings-backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(screen, Sense::click());
                let alpha = (if p.dark { 90.0 } else { 36.0 } * pop.clamp(0.0, 1.0)) as u8;
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(alpha));
                close |= response.clicked() && !self.settings.fresh;
            });
        let area = egui::Area::new(Id::new("settings"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(screen.center())
            .show(ctx, |ui| {
                ui.multiply_opacity(pop.clamp(0.0, 1.0));
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .corner_radius(CornerRadius::same(14))
                    .stroke(Stroke::new(1.0, p.card_stroke))
                    .shadow(egui::Shadow {
                        offset: [0, 18],
                        blur: 48,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 150 } else { 50 }),
                    })
                    .show(ui, |ui| {
                        let (frame, _) = ui.allocate_exact_size(size, Sense::hover());
                        close |= self.settings_header(ui, frame);
                        let body =
                            Rect::from_min_max(pos2(frame.left(), frame.top() + HEADER), frame.max)
                                .shrink2(vec2(26.0, 18.0));
                        let mut content = ui.new_child(
                            UiBuilder::new()
                                .max_rect(body)
                                .layout(Layout::top_down(Align::Min)),
                        );
                        match self.settings.tab {
                            Tab::General => self.general_tab(&mut content),
                            Tab::Canvas => self.canvas_tab(&mut content),
                            Tab::Shortcuts => self.shortcuts_tab(&mut content),
                            Tab::Agents => self.agents_tab(&mut content),
                            Tab::About => about_tab(&mut content),
                        }
                    });
            });
        let layer = area.response.layer_id;
        ctx.move_to_top(layer);
        let scale = 0.96 + 0.04 * pop;
        let centre = area.response.rect.center().to_vec2();
        ctx.set_transform_layer(
            layer,
            egui::emath::TSTransform::new(centre * (1.0 - scale), scale),
        );
        self.settings.fresh = false;
        if close {
            self.settings.hide();
        }
    }

    /// The title, the close button and the tabs, sized to their names as Arc's. Returns
    /// whether the close button was clicked.
    fn settings_header(&mut self, ui: &mut Ui, frame: Rect) -> bool {
        let p = theme::of(ui);
        let tab = self.settings.tab;
        ui.painter().text(
            pos2(frame.center().x, frame.top() + 20.0),
            Align2::CENTER_CENTER,
            tab.label(),
            theme::semibold(13.0),
            p.text,
        );
        let close_at = Rect::from_center_size(
            pos2(frame.left() + 20.0, frame.top() + 20.0),
            vec2(12.0, 12.0),
        );
        let close = ui.interact(
            close_at.expand(5.0),
            Id::new("settings-close"),
            Sense::click(),
        );
        close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Close settings"));
        ui.painter()
            .circle_filled(close_at.center(), 6.0, Color32::from_rgb(255, 95, 87));
        if close.hovered() {
            icons::paint(
                ui.painter(),
                Icon::Close,
                close_at.shrink(2.5),
                Color32::from_rgb(110, 20, 15),
            );
        }
        // Each tab as wide as its name, like a macOS toolbar.
        let font = egui::FontId::proportional(11.5);
        let widths: Vec<f32> = Tab::ALL
            .iter()
            .map(|t| {
                let w = ui
                    .painter()
                    .layout_no_wrap(t.label().to_owned(), font.clone(), p.text)
                    .size()
                    .x;
                (w + 22.0).max(58.0)
            })
            .collect();
        let total: f32 = widths.iter().sum::<f32>() + 4.0 * (widths.len() - 1) as f32;
        let mut x = frame.center().x - total / 2.0;
        let top = frame.top() + 38.0;
        let mut rects = Vec::new();
        for w in &widths {
            rects.push(Rect::from_min_size(pos2(x, top), vec2(*w, 46.0)));
            x += w + 4.0;
        }
        let index = Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
        let lit = rects[index];
        let ctx = ui.ctx().clone();
        let lx = anim::spring(&ctx, Id::new("settings-tab-x"), lit.left(), Spring::SMOOTH);
        let lw = anim::spring(&ctx, Id::new("settings-tab-w"), lit.width(), Spring::SMOOTH);
        let fill = if p.dark {
            Color32::from_white_alpha(20)
        } else {
            Color32::from_black_alpha(12)
        };
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(lx, lit.top()), vec2(lw, lit.height())),
            CornerRadius::same(8),
            fill,
        );
        for (i, (t, r)) in Tab::ALL.into_iter().zip(rects).enumerate() {
            let response = ui.interact(r, Id::new(("settings-tab", i)), Sense::click());
            let on = tab == t;
            response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, on, t.label()));
            let (hover, _) = icons::feedback(ui, &response);
            let color = if on {
                theme::MENU_HIGHLIGHT.lerp_to_gamma(Color32::WHITE, if p.dark { 0.25 } else { 0.0 })
            } else {
                p.text_dim.lerp_to_gamma(p.text, hover)
            };
            icons::paint(
                ui.painter(),
                t.icon(),
                Rect::from_center_size(pos2(r.center().x, r.top() + 16.0), vec2(18.0, 18.0)),
                color,
            );
            ui.painter().text(
                pos2(r.center().x, r.bottom() - 10.0),
                Align2::CENTER_CENTER,
                t.label(),
                font.clone(),
                color,
            );
            if response.clicked() {
                self.settings.tab = t;
                self.settings.recording = None;
            }
        }
        ui.painter().hline(
            frame.x_range(),
            frame.top() + HEADER - 4.0,
            Stroke::new(1.0, p.separator),
        );
        close.clicked()
    }

    fn general_tab(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let (mut ui, mut right) = columns(ui);
        let ui = &mut ui;
        group(ui, "Look", |ui| {
            let mut picked = None;
            setting(ui, "Appearance", "", |ui| {
                picked = segmented(
                    ui,
                    "appearance",
                    self.appearance,
                    &[
                        ("System", Appearance::System),
                        ("Light", Appearance::Light),
                        ("Dark", Appearance::Dark),
                    ],
                );
            });
            if let Some(a) = picked {
                self.apply(Action::Appearance(a), &ctx);
            }
            let mut reduce = self.reduce_motion;
            setting(ui, "Reduce motion", "No springs, slides or fades", |ui| {
                if switch(ui, &mut reduce, "Reduce motion").changed() {
                    self.apply(Action::ReduceMotion, &ctx);
                }
            });
            #[cfg(target_os = "macos")]
            setting(
                ui,
                "Translucent sidebar",
                "The desktop shows through, blurred",
                |ui| {
                    switch(ui, &mut self.prefs.translucent, "Translucent sidebar");
                },
            );
        });
        group(ui, "Window", |ui| {
            setting(ui, "Show the sidebar", "", |ui| {
                switch(ui, &mut self.show_left, "Show the sidebar");
            });
            setting(ui, "Show the properties", "", |ui| {
                switch(ui, &mut self.show_right, "Show the properties");
            });
        });
        explain(
            &mut right,
            Icon::Gear,
            "General",
            "How figstudio looks. The appearance can follow the system or stay light or dark; \
             each space keeps its own colour (right-click its dot).",
        );
    }

    fn canvas_tab(&mut self, ui: &mut Ui) {
        let (mut ui, mut right) = columns(ui);
        let ui = &mut ui;
        #[cfg(not(target_arch = "wasm32"))]
        group(ui, "Drawing", |ui| {
            let note = format!(
                "Now: {}. Applies the next time figstudio opens",
                self.renderer
            );
            let mut picked = None;
            setting(ui, "Renderer", &note, |ui| {
                picked = segmented(
                    ui,
                    "renderer",
                    self.prefs.renderer,
                    &[
                        ("Auto", Renderer::Automatic),
                        ("GPU", Renderer::Gpu),
                        ("CPU", Renderer::Cpu),
                    ],
                );
            });
            if let Some(r) = picked {
                self.prefs.renderer = r;
            }
        });
        group(ui, "On the canvas", |ui| {
            setting(ui, "Frame names", "Above top-level frames", |ui| {
                switch(ui, &mut self.prefs.frame_labels, "Frame names");
            });
            let mut picked = None;
            setting(
                ui,
                "Scroll wheel",
                "The other one with ⌘ or Ctrl held",
                |ui| {
                    picked = segmented(
                        ui,
                        "scroll",
                        self.prefs.scroll_zooms,
                        &[("Pans", false), ("Zooms", true)],
                    );
                },
            );
            if let Some(z) = picked {
                self.prefs.scroll_zooms = z;
            }
        });
        explain(
            &mut right,
            Icon::Frame,
            "Canvas",
            "How pages are drawn and moved around. The GPU draws faster; the CPU draws \
             everything figstudio supports and is used when the GPU cannot.",
        );
    }

    /// Agent access (MCP), as the View menu's Agent access used to show it.
    fn agents_tab(&mut self, ui: &mut Ui) {
        let (mut ui, mut right) = columns(ui);
        group(&mut ui, "Agent access", |ui| {
            ui.add_space(4.0);
            self.mcp.menu(ui);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| self.mcp.status(ui));
            ui.add_space(4.0);
        });
        explain(
            &mut right,
            Icon::Sparkle,
            "Agents",
            "Let AI agents (Claude, Cursor, Codex…) read the files open here through MCP, \
             offline. Off at every launch; your MCP client connects to the address shown \
             while it is on.",
        );
    }

    /// The Shortcuts tab: the list (searchable by name or keys; a click on a shortcut records
    /// new keys) left, what it is and Reset All right.
    fn shortcuts_tab(&mut self, ui: &mut Ui) {
        let (mut ui, mut right) = columns(ui);
        let p = theme::of(&ui);
        egui::Frame::new()
            .fill(p.well)
            .corner_radius(CornerRadius::same(10))
            .show(&mut ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(ui.available_height());
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::hover());
                            icons::paint(
                                ui.painter(),
                                Icon::Search,
                                Rect::from_center_size(r.center(), vec2(13.0, 13.0)),
                                p.text_faint,
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.filter)
                                    .hint_text("Type a feature name or shortcut")
                                    .frame(egui::Frame::NONE)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    });
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0, p.separator),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin::symmetric(14, 6))
                            .show(ui, |ui| {
                                self.shortcut_list(ui);
                            });
                    });
            });
        explain(
            &mut right,
            Icon::Keyboard,
            "Custom shortcuts",
            "Click a shortcut, then press the keys you want. Backspace removes it, Escape \
             keeps it. Keys another shortcut had move to this one.",
        );
        if let Some(notice) = &self.settings.notice {
            right.add_space(8.0);
            right.label(RichText::new(notice).size(12.0).color(p.warning));
        }
        right.add_space(14.0);
        let any = self.prefs.keymap.any_changed();
        if right
            .add_enabled(any, egui::Button::new("Reset All Shortcuts"))
            .clicked()
        {
            self.prefs.keymap.reset_all();
            self.settings.notice = None;
        }
    }

    fn shortcut_list(&mut self, ui: &mut Ui) {
        let p = theme::of(ui);
        let ctx = ui.ctx().clone();
        let query = self.settings.filter.trim().to_lowercase();
        let keys_of = |b: Binding, map: &crate::keymap::Keymap| {
            map.get(b)
                .map_or_else(|| "---".to_owned(), |s| ctx.format_shortcut(&s))
        };
        let matches = |what: &str, keys: &str| {
            query.is_empty()
                || what.to_lowercase().contains(&query)
                || keys.to_lowercase().contains(&query)
        };
        let changed: Vec<Binding> = Binding::ALL
            .into_iter()
            .filter(|b| self.prefs.keymap.is_changed(*b))
            .collect();
        let mut sections: Vec<(&str, Vec<Binding>)> = Vec::new();
        if !changed.is_empty() {
            sections.push(("Customized Shortcuts", changed.clone()));
        }
        for group in ["General", "View", "Tools", "Spaces"] {
            let rows = Binding::ALL
                .into_iter()
                .filter(|b| b.group() == group && !changed.contains(b))
                .collect();
            sections.push((group, rows));
        }
        let mut shown = 0;
        let mut record = None;
        let mut reset = None;
        for (title, rows) in sections {
            let rows: Vec<Binding> = rows
                .into_iter()
                .filter(|b| matches(b.label(), &keys_of(*b, &self.prefs.keymap)))
                .collect();
            if rows.is_empty() {
                continue;
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(title)
                    .font(theme::semibold(12.0))
                    .color(p.text_dim),
            );
            for (i, binding) in rows.into_iter().enumerate() {
                if i > 0 {
                    let y = ui.cursor().top();
                    ui.painter()
                        .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, p.separator));
                }
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, binding.label()));
                ui.painter().text(
                    pos2(rect.left(), rect.center().y),
                    Align2::LEFT_CENTER,
                    binding.label(),
                    egui::FontId::proportional(13.0),
                    p.text,
                );
                let recording = self.settings.recording == Some(binding);
                let keys = keys_of(binding, &self.prefs.keymap);
                let cap = keycap(ui, &keys, rect.right(), rect.center().y, recording);
                let hit = ui.interact(cap, Id::new(("shortcut", binding)), Sense::click());
                let label = format!("{} shortcut", binding.label());
                hit.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
                if hit
                    .on_hover_text("Click, then press the new keys")
                    .clicked()
                {
                    record = Some(binding);
                }
                if self.prefs.keymap.is_changed(binding) {
                    let at = Rect::from_center_size(
                        pos2(cap.left() - 16.0, rect.center().y),
                        vec2(22.0, 22.0),
                    );
                    let label = format!("Reset {}", binding.label());
                    if ui
                        .put(at, |ui: &mut Ui| {
                            icons::sized_button(ui, Icon::Close, &label, false, 22.0)
                        })
                        .clicked()
                    {
                        reset = Some(binding);
                    }
                }
                shown += 1;
            }
        }
        let fixed: Vec<_> = FIXED
            .iter()
            .filter(|(what, how)| matches(what, how))
            .collect();
        if !fixed.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Pointer and fixed keys")
                    .font(theme::semibold(12.0))
                    .color(p.text_dim),
            );
            for (what, how) in fixed {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, *what));
                ui.painter().text(
                    pos2(rect.left(), rect.center().y),
                    Align2::LEFT_CENTER,
                    *what,
                    egui::FontId::proportional(13.0),
                    p.text,
                );
                ui.painter().text(
                    pos2(rect.right(), rect.center().y),
                    Align2::RIGHT_CENTER,
                    *how,
                    egui::FontId::proportional(11.5),
                    p.text_dim,
                );
                shown += 1;
            }
        }
        if shown == 0 {
            ui.add_space(8.0);
            ui.label(RichText::new("No shortcut matches").color(p.text_dim));
        }
        if let Some(binding) = record {
            self.settings.recording = Some(binding);
            self.settings.notice = None;
        }
        if let Some(binding) = reset {
            self.prefs.keymap.reset(binding);
        }
    }
}

/// The About tab: the mark, the name and version, and what figstudio is.
fn about_tab(ui: &mut Ui) {
    let p = theme::of(ui);
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        let (r, _) = ui.allocate_exact_size(vec2(84.0, 84.0), Sense::hover());
        let tile = Rect::from_center_size(r.center(), vec2(76.0, 76.0));
        let tint = theme::tint(theme::HOME_TINT, p.dark);
        ui.painter()
            .add(crate::chrome::card_shadow(&p, 16).as_shape(tile, CornerRadius::same(20)));
        ui.painter()
            .add(egui::Shape::mesh(crate::chrome::rounded_gradient(
                tile,
                20.0,
                [
                    tint.glow,
                    tint.accent,
                    tint.accent,
                    tint.accent.lerp_to_gamma(tint.glow, 0.4),
                ],
            )));
        icons::paint(ui.painter(), Icon::Frame, tile.shrink(22.0), Color32::WHITE);
        ui.add_space(10.0);
        ui.label(RichText::new(APP_NAME).font(theme::semibold(22.0)));
        ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(p.text_dim));
        ui.add_space(10.0);
        ui.label("Opens Figma .fig local copies offline.");
        ui.label(
            RichText::new("Nothing leaves this computer unless you turn on agent access.")
                .color(p.text_dim),
        );
        ui.add_space(16.0);
        ui.label(
            RichText::new("MIT or Apache-2.0 · sample files from fig2sketch (MIT)")
                .small()
                .color(p.text_faint),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_has_a_name_and_an_icon() {
        for tab in Tab::ALL {
            assert!(!tab.label().is_empty());
            let _ = tab.icon();
        }
        assert_eq!(Renderer::Cpu.env_value(), Some("cpu"));
        assert_eq!(Renderer::Automatic.env_value(), None);
    }
}
