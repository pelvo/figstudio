//! Settings (⌘, / Ctrl+,), laid out like Arc's: the tab's name as the title, tabs with an icon
//! above their name, and below them a list on the left with a short explanation on the right.
//! General, Canvas, Shortcuts, Agents (agent access, the MCP server) and About.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Key, KeyboardShortcut, Layout, Margin,
    Modifiers, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo,
    WidgetType, pos2, vec2,
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

/// Desktop access preferences, without the server's endpoint or per-start bearer token.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct AgentPrefs {
    pub on: bool,
    pub remote: bool,
    pub remote_addr: String,
}

/// What Settings → Agents changes on the desktop MCP server (`McpHost::access`); the
/// browser has its own relay controls instead.
#[derive(Debug)]
pub struct Access<'a> {
    pub on: &'a mut bool,
    pub remote: &'a mut bool,
    pub remote_addr: &'a mut String,
    /// While it runs: where clients connect, and a remote bind's bearer token.
    pub endpoint: Option<(String, Option<String>)>,
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
    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open, or still fading out: plain keys stay away until it is gone.
    pub fn is_up(&self, ctx: &egui::Context) -> bool {
        self.open || anim::on_screen(ctx, Id::new("settings-pop"))
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

/// Height of the toolbar (the title and the tabs), as Arc's.
const TOOLBAR: f32 = 85.0;
/// Where a list row's text starts and its control ends, from the list's edge.
const INSET: f32 = 24.0;
/// The window's corners.
const RADIUS: u8 = 10;
/// Arc's settings blue: fills and rings (the switch track, the tiles' rim, a keycap's ring).
const ACCENT: Color32 = Color32::from_rgb(52, 120, 246);
/// The blue for marks and words on light surfaces (a white keycap, a tile's face, the light
/// lit tab): darker than [`ACCENT`], which reads too faintly there.
const ACCENT_INK: Color32 = Color32::from_rgb(36, 99, 220);
/// A tile's key face.
const TILE_FACE: Color32 = Color32::from_rgb(201, 215, 242);
/// A keycap's keys when not recording.
const KEYCAP_INK: Color32 = Color32::from_gray(88);
/// Memory key: the next list row is the first of its section (no line above it).
const FIRST_ROW: &str = "settings-first-row";

/// Shift, Control, Alt and ⌘ pressed on their own (egui sends them as keys too).
fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// Things done with the pointer or fixed keys: listed, not changeable.
const FIXED: [(&str, &str); 7] = [
    ("Clear the selection", "Esc"),
    (
        "Next or previous match",
        "Enter or Shift+Enter in the find bar",
    ),
    ("Zoom around the pointer", "⌘ or Ctrl + scroll, or pinch"),
    ("Pan", "Scroll, Space + drag, or middle drag"),
    ("Zoom to a layer", "Double-click it"),
    ("Switch spaces", "Two-finger swipe on the sidebar"),
    ("Peek at the hidden sidebar", "Point at the left edge"),
];

/// Arc's settings colours, measured from its dark window; the light ones mirror them.
#[derive(Clone, Copy, Debug)]
struct Look {
    dark: bool,
    toolbar: Color32,
    body: Color32,
    toolbar_line: Color32,
    edge: Color32,
    search: Color32,
    search_line: Color32,
    row_line: Color32,
    lit: Color32,
    title: Color32,
    text: Color32,
    section: Color32,
    dim: Color32,
    button: Color32,
    track: Color32,
    segment: Color32,
    /// The lit tab's icon.
    accent_ink: Color32,
    /// A notice ("moved here from …").
    warning: Color32,
}

impl Look {
    fn of(ui: &Ui) -> Self {
        Self::new(theme::of(ui).dark)
    }

    fn new(dark: bool) -> Self {
        let c = Color32::from_rgb;
        if dark {
            Look {
                dark,
                toolbar: c(49, 48, 47),
                body: c(42, 40, 39),
                toolbar_line: c(57, 55, 54),
                edge: c(62, 62, 62),
                search: c(63, 61, 61),
                search_line: c(36, 35, 34),
                row_line: c(63, 61, 60),
                lit: c(77, 76, 75),
                title: c(173, 172, 171),
                text: c(223, 222, 222),
                section: c(159, 158, 158),
                dim: c(145, 144, 143),
                button: c(58, 56, 55),
                track: c(62, 60, 59),
                segment: c(92, 90, 89),
                accent_ink: c(122, 170, 255),
                warning: c(240, 180, 70),
            }
        } else {
            Look {
                dark,
                toolbar: c(236, 235, 234),
                body: c(247, 246, 245),
                toolbar_line: c(216, 215, 214),
                edge: c(214, 213, 212),
                search: c(234, 233, 232),
                search_line: c(214, 213, 212),
                row_line: c(228, 227, 226),
                lit: c(220, 219, 218),
                title: c(90, 89, 88),
                text: c(30, 30, 30),
                section: c(98, 97, 96),
                dim: c(104, 103, 102),
                button: c(226, 225, 224),
                track: c(226, 225, 224),
                segment: Color32::WHITE,
                accent_ink: ACCENT_INK,
                warning: c(150, 85, 0),
            }
        }
    }
}

/// A keycap's words: blue while it waits for keys.
fn keycap_ink(recording: bool) -> Color32 {
    if recording { ACCENT_INK } else { KEYCAP_INK }
}

/// A toolbar tab's icon and name. The tab you are on has a blue icon and its name in the
/// text colour (blue words on the lit pill would be too faint); the others brighten on hover.
fn tab_ink(look: &Look, on: bool, hover: f32) -> (Color32, Color32) {
    if on {
        (look.accent_ink, look.text)
    } else {
        let c = look.section.lerp_to_gamma(look.text, hover);
        (c, c)
    }
}

/// A keycap: the keys on a white rounded box, as Arc draws them, ending at `right`.
/// `recording` outlines it in blue and asks for keys.
fn keycap(ui: &Ui, text: &str, right: f32, y: f32, recording: bool) -> Rect {
    let look = Look::of(ui);
    let (shown, ink) = if recording {
        ("Type keys…", keycap_ink(true))
    } else {
        (text, keycap_ink(false))
    };
    let galley = ui
        .painter()
        .layout_no_wrap(shown.to_owned(), FontId::proportional(12.5), ink);
    let w = (galley.size().x + 22.0).max(62.0);
    let cap = Rect::from_min_size(pos2(right - w, y - 12.0), vec2(w, 24.0));
    ui.painter()
        .rect_filled(cap, CornerRadius::same(5), Color32::WHITE);
    // The blue ring fades in while it waits for keys, and out after.
    let ring = anim::fade(
        ui.ctx(),
        Id::new(("keycap-ring", right as i32, y as i32)),
        if recording {
            ACCENT
        } else {
            Color32::TRANSPARENT
        },
        0.15,
    );
    if !look.dark {
        ui.painter().rect_stroke(
            cap,
            CornerRadius::same(5),
            Stroke::new(1.0, Color32::from_black_alpha(36)),
            StrokeKind::Outside,
        );
    }
    ui.painter().rect_stroke(
        cap,
        CornerRadius::same(5),
        Stroke::new(2.0, ring),
        StrokeKind::Outside,
    );
    ui.painter()
        .galley(cap.center() - galley.size() / 2.0, galley, ink);
    cap
}

/// A switch, macOS style: the knob slides, the track fills with blue when on.
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
        Color32::from_gray(84)
    } else {
        Color32::from_gray(214)
    };
    let track = off.lerp_to_gamma(ACCENT, t.clamp(0.0, 1.0));
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

/// A row of choices, each as wide as its name; the chosen one is lit. Returns the one
/// clicked.
fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    current: T,
    choices: &[(&str, T)],
) -> Option<T> {
    if choices.is_empty() {
        return None;
    }
    let look = Look::of(ui);
    let p = theme::of(ui);
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = choices
        .iter()
        .map(|(label, _)| {
            let w = ui
                .painter()
                .layout_no_wrap((*label).to_owned(), font.clone(), look.text)
                .size()
                .x;
            (w + 22.0).max(50.0)
        })
        .collect();
    let (rect, _) =
        ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 4.0, 24.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(7), look.track);
    let mut lefts = Vec::with_capacity(widths.len());
    let mut x = 2.0;
    for w in &widths {
        lefts.push(x);
        x += w;
    }
    let index = choices.iter().position(|(_, v)| *v == current).unwrap_or(0);
    let lx = anim::spring(
        ui.ctx(),
        Id::new((id, "lit-x")),
        lefts[index],
        Spring::SMOOTH,
    );
    let lw = anim::spring(
        ui.ctx(),
        Id::new((id, "lit-w")),
        widths[index],
        Spring::SMOOTH,
    );
    let lit = Rect::from_min_size(pos2(rect.left() + lx, rect.top() + 2.0), vec2(lw, 20.0));
    ui.painter()
        .add(crate::chrome::card_shadow(&p, 3).as_shape(lit, CornerRadius::same(5)));
    ui.painter()
        .rect_filled(lit, CornerRadius::same(5), look.segment);
    let mut picked = None;
    for (i, (label, value)) in choices.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.left() + lefts[i], rect.top() + 2.0),
            vec2(widths[i], 20.0),
        );
        let response = ui.interact(r, Id::new((id, i)), Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::Button, true, *value == current, *label)
        });
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            font.clone(),
            look.text,
        );
        if response.clicked() {
            picked = Some(*value);
        }
    }
    picked
}

/// A grey rounded button, as Arc's "Reset All Shortcuts".
fn pill(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let look = Look::of(ui);
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(13.0), look.text);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 26.0), sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text));
    let lift = if enabled && response.hovered() {
        0.08
    } else {
        0.0
    };
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(6),
        look.button.lerp_to_gamma(look.text, lift),
    );
    let ink = if enabled { look.text } else { look.dim };
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    response
}

/// Whether the next list row starts its section, and from now on it does not.
fn first_row(ui: &Ui) -> bool {
    ui.ctx().data_mut(|d| {
        let id = Id::new(FIRST_ROW);
        let first = d.get_temp::<bool>(id).unwrap_or(true);
        d.insert_temp(id, false);
        first
    })
}

/// The line between list rows, from the text's start to near the list's edge.
fn row_line(ui: &Ui, rect: Rect) {
    if !first_row(ui) {
        let look = Look::of(ui);
        ui.painter().hline(
            rect.left() + INSET..=rect.right() - 16.0,
            rect.top(),
            Stroke::new(1.0, look.row_line),
        );
    }
}

/// A section's title in a list ("Look", "Default Shortcuts").
fn section(ui: &mut Ui, title: &str) {
    let look = Look::of(ui);
    // More room above a section that follows rows, as Arc's.
    let at_top = ui.next_widget_position().y <= ui.max_rect().top() + 1.0;
    let height = if at_top { 36.0 } else { 50.0 };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, title));
    ui.painter().text(
        pos2(rect.left() + 16.0, rect.bottom() - 8.0),
        Align2::LEFT_BOTTOM,
        title,
        theme::semibold(12.0),
        look.section,
    );
    ui.ctx()
        .data_mut(|d| d.insert_temp(Id::new(FIRST_ROW), true));
}

/// A list row: the label (and a note under it) left, `control` right and centred. The text
/// wraps short of the control, whose width is remembered from the last frame (a change
/// redraws the frame), so the two never overlap.
fn setting(ui: &mut Ui, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
    let look = Look::of(ui);
    let width = ui.available_width();
    let remembered = Id::new(("settings-control-width", label));
    let control_w = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(remembered))
        .unwrap_or(0.0);
    let wrap = (width - 2.0 * INSET - control_w - 16.0).max(80.0);
    let title = ui.painter().layout(
        label.to_owned(),
        FontId::proportional(13.0),
        look.text,
        wrap,
    );
    let note = (!note.is_empty()).then(|| {
        ui.painter()
            .layout(note.to_owned(), FontId::proportional(11.5), look.dim, wrap)
    });
    let text = vec2(
        note.as_ref()
            .map_or(title.size().x, |n| title.size().x.max(n.size().x)),
        title.size().y + note.as_ref().map_or(0.0, |n| 2.0 + n.size().y),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, (text.y + 20.0).max(48.0)), Sense::hover());
    row_line(ui, rect);
    let at = pos2(rect.left() + INSET, rect.center().y - text.y / 2.0);
    let spoken = if note.is_some() {
        format!("{label}. {}", note.as_ref().map_or("", |n| n.text()))
    } else {
        label.to_owned()
    };
    ui.interact(
        Rect::from_min_size(at, text),
        Id::new(("settings-text", label)),
        Sense::hover(),
    )
    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &spoken));
    let title_h = title.size().y;
    ui.painter().galley(at, title, look.text);
    if let Some(note) = note {
        ui.painter()
            .galley(at + vec2(0.0, title_h + 2.0), note, look.dim);
    }
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                rect.min,
                pos2(rect.right() - INSET, rect.bottom()),
            ))
            .layout(Layout::right_to_left(Align::Center)),
    );
    control(&mut child);
    let used = child.min_rect().width();
    if (used - control_w).abs() > 0.5 {
        ui.ctx().data_mut(|d| d.insert_temp(remembered, used));
        ui.ctx().request_discard("a settings control changed width");
    }
}

/// The list's thin rounded outline around `ui`; returns the space inside it.
fn outline(ui: &Ui) -> Rect {
    let look = Look::of(ui);
    let rect = ui.max_rect();
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0, look.edge),
        StrokeKind::Inside,
    );
    rect.shrink(1.0)
}

/// A list as Arc's settings draw it: outlined, scrolling, rows touching.
fn list(ui: &mut Ui, id: &str, body: impl FnOnce(&mut Ui)) {
    let inside = outline(ui);
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(inside)
            .layout(Layout::top_down(Align::Min)),
    );
    inner.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .show(&mut inner, |ui| {
            body(ui);
            ui.add_space(10.0);
        });
}

/// What a tile in the right-hand column shows.
#[derive(Clone, Copy, Debug)]
enum Glyph {
    Icon(Icon),
    Text(&'static str),
}

/// A blue keycap tile, as the ones on Arc's settings pages.
fn tile(painter: &egui::Painter, rect: Rect, glyph: Glyph) {
    painter.rect_filled(
        rect,
        CornerRadius::same(14),
        Color32::from_rgb(48, 103, 204),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(14),
        Stroke::new(1.0, ACCENT),
        StrokeKind::Inside,
    );
    // The key's top: inset, a little higher than centred, as a key seen from the front.
    let face = Rect::from_min_max(rect.min + vec2(4.0, 4.0), rect.max - vec2(4.0, 6.0));
    painter.rect_filled(face, CornerRadius::same(11), TILE_FACE);
    painter.rect_stroke(
        face,
        CornerRadius::same(11),
        Stroke::new(1.0, Color32::from_rgb(239, 243, 251)),
        StrokeKind::Inside,
    );
    match glyph {
        Glyph::Icon(icon) => icons::paint(painter, icon, face.shrink(13.0), ACCENT_INK),
        Glyph::Text(text) => {
            painter.text(
                face.center(),
                Align2::CENTER_CENTER,
                text,
                theme::semibold(28.0),
                ACCENT_INK,
            );
        }
    }
}

/// The right-hand column: blue tiles, a title and what the tab is for.
fn explain(ui: &mut Ui, glyphs: &[Glyph], title: &str, text: &str) {
    let look = Look::of(ui);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
    for (i, glyph) in glyphs.iter().enumerate() {
        let at = Rect::from_min_size(pos2(r.left() + i as f32 * 67.0, r.top()), vec2(64.0, 64.0));
        tile(ui.painter(), at, *glyph);
    }
    ui.add_space(22.0);
    ui.label(
        RichText::new(title)
            .font(theme::semibold(16.0))
            .color(look.text),
    );
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(13.0).color(look.dim));
}

/// The body split as Arc's settings: the list left (about three fifths), a narrower
/// explanation right.
fn columns(ui: &mut Ui) -> (Ui, Ui) {
    let r = ui.max_rect();
    let gap = 16.0;
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
    /// Gives the next key press to the shortcut being recorded. Returns whether it took one.
    /// Call first in the frame: while a shortcut is being recorded, every key and typed
    /// character goes to it (none reach a text field, a shortcut or the canvas). Modifier
    /// keys alone (egui sends Shift, Control, Alt and ⌘ as keys of their own) do not end the
    /// recording; the next other key does, with the modifiers held then as the chord.
    pub(crate) fn record_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let Some(binding) = self.settings.recording() else {
            return false;
        };
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if !is_modifier(*key) => Some((*key, *modifiers)),
                // ⌘X, ⌘C and ⌘V arrive as Cut, Copy and Paste, without a key event.
                egui::Event::Cut => Some((Key::X, i.modifiers)),
                egui::Event::Copy => Some((Key::C, i.modifiers)),
                egui::Event::Paste(_) => Some((Key::V, i.modifiers)),
                _ => None,
            })
        });
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Ime(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                )
            });
        });
        let Some((key, modifiers)) = pressed else {
            return false;
        };
        let plain = !(modifiers.command || modifiers.ctrl || modifiers.alt || modifiers.mac_cmd);
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
        true
    }

    /// Draws the settings window while it is open.
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        // It pops in, and after closing fades and shrinks away taking no input.
        let open = self.settings.open;
        let pop = anim::presence(ctx, Id::new("settings-pop"), open);
        if !open && pop <= 0.001 {
            return;
        }
        let look = Look::new(theme::palette(ctx).dark);
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 32.0).min(680.0),
            (screen.height() - 40.0).min(640.0),
        );
        // A key that ended a recording was taken from the input, so Escape here is not it.
        let mut close = open
            && self.settings.recording.is_none()
            && !self.update_notes_up()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        // Both stay interactable while they fade (inert, sensing only hover): a click then
        // does nothing rather than land on the app under the dimmed screen.
        egui::Area::new(Id::new("settings-backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let sense = if open { Sense::click() } else { Sense::hover() };
                let response = ui.allocate_rect(screen, sense);
                let alpha = (if look.dark { 90.0 } else { 36.0 } * pop.clamp(0.0, 1.0)) as u8;
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(alpha));
                close |= response.clicked() && !self.settings.fresh;
            });
        let area = egui::Area::new(Id::new("settings"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(screen.center())
            .fade_in(false)
            .show(ctx, |ui| {
                if !open {
                    anim::inert(ui);
                }
                ui.multiply_opacity(pop.clamp(0.0, 1.0));
                egui::Frame::new()
                    .fill(look.body)
                    .corner_radius(CornerRadius::same(RADIUS))
                    .shadow(egui::Shadow {
                        offset: [0, 18],
                        blur: 48,
                        spread: 0,
                        color: Color32::from_black_alpha(if look.dark { 150 } else { 50 }),
                    })
                    .show(ui, |ui| {
                        let (frame, _) = ui.allocate_exact_size(size, Sense::hover());
                        close |= self.settings_header(ui, frame);
                        // Arc keeps the page's content in a centred column.
                        let margin = ((frame.width() - 560.0) / 2.0).clamp(16.0, 60.0);
                        // A tab's page fades in and settles from a few points down.
                        let shown = anim::arrive(
                            ui.ctx(),
                            Id::new("settings-page"),
                            self.settings.tab.label(),
                            0.18,
                        );
                        let body = Rect::from_min_max(
                            pos2(frame.left() + margin, frame.top() + TOOLBAR + 26.0),
                            pos2(frame.right() - margin, frame.bottom() - 26.0),
                        )
                        .translate(vec2(0.0, (1.0 - shown) * 6.0));
                        let mut content = ui.new_child(
                            UiBuilder::new()
                                .max_rect(body)
                                .layout(Layout::top_down(Align::Min)),
                        );
                        content.multiply_opacity(shown);
                        match self.settings.tab {
                            Tab::General => self.general_tab(&mut content),
                            Tab::Canvas => self.canvas_tab(&mut content),
                            Tab::Shortcuts => self.shortcuts_tab(&mut content),
                            Tab::Agents => self.agents_tab(&mut content),
                            Tab::About => self.about_tab(&mut content),
                        }
                        let rim = if look.dark {
                            Color32::from_black_alpha(170)
                        } else {
                            Color32::from_black_alpha(40)
                        };
                        ui.painter().rect_stroke(
                            frame,
                            CornerRadius::same(RADIUS),
                            Stroke::new(1.0, rim),
                            StrokeKind::Outside,
                        );
                    });
            });
        let layer = area.response.layer_id;
        // egui raises a frame's layers in no fixed order: the update popup above Settings
        // raises itself, so Settings does not while it is open.
        if !self.update_notes_up() {
            ctx.move_to_top(layer);
        }
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

    /// The toolbar: the window buttons, the tab's name as the title, and the tabs (an icon
    /// above each name, each as wide as its name) as Arc's. Returns whether close was
    /// clicked.
    fn settings_header(&mut self, ui: &mut Ui, frame: Rect) -> bool {
        let look = Look::of(ui);
        let tab = self.settings.tab;
        let bar = Rect::from_min_max(frame.min, pos2(frame.right(), frame.top() + TOOLBAR));
        ui.painter().rect_filled(
            bar,
            CornerRadius {
                nw: RADIUS,
                ne: RADIUS,
                sw: 0,
                se: 0,
            },
            look.toolbar,
        );
        ui.painter().hline(
            frame.x_range(),
            bar.bottom() - 0.5,
            Stroke::new(1.0, look.toolbar_line),
        );
        ui.painter().text(
            pos2(frame.center().x, frame.top() + 14.0),
            Align2::CENTER_CENTER,
            tab.label(),
            theme::semibold(13.0),
            look.title,
        );
        // The window buttons: close works; the other two are greyed, as macOS shows what a
        // window cannot do.
        let y = frame.top() + 14.0;
        let close_at = pos2(frame.left() + 16.0, y);
        let close = ui.interact(
            Rect::from_center_size(close_at, vec2(22.0, 22.0)),
            Id::new("settings-close"),
            Sense::click(),
        );
        close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Close settings"));
        ui.painter()
            .circle_filled(close_at, 6.5, Color32::from_rgb(255, 95, 87));
        if close.hovered() {
            icons::paint(
                ui.painter(),
                Icon::Close,
                Rect::from_center_size(close_at, vec2(8.0, 8.0)),
                Color32::from_rgb(110, 20, 15),
            );
        }
        for x in [39.0, 62.0] {
            ui.painter()
                .circle_filled(pos2(frame.left() + x, y), 6.5, look.lit);
        }
        // Each tab as wide as its name, touching, centred.
        let font = FontId::proportional(12.5);
        let widths: Vec<f32> = Tab::ALL
            .iter()
            .map(|t| {
                let w = ui
                    .painter()
                    .layout_no_wrap(t.label().to_owned(), font.clone(), look.text)
                    .size()
                    .x;
                (w + 14.0).max(56.0)
            })
            .collect();
        let total: f32 = widths.iter().sum();
        let mut x = frame.center().x - total / 2.0;
        let top = frame.top() + 29.0;
        let mut rects = Vec::new();
        for w in &widths {
            rects.push(Rect::from_min_size(pos2(x, top), vec2(*w, 48.0)));
            x += w;
        }
        let index = Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
        let lit = rects[index];
        let ctx = ui.ctx().clone();
        let lx = anim::spring(
            &ctx,
            Id::new("settings-tab-x"),
            lit.left() - frame.left(),
            Spring::SMOOTH,
        );
        let lw = anim::spring(&ctx, Id::new("settings-tab-w"), lit.width(), Spring::SMOOTH);
        let lit = Rect::from_min_size(pos2(frame.left() + lx, lit.top()), vec2(lw, lit.height()));
        ui.painter()
            .rect_filled(lit, CornerRadius::same(7), look.lit);
        let rim = if look.dark {
            Color32::from_white_alpha(10)
        } else {
            Color32::from_black_alpha(14)
        };
        ui.painter().rect_stroke(
            lit,
            CornerRadius::same(7),
            Stroke::new(1.0, rim),
            StrokeKind::Inside,
        );
        for (i, (t, r)) in Tab::ALL.into_iter().zip(rects).enumerate() {
            let response = ui.interact(r, Id::new(("settings-tab", i)), Sense::click());
            let on = tab == t;
            response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, on, t.label()));
            let (hover, _) = icons::feedback(ui, &response);
            let (icon, name) = tab_ink(&look, on, hover);
            icons::paint(
                ui.painter(),
                t.icon(),
                Rect::from_center_size(pos2(r.center().x, r.top() + 17.0), vec2(21.0, 21.0)),
                icon,
            );
            ui.painter().text(
                pos2(r.center().x, r.top() + 37.0),
                Align2::CENTER_CENTER,
                t.label(),
                font.clone(),
                name,
            );
            if response.clicked() {
                self.settings.tab = t;
                self.settings.recording = None;
            }
        }
        close.clicked()
    }

    fn general_tab(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let (mut left, mut right) = columns(ui);
        list(&mut left, "general", |ui| {
            section(ui, "Look");
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
            section(ui, "Window");
            setting(ui, "Show the sidebar", "", |ui| {
                switch(ui, &mut self.show_left, "Show the sidebar");
            });
            setting(ui, "Show the properties", "", |ui| {
                switch(ui, &mut self.show_right, "Show the properties");
            });
            section(ui, "Help");
            let mut tour = false;
            setting(ui, "Welcome tour", "The first-run sheet and tips", |ui| {
                tour = pill(ui, "Show again", true).clicked();
            });
            if tour {
                self.apply(Action::Welcome, &ctx);
            }
        });
        explain(
            &mut right,
            &[Glyph::Icon(Icon::Gear)],
            "General",
            "How figstudio looks. The appearance can follow the system or stay light or \
             dark; each space keeps its own colour (right-click its dot).",
        );
    }

    fn canvas_tab(&mut self, ui: &mut Ui) {
        let (mut left, mut right) = columns(ui);
        list(&mut left, "canvas", |ui| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                section(ui, "Drawing");
                let note = format!("Now {}. Applies at the next start", self.renderer);
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
            }
            section(ui, "On the canvas");
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
            &[Glyph::Icon(Icon::Frame)],
            "Canvas",
            "How pages are drawn and moved around. The GPU draws faster; the CPU draws \
             everything figstudio supports and is used when the GPU cannot.",
        );
    }

    /// Agent access (MCP), as the View menu's Agent access used to show it.
    fn agents_tab(&mut self, ui: &mut Ui) {
        let (mut left, mut right) = columns(ui);
        list(&mut left, "agents", |ui| {
            section(ui, "MCP Server");
            if let Some(access) = self.mcp.access() {
                let on = *access.on;
                setting(
                    ui,
                    "Agent access",
                    "An MCP server for the open files. Remembered between launches",
                    |ui| {
                        switch(ui, access.on, "Agent access");
                    },
                );
                setting(
                    ui,
                    "Allow remote agents",
                    "Listen on an address you give, not only on this computer. Clients send \
                     the access token (Copy token, once on); plain HTTP, so use Tailscale or \
                     an SSH tunnel",
                    |ui| {
                        ui.add_enabled_ui(!on, |ui| {
                            switch(ui, access.remote, "Allow remote agents");
                        });
                    },
                );
                if *access.remote {
                    setting(ui, "Address", "This computer's address and port", |ui| {
                        ui.add_enabled_ui(!on, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(access.remote_addr)
                                    .hint_text("100.x.y.z:3845")
                                    .desired_width(130.0),
                            );
                        });
                    });
                }
                if let Some((address, token)) = access.endpoint {
                    setting(ui, "Connect to", &address, |ui| {
                        if let Some(token) = token
                            && pill(ui, "Copy token", true)
                                .on_hover_text("Send it as 'Authorization: Bearer <token>'.")
                                .clicked()
                        {
                            ui.ctx().copy_text(token);
                        }
                    });
                }
            } else {
                egui::Frame::new()
                    .inner_margin(Margin {
                        left: INSET as i8,
                        right: INSET as i8,
                        top: 6,
                        bottom: 6,
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        self.mcp.menu(ui);
                        ui.horizontal_wrapped(|ui| self.mcp.status(ui));
                    });
            }
        });
        explain(
            &mut right,
            &[Glyph::Icon(Icon::Sparkle)],
            "Agents",
            "Let AI agents (Claude, Cursor, Codex…) read the files open here through MCP, \
             offline. Desktop access settings are remembered between launches; your MCP client \
             connects to the address shown while it is on.",
        );
    }

    /// The Shortcuts tab: the list (searchable by name or keys; a click on a shortcut records
    /// new keys) left, what it is and Reset All right.
    fn shortcuts_tab(&mut self, ui: &mut Ui) {
        let (mut left, mut right) = columns(ui);
        let look = Look::of(&left);
        let inside = outline(&left);
        let band = Rect::from_min_size(inside.min, vec2(inside.width(), 40.0));
        left.painter().rect_filled(
            band,
            CornerRadius {
                nw: 7,
                ne: 7,
                sw: 0,
                se: 0,
            },
            look.search,
        );
        left.painter().hline(
            band.x_range(),
            band.bottom() - 0.5,
            Stroke::new(1.0, look.search_line),
        );
        icons::paint(
            left.painter(),
            Icon::Search,
            Rect::from_center_size(pos2(band.left() + 18.0, band.center().y), vec2(14.0, 14.0)),
            look.section,
        );
        let mut search = left.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(band.left() + 34.0, band.top()),
                    pos2(band.right() - 12.0, band.bottom() - 1.0),
                ))
                .layout(Layout::left_to_right(Align::Center)),
        );
        search.add(
            egui::TextEdit::singleline(&mut self.settings.filter)
                .hint_text(RichText::new("Type a feature name or shortcut").color(look.title))
                .font(FontId::proportional(13.5))
                .text_color(look.text)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY),
        );
        let mut rows = left.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(inside.left(), band.bottom()),
                    inside.max,
                ))
                .layout(Layout::top_down(Align::Min)),
        );
        rows.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("shortcuts")
            .auto_shrink([false, false])
            .show(&mut rows, |ui| {
                self.shortcut_list(ui);
                ui.add_space(10.0);
            });
        explain(
            &mut right,
            &[Glyph::Text("⌘"), Glyph::Icon(Icon::Keyboard)],
            "Custom Shortcuts",
            "Click a shortcut, then press the keys you want. Backspace removes it, Escape \
             keeps it. Keys another shortcut had move to this one.",
        );
        if let Some(notice) = &self.settings.notice {
            right.add_space(10.0);
            let shown = anim::arrive(right.ctx(), Id::new("settings-notice"), notice, 0.2);
            right.scope(|ui| {
                ui.multiply_opacity(shown);
                ui.label(RichText::new(notice).size(12.5).color(Look::of(ui).warning));
            });
        }
        let column = right.max_rect();
        let mut foot = right.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(column.left(), column.bottom() - 26.0),
                    column.max,
                ))
                .layout(Layout::left_to_right(Align::Center)),
        );
        if pill(
            &mut foot,
            "Reset All Shortcuts",
            self.prefs.keymap.any_changed(),
        )
        .clicked()
        {
            self.prefs.keymap.reset_all();
            self.settings.notice = None;
        }
    }

    fn shortcut_list(&mut self, ui: &mut Ui) {
        let look = Look::of(ui);
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
            section(ui, title);
            for binding in rows {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
                row_line(ui, rect);
                let recording = self.settings.recording == Some(binding);
                let keys = keys_of(binding, &self.prefs.keymap);
                let cap = keycap(ui, &keys, rect.right() - INSET, rect.center().y, recording);
                // The name wraps short of the keycap and its reset button.
                let room = (cap.left() - 36.0 - rect.left() - INSET).max(60.0);
                let galley = ui.painter().layout(
                    binding.label().to_owned(),
                    FontId::proportional(13.0),
                    look.text,
                    room,
                );
                let at = pos2(rect.left() + INSET, rect.center().y - galley.size().y / 2.0);
                ui.interact(
                    Rect::from_min_size(at, galley.size()),
                    Id::new(("shortcut-name", binding)),
                    Sense::hover(),
                )
                .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, binding.label()));
                ui.painter().galley(at, galley, look.text);
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
            section(ui, "Pointer and Fixed Keys");
            for (what, how) in fixed {
                setting(ui, what, how, |_| {});
                shown += 1;
            }
        }
        if shown == 0 {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(INSET);
                ui.label(RichText::new("No shortcut matches").color(look.dim));
            });
        }
        if let Some(binding) = record {
            self.settings.recording = Some(binding);
            self.settings.notice = None;
            // The menu bar's own shortcuts are taken away now, before the keys are pressed.
            self.sync_native_keys();
        }
        if let Some(binding) = reset {
            self.prefs.keymap.reset(binding);
        }
    }
}

impl App {
    /// The About tab: the mark, the name and version, what figstudio is, and (desktop) the
    /// update rows.
    fn about_tab(&mut self, ui: &mut Ui) {
        let p = theme::of(ui);
        let look = Look::of(ui);
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            let (r, _) = ui.allocate_exact_size(vec2(84.0, 84.0), Sense::hover());
            // The app's icon (the PNG has its own margin and shadow); the drawn mark in the
            // browser, which does not carry the PNG, or if it does not decode.
            #[cfg(not(target_arch = "wasm32"))]
            let drawn = crate::chrome::paint_app_icon(ui, r.expand(10.0));
            #[cfg(target_arch = "wasm32")]
            let drawn = false;
            if !drawn {
                let tint = theme::tint(theme::HOME_TINT, p.dark);
                crate::chrome::paint_mark(ui.painter(), &p, tint, r.center(), 76.0);
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(APP_NAME)
                    .font(theme::semibold(22.0))
                    .color(look.text),
            );
            ui.label(
                RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(look.dim),
            );
            ui.add_space(10.0);
            ui.label(RichText::new("Opens Figma .fig local copies offline.").color(look.text));
            #[cfg(target_arch = "wasm32")]
            let privacy = "Nothing leaves this computer unless you turn on agent access.";
            #[cfg(not(target_arch = "wasm32"))]
            let privacy = "Nothing leaves this computer unless you turn on agent access or \
                           check for updates.";
            ui.label(RichText::new(privacy).color(look.dim));
            ui.add_space(16.0);
            ui.label(
                RichText::new("Apache-2.0 · sample files from fig2sketch (MIT)")
                    .small()
                    .color(look.dim),
            );
        });
        #[cfg(not(target_arch = "wasm32"))]
        self.updates(ui);
    }

    /// Settings → About's update rows (desktop; the browser is always served the latest).
    #[cfg(not(target_arch = "wasm32"))]
    fn updates(&mut self, ui: &mut Ui) {
        use crate::update::{self, State};
        use crate::update_view::{Ask, offer};
        ui.add_space(18.0);
        let rest = Rect::from_min_max(
            pos2(ui.max_rect().left(), ui.cursor().top()),
            ui.max_rect().max,
        );
        let mut rows = ui.new_child(
            UiBuilder::new()
                .max_rect(rest)
                .layout(Layout::top_down(Align::Min)),
        );
        let route = update::route();
        let progress = self.updater.progress();
        let mut ask = None;
        list(&mut rows, "about-updates", |ui| {
            section(ui, "Updates");
            setting(
                ui,
                "Check at start",
                "Asks GitHub for a newer release once per launch",
                |ui| {
                    switch(ui, &mut self.prefs.check_updates, "Check at start");
                },
            );
            let check = |ui: &mut Ui, ask: &mut Option<Ask>, text: &str, enabled: bool| {
                if pill(ui, text, enabled).clicked() {
                    *ask = Some(Ask::Check);
                }
            };
            match &self.updater.state {
                State::Idle => setting(ui, "Check for updates", "", |ui| {
                    check(ui, &mut ask, "Check now", true);
                }),
                State::Checking => setting(ui, "Checking for updates…", "", |ui| {
                    check(ui, &mut ask, "Check now", false);
                }),
                State::UpToDate => {
                    let note = format!("Version {} is the latest", update::current());
                    setting(ui, "figstudio is up to date", &note, |ui| {
                        check(ui, &mut ask, "Check now", true);
                    });
                }
                State::Failed(why) => setting(ui, "The update did not work", why, |ui| {
                    check(ui, &mut ask, "Try again", true);
                }),
                State::Available(release) => {
                    let title = format!("Version {} is available", release.version);
                    let offer = offer(&route, release);
                    setting(ui, &title, &offer.note, |ui| {
                        if let Some((text, then)) = offer.action
                            && pill(ui, text, true).clicked()
                        {
                            ask = Some(then);
                        }
                        if pill(ui, "What's new", true).clicked() {
                            ask = Some(Ask::Notes);
                        }
                    });
                }
                State::Installing(release) => {
                    let title = format!("Installing version {}", release.version);
                    let note = match progress {
                        Some(p) if p < 1.0 => format!("Downloading… {:.0}%", p * 100.0),
                        _ => "Checking and replacing the app…".to_owned(),
                    };
                    setting(ui, &title, &note, |ui| {
                        ui.spinner();
                    });
                }
                State::Installed(release, bundle) => {
                    let title = format!("Version {} is installed", release.version);
                    setting(ui, &title, "Restart figstudio to use it", |ui| {
                        if pill(ui, "Restart now", true).clicked() {
                            ask = Some(Ask::Restart(bundle.clone()));
                        }
                    });
                }
            }
        });
        if let Some(ask) = ask {
            let ctx = ui.ctx().clone();
            self.update_ask(ask, &ctx);
        }
    }
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

#[cfg(test)]
mod look_tests {
    use super::*;

    /// Every word and mark Settings draws, on what it is drawn on, at its WCAG floor: 4.5:1
    /// for text, 3:1 for icons and large text (the 28 pt tile glyph).
    #[test]
    fn settings_colours_are_readable_in_both_themes() {
        for dark in [false, true] {
            let l = Look::new(dark);
            let (lit_icon, lit_name) = tab_ink(&l, true, 0.0);
            let (tab, _) = tab_ink(&l, false, 0.0);
            let pairs: [(&str, Color32, Color32, f32); 15] = [
                ("row text", l.text, l.body, 4.5),
                ("row note", l.dim, l.body, 4.5),
                ("section title", l.section, l.body, 4.5),
                ("window title", l.title, l.toolbar, 4.5),
                ("a tab", tab, l.toolbar, 4.5),
                ("the lit tab's name", lit_name, l.lit, 4.5),
                ("the lit tab's icon", lit_icon, l.lit, 3.0),
                ("a chosen segment", l.text, l.segment, 4.5),
                ("a segment", l.text, l.track, 4.5),
                ("a pill", l.text, l.button, 4.5),
                ("the search hint", l.title, l.search, 4.5),
                ("a notice", l.warning, l.body, 4.5),
                ("a keycap", keycap_ink(false), Color32::WHITE, 4.5),
                ("a keycap waiting", keycap_ink(true), Color32::WHITE, 4.5),
                ("a tile's glyph", ACCENT_INK, TILE_FACE, 3.0),
            ];
            for (what, ink, under, floor) in pairs {
                let c = theme::contrast(ink, under);
                assert!(
                    c >= floor,
                    "{what} (dark {dark}): {c:.2}:1, needs {floor}:1"
                );
            }
        }
    }
}
