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
use crate::spaces::{self, Symbol};
use crate::theme::{self, CARD_RADIUS, GAP, Palette, Tint};
use crate::{APP_NAME, App, panels};

/// How far the sidebar's content slides when switching to a space that is not next to the
/// current one (a click on a far dot, the command bar); neighbours turn a whole page.
const SLIDE: f32 = 36.0;
/// Space between two spaces' pages while they slide side by side.
const PAGE_GAP: f32 = 24.0;
/// The sidebar panel's id (its width is kept with egui's memory).
/// Renamed once, so a width the old slide had inflated is not restored.
pub(crate) const SIDEBAR: &str = "space-sidebar";
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

/// Paints the window gradient for `tint` over `rect` (see [`gradient_mesh`]).
pub(crate) fn paint_gradient(painter: &egui::Painter, rect: Rect, tint: Tint, alpha: f32) {
    painter.add(Shape::mesh(gradient_mesh(rect, tint, alpha)));
}

/// The window gradient for `tint` over `rect`: `top` at the top left, `bottom` at the bottom
/// right, a soft `glow` near the top. A grid of vertices keeps the glow round. `alpha` below
/// 1 lets the window's blur show through (macOS vibrancy).
pub(crate) fn gradient_mesh(rect: Rect, tint: Tint, alpha: f32) -> Mesh {
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
                color: base.lerp_to_gamma(tint.glow, glow).gamma_multiply(alpha),
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
    mesh
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

/// Opens `add` as a right-click menu on `response`, in the macOS style (`theme::menu_style`).
pub(crate) fn context_menu(response: &Response, add: impl FnOnce(&mut Ui)) {
    let menu = egui::Popup::context_menu(response)
        .style(theme::menu_style)
        .show(add);
    if let Some(menu) = menu {
        crate::a11y::name_menu(&menu.response, "Context menu");
    }
}

/// What was clicked among the file dots this frame.
#[derive(Debug, Default)]
struct Dots {
    pick: Option<usize>,
    close: Option<usize>,
    action: Option<(usize, Action)>,
}

/// Width of a file dot's slot in the sidebar's footer.
pub(crate) const DOT: f32 = 26.0;
/// Most a dot grows (the spring's overshoot plus hover).
const DOT_MAX_SCALE: f32 = 1.15;

/// The badge's size for a dot at `scale`: it stays inside the slot, so neighbouring dots
/// never overlap.
pub(crate) fn dot_size(symbol: bool, scale: f32) -> f32 {
    let scale = scale.clamp(0.0, DOT_MAX_SCALE);
    if symbol { 20.0 * scale } else { 13.0 * scale }
}

/// A colour drained toward grey and faded by `away` (0: as it is, 1: an inactive space's
/// badge), as Arc shows the spaces you are not in.
fn drained(c: Color32, away: f32) -> Color32 {
    let grey = (0.3 * f32::from(c.r()) + 0.59 * f32::from(c.g()) + 0.11 * f32::from(c.b())) as u8;
    let grey = Color32::from_rgba_unmultiplied(grey, grey, grey, c.a());
    c.lerp_to_gamma(grey, 0.8 * away)
        .gamma_multiply(1.0 - 0.45 * away)
}

/// How long the pointer rests on a dot before its name pill shows, as in Arc (like a
/// tooltip). Within `PILL_WARM` of one showing, the next dot's shows at once, so running
/// along the row reads each name without waiting again.
const PILL_DELAY: f64 = 0.5;
const PILL_WARM: f64 = 0.4;
/// The widest a name gets in the pill before it is cut.
const MAX_PILL_NAME: f32 = 280.0;

/// The dot the pointer rests on: since when, and whether its pill has shown.
#[derive(Clone, Copy)]
struct Rest {
    key: u64,
    since: f64,
    due: bool,
}

/// Whether the pill of the dot `key`, which the pointer is on, is due. Like a tooltip, the
/// wait counts from when the pointer stopped moving; once shown, the pill stays while the
/// pointer is on the dot. Asks for a repaint for when it will be due.
fn pill_due(ctx: &egui::Context, key: u64) -> bool {
    let (now, moved) = ctx.input(|i| {
        (
            i.time,
            i.time - f64::from(i.pointer.time_since_last_movement()),
        )
    });
    let (rest, shown) = (Id::new("space-pill-rest"), Id::new("space-pill-shown"));
    let waited = ctx.data_mut(|d| {
        let last = d.get_temp::<f64>(shown).unwrap_or(f64::NEG_INFINITY);
        let mut r = match d.get_temp::<Rest>(rest) {
            Some(r) if r.key == key => r,
            // Warm: one showed a moment ago, so this one shows straight away.
            _ => Rest {
                key,
                since: now,
                due: now - last <= PILL_WARM,
            },
        };
        if !r.due {
            r.since = r.since.max(moved);
            r.due = now - r.since >= PILL_DELAY;
        }
        d.insert_temp(rest, r);
        if r.due {
            d.insert_temp(shown, now);
            None
        } else {
            Some(now - r.since)
        }
    });
    match waited {
        None => true,
        Some(waited) => {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(PILL_DELAY - waited));
            false
        }
    }
}

/// What the name pill shows: the dot it sits over, the space's name and its keys.
#[derive(Clone, Debug)]
struct PillContent {
    dot: Rect,
    name: String,
    keys: Option<egui::KeyboardShortcut>,
}

/// Arc's hover card over a space: a dark pill with the space's name and its keys as caps,
/// above its dot. It fades and rises in, glides from dot to dot, and fades out after the
/// pointer leaves. `want` is the pill to show this frame, if any.
fn space_pill(ctx: &egui::Context, want: Option<PillContent>) {
    let last = Id::new("space-pill-last");
    let showing = want.is_some();
    let was = ctx.data(|d| {
        d.get_temp::<bool>(Id::new("space-pill-visible"))
            .unwrap_or(false)
    });
    let t = anim::presence(ctx, Id::new("space-pill-t"), showing);
    let content = match want {
        Some(c) => {
            ctx.data_mut(|d| d.insert_temp(last, c.clone()));
            Some(c)
        }
        None => ctx.data(|d| d.get_temp::<PillContent>(last)),
    };
    let visible = t > 0.001 && content.is_some();
    ctx.data_mut(|d| d.insert_temp(Id::new("space-pill-visible"), visible));
    let Some(PillContent { dot, name, keys }) = content.filter(|_| visible) else {
        return;
    };
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    let mut caps: Vec<String> = Vec::new();
    if let Some(keys) = keys {
        let m = keys.modifiers;
        let names: [(bool, &str, &str); 4] = [
            (m.ctrl && !(m.command && !mac), "⌃", "Ctrl"),
            (m.alt, "⌥", "Alt"),
            (m.shift, "⇧", "Shift"),
            (m.command || m.mac_cmd, "⌘", "Ctrl"),
        ];
        for (on, symbol, word) in names {
            if on {
                caps.push(if mac { symbol } else { word }.to_owned());
            }
        }
        caps.dedup();
        caps.push(keys.logical_key.symbol_or_name().to_owned());
    }
    let ink = Color32::from_gray(245);
    let cap_ink = Color32::from_gray(30);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("space-pill"),
    ));
    let caps: Vec<_> = caps
        .into_iter()
        .map(|c| painter.layout_no_wrap(c, theme::semibold(11.0), cap_ink))
        .collect();
    let cap_w = |g: &std::sync::Arc<Galley>| (g.size().x + 8.0).max(17.0);
    let caps_w: f32 = caps.iter().map(|g| cap_w(g) + 3.0).sum();
    let screen = ctx.content_rect();
    // A long name (a file title, say) is cut with an ellipsis so the pill fits the window.
    let room = (screen.width() - 8.0 - 12.0 - 8.0 - caps_w - 2.0).min(MAX_PILL_NAME);
    let mut job = LayoutJob::simple_singleline(name.clone(), theme::semibold(12.0), ink);
    job.wrap = TextWrapping {
        max_width: room.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let title = painter.layout_job(job);
    let height = 26.0;
    let width = 12.0
        + title.size().x
        + if caps.is_empty() {
            12.0
        } else {
            8.0 + caps_w + 2.0
        };
    // Glides between dots while shown; starts over its dot when it appears.
    let glide = Id::new("space-pill-x");
    if !was {
        anim::reset(ctx, glide, dot.center().x);
    }
    let centre = anim::spring(ctx, glide, dot.center().x, Spring::SMOOTH);
    // Inside the window when it fits; from its left edge when even the cut pill does not.
    let x = (centre - width / 2.0)
        .min(screen.right() - width - 4.0)
        .max(screen.left() + 4.0);
    let rect = Rect::from_min_size(pos2(x, dot.top() - 6.0 - height), vec2(width, height));
    let label = match keys {
        Some(keys) => format!("{name}, {}", ctx.format_shortcut(&keys)),
        None => name.clone(),
    };
    let area = egui::Area::new(Id::new("space-pill"))
        .order(egui::Order::Tooltip)
        .fixed_pos(rect.min)
        .interactable(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.multiply_opacity(t.clamp(0.0, 1.0));
            let (rect, response) = ui.allocate_exact_size(rect.size(), Sense::hover());
            if showing {
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
            }
            let painter = ui.painter();
            painter.add(
                Shadow {
                    offset: [0, 3],
                    blur: 10,
                    spread: 0,
                    color: Color32::from_black_alpha(60),
                }
                .as_shape(rect, CornerRadius::same(13)),
            );
            painter.rect_filled(
                rect,
                CornerRadius::same(13),
                Color32::from_rgba_unmultiplied(22, 22, 24, 240),
            );
            let title_w = title.size().x;
            painter.galley(
                pos2(rect.left() + 12.0, rect.center().y - title.size().y / 2.0),
                title,
                ink,
            );
            let mut x = rect.left() + 12.0 + title_w + 8.0;
            for cap in caps {
                let w = cap_w(&cap);
                let r = Rect::from_min_size(pos2(x, rect.center().y - 8.5), vec2(w, 17.0));
                painter.rect_filled(r, CornerRadius::same(4), Color32::from_gray(222));
                painter.galley(r.center() - cap.size() / 2.0, cap, cap_ink);
                x += w + 3.0;
            }
        });
    // Rises a few points and grows from its foot while it appears.
    let scale = 0.9 + 0.1 * t;
    let foot = area.response.rect.center_bottom().to_vec2();
    ctx.set_transform_layer(
        area.response.layer_id,
        egui::emath::TSTransform::new(foot * (1.0 - scale) + vec2(0.0, (1.0 - t) * 4.0), scale),
    );
}

/// Paints a space's badge in the square `rect`, as Arc's: its icon in its `accent`, its emoji
/// (or letter) in `text`, or a plain accent dot. Nothing behind it.
pub(crate) fn paint_badge(
    painter: &egui::Painter,
    rect: Rect,
    symbol: Option<&Symbol>,
    accent: Color32,
    text: Color32,
) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    match symbol {
        Some(Symbol::Icon(name)) if spaces::icon(name).is_some() => {
            if let Some(icon) = spaces::icon(name) {
                icons::paint(painter, icon, rect.shrink(r * 0.12), accent);
            }
        }
        Some(Symbol::Emoji(e)) => {
            painter.text(
                c,
                Align2::CENTER_CENTER,
                e,
                egui::FontId::proportional(r * 1.6),
                text,
            );
        }
        _ => {
            painter.circle_filled(c, r * 0.55, accent);
        }
    }
}

/// What the canvas showed in a frame: enough to paint it again from the tile cache.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CanvasShot {
    /// The file and page shown (`Tab::id`, `Tab::page`).
    key: (u64, figstudio_fig::Guid),
    /// `Document::instance`, which keys the tile cache.
    doc: u64,
    /// The view on screen (`Tab::shown`).
    view: crate::tab::View,
    background: Color32,
}

impl CanvasShot {
    fn of(tab: &crate::tab::Tab) -> Self {
        CanvasShot {
            key: (tab.id, tab.page),
            doc: tab.doc.instance,
            view: tab.shown,
            background: canvas::to_color32(figstudio_render::page_background(&tab.doc, tab.page)),
        }
    }
}

/// The canvas's crossfade: when the file or page changes, the new one fades in over the last
/// picture (its page colour and cached tiles) instead of over the card, which is near black
/// in the dark theme and flashed for a frame or two.
#[derive(Debug, Default)]
pub(crate) struct Crossfade {
    /// The last frame's picture.
    last: Option<CanvasShot>,
    /// The picture being faded from, until the fade ends.
    from: Option<CanvasShot>,
}

impl Crossfade {
    /// Whether a fade may still paint `doc`'s tiles (the last picture, or the one fading).
    fn paints(&self, doc: u64) -> bool {
        [self.last, self.from]
            .iter()
            .flatten()
            .any(|s| s.doc == doc)
    }

    /// Paints what `tab`'s canvas fades in over while `shown` < 1: the picture of the file or
    /// page shown before, or with none (the first file), this page's own colour. Returns the
    /// cached tiles painted.
    fn under(
        &mut self,
        painter: &egui::Painter,
        tiles: &crate::tiles_view::Tiles,
        tab: &crate::tab::Tab,
        (rect, radius): (Rect, f32),
        shown: f32,
        ppp: f32,
    ) -> Vec<crate::tiles::Key> {
        if self.last.is_some_and(|s| s.key != (tab.id, tab.page)) {
            self.from = self.last.take();
        }
        if shown >= 1.0 {
            self.from = None;
            return Vec::new();
        }
        match self.from {
            Some(from) => {
                painter.rect_filled(rect, radius, from.background);
                let page = (from.doc, from.key.1);
                tiles.paint_cached(painter, (rect, radius), page, from.view, ppp)
            }
            None => {
                let background = figstudio_render::page_background(&tab.doc, tab.page);
                painter.rect_filled(rect, radius, canvas::to_color32(background));
                Vec::new()
            }
        }
    }
}

/// The zoom the tool pill shows: the picture's (`Tab::shown`, which eases after a jump),
/// not the jump's target, so the number matches what is on screen.
fn zoom_percent(tab: Option<&crate::tab::Tab>) -> f32 {
    tab.map_or(100.0, |t| t.shown.zoom * 100.0)
}

/// The sidebar pages to draw while they move by `slide`: the active space's, and while a
/// page turn or a swipe is on, the one beside it (index, x offset, opacity). Pages stay side
/// by side until the spring settles; only a far jump's short nudge (`nudge`) fades instead.
fn sidebar_pages(
    active: usize,
    count: usize,
    slide: f32,
    nudge: bool,
    page_width: f32,
) -> Vec<(usize, f32, f32)> {
    if nudge {
        let opacity = 1.0 - (slide.abs() / SLIDE).min(1.0) * 0.85;
        return vec![(active, slide, opacity)];
    }
    let mut pages = vec![(active, slide, 1.0)];
    let beside = if slide.abs() < 0.5 {
        None
    } else if slide > 0.0 {
        active.checked_sub(1)
    } else {
        Some(active + 1).filter(|i| *i < count)
    };
    if let Some(i) = beside {
        pages.push((i, slide - slide.signum() * page_width, 1.0));
    }
    pages
}

/// The app's mark: a rounded tile `side` wide at `centre`, in `tint`'s gradient, with a frame
/// glyph (the empty canvas, the welcome sheet).
pub(crate) fn paint_mark(
    painter: &egui::Painter,
    p: &Palette,
    tint: Tint,
    centre: Pos2,
    side: f32,
) {
    let tile = Rect::from_center_size(centre, vec2(side, side));
    let radius = (side * 0.28).round();
    painter.add(card_shadow(p, 18).as_shape(tile, CornerRadius::same(radius as u8)));
    let colours = [
        tint.glow,
        tint.accent,
        tint.accent,
        tint.accent.lerp_to_gamma(tint.glow, 0.4),
    ];
    painter.add(Shape::mesh(rounded_gradient(tile, radius, colours)));
    icons::paint(
        painter,
        Icon::Frame,
        Rect::from_center_size(tile.center(), vec2(side, side) * 0.47),
        Color32::WHITE,
    );
}

/// The app's own icon (`assets/icon.png`, as the Dock shows it) filling `rect`. Desktop
/// only: the browser build does not carry the 200 KB PNG. Returns false if it could not be
/// decoded (nothing is drawn then).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn paint_app_icon(ui: &Ui, rect: Rect) -> bool {
    let id = Id::new("app-icon-texture");
    let cached = ui
        .ctx()
        .data_mut(|d| d.get_temp::<Option<egui::TextureHandle>>(id));
    let texture = cached.unwrap_or_else(|| {
        let loaded = app_icon_image().map(|image| {
            ui.ctx()
                .load_texture("app-icon", image, egui::TextureOptions::LINEAR)
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, loaded.clone()));
        loaded
    });
    let Some(texture) = texture else {
        return false;
    };
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    true
}

/// The icon decoded and scaled down once on the CPU (wgpu has no mipmaps here), sharp up to
/// about 96 points on a Retina screen.
#[cfg(not(target_arch = "wasm32"))]
fn app_icon_image() -> Option<egui::ColorImage> {
    let decoded = image::load_from_memory(crate::ICON_PNG)
        .inspect_err(|error| tracing::warn!("app icon not decoded: {error}"))
        .ok()?
        .to_rgba8();
    let small = image::imageops::resize(&decoded, 192, 192, image::imageops::FilterType::Lanczos3);
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [small.width() as usize, small.height() as usize],
        small.as_raw(),
    ))
}

/// A button filled with `tint`'s accent at `rect` that lifts on hover. `label` is its
/// accessible name (with its keys, if any); `text` is what it shows.
pub(crate) fn accent_button(
    ui: &mut Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    label: &str,
    tint: Tint,
) -> Response {
    let p = theme::of(ui);
    let response = ui.interact(rect, Id::new(id), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let (hover, press) = icons::feedback(ui, &response);
    let r = rect.translate(vec2(0.0, -hover * 1.5)).shrink(press * 2.0);
    let painter = ui.painter();
    let radius = CornerRadius::same((rect.height() * 0.32) as u8);
    painter.add(card_shadow(&p, (8.0 + hover * 10.0) as u8).as_shape(r, radius));
    let fill = tint.accent.lerp_to_gamma(Color32::WHITE, hover * 0.1);
    painter.rect_filled(r, radius, fill);
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        text,
        theme::semibold(14.0),
        theme::on_accent(fill),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A control's accessible name with its keys, as the hint painted on it shows them.
pub(crate) fn with_keys(name: &str, keys: &str) -> String {
    if keys.is_empty() {
        name.to_owned()
    } else {
        format!("{name} ({keys})")
    }
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
        // On the window gradient: dim, not faint, so small text stays readable (4.5:1).
        p.text_dim,
    );
    if let Some(n) = count {
        painter.text(
            pos2(rect.right() - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            n.to_string(),
            egui::FontId::proportional(11.0),
            p.text_dim,
        );
    }
}

/// Clicks and drags without taking keyboard focus: window drag strips and the click catchers
/// behind floating cards, which are not controls.
const WINDOW_DRAG: Sense = Sense::CLICK.union(Sense::DRAG);

/// The draggable line between Pages and Layers. Dragging moves the split.
pub(crate) fn split_handle(ui: &mut Ui, share: &mut f32) {
    let p = theme::of(ui);
    let total = ui.max_rect().height().max(1.0);
    // Pointer only (Tab skips it): the keyboard has no way to move a split.
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::DRAG);
    response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::ResizeHandle, true, "Resize Pages and Layers")
    });
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

/// A rounded field with a magnifier and a clear button: the layer filter (`fill` is
/// `Palette::field` on the gradient), the Recents filter (`Palette::well` on a card).
pub(crate) fn filter_field(ui: &mut Ui, text: &mut String, hint: &str, fill: Color32) {
    let p = theme::of(ui);
    egui::Frame::new()
        .fill(fill)
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
                let field = ui.add_sized(
                    vec2(width, 24.0),
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .vertical_align(Align::Center)
                        .frame(egui::Frame::NONE)
                        .margin(Margin::symmetric(2, 0)),
                );
                crate::a11y::name_field(&field, hint);
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
    // `Some` for a tool (a toggle screen readers hear as on or off), `None` for an action.
    active: Option<bool>,
    accent: Color32,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| match active {
        // The tools are one of a set: radio buttons.
        Some(on) => WidgetInfo::selected(WidgetType::RadioButton, true, on, label),
        None => WidgetInfo::labeled(WidgetType::Button, true, label),
    });
    let active = active.unwrap_or(false);
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
    /// Drags (and double-click maximises) the window; not a control, so Tab skips it.
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
        // No file open: Slate, or the colour of the Recents card under the pointer.
        #[cfg(not(target_arch = "wasm32"))]
        let home = self
            .recents
            .hover
            .unwrap_or(theme::Colour::Preset(theme::HOME_TINT));
        #[cfg(target_arch = "wasm32")]
        let home = theme::Colour::Preset(theme::HOME_TINT);
        let colour = self.tabs.get(self.active).map_or(home, |t| t.colour());
        let mut target = theme::space_tint(colour, dark);
        // While two spaces' pages move side by side, their colours blend with the pages.
        let neighbour = self.neighbour(self.slide).and_then(|i| self.tabs.get(i));
        let blending = neighbour.is_some();
        if let Some(other) = neighbour {
            let t = (self.slide.abs() / self.page_width.max(1.0)).clamp(0.0, 1.0);
            target = target.mix(theme::space_tint(other.colour(), dark), t);
        }
        let id = Id::new("window-tint");
        // Quick while the colour pad is being dragged, a slow crossfade otherwise.
        let picking = self
            .space_editing
            .as_ref()
            .is_some_and(|e| e.kind == crate::space_editor::Popover::Colour);
        let secs = if blending {
            0.0
        } else if picking {
            0.08
        } else {
            0.45
        };
        let fade = |n: &str, c: Color32| anim::fade(ctx, id.with(n), c, secs);
        Tint {
            top: fade("top", target.top),
            bottom: fade("bottom", target.bottom),
            glow: fade("glow", target.glow),
            accent: fade("accent", target.accent),
        }
    }

    /// Starts the sidebar slide when the active file changed since the last frame: a whole
    /// page from the side for the next or previous space, a short nudge for a far one.
    pub(crate) fn note_active_file(&mut self, ctx: &egui::Context) {
        let now = self.tabs.get(self.active).map(|t| t.id);
        if now != self.shown_file {
            if let (Some(before), Some(now)) = (self.shown_file, now) {
                let pos = |id: u64| self.tabs.iter().position(|t| t.id == id);
                let (dir, step) = match (pos(before), pos(now)) {
                    (Some(a), Some(b)) => (
                        if a > b { -1.0 } else { 1.0 },
                        if a.abs_diff(b) == 1 {
                            self.page_width
                        } else {
                            SLIDE
                        },
                    ),
                    _ => (1.0, SLIDE),
                };
                anim::reset(ctx, Id::new("space-slide"), dir * step);
                self.slide_nudge = step == SLIDE;
            }
            self.shown_file = now;
        }
    }

    /// The space shown beside the active one while the sidebar's pages move by `slide`.
    fn neighbour(&self, slide: f32) -> Option<usize> {
        sidebar_pages(
            self.active,
            self.tabs.len(),
            slide,
            self.slide_nudge,
            self.page_width,
        )
        .get(1)
        .map(|p| p.0)
    }

    /// The window controls row at the top of the sidebar (or of the title strip).
    fn window_row(&mut self, ui: &mut Ui, sidebar: bool) {
        let peek = sidebar && !self.show_left;
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            if self.traffic_lights(ui.ctx()) && !peek {
                let (_, r) = ui.allocate_exact_size(vec2(TRAFFIC_LIGHTS, 28.0), WINDOW_DRAG);
                crate::a11y::hide(&r);
                self.drag_window(ui, &r);
            }
            let (label, show) = match (sidebar, peek) {
                (true, false) => ("Hide sidebar", false),
                (true, true) => ("Keep sidebar open", true),
                (false, _) => ("Show sidebar", true),
            };
            let button = icons::button(ui, Icon::Sidebar, label, false);
            if !peek {
                self.onboarding.targets.sidebar = Some(button.rect);
            }
            if button.clicked() {
                self.show_left = show;
            }
            let menu = self.show_drawn_menu();
            let rest = ui.available_width() - if menu { 30.0 } else { 0.0 };
            let (rect, r) = ui.allocate_exact_size(vec2(rest.max(0.0), 28.0), WINDOW_DRAG);
            crate::a11y::hide(&r);
            self.drag_window(ui, &r);
            if !sidebar && let Some(tab) = self.tabs.get(self.active) {
                let p = theme::of(ui);
                let page = tab
                    .doc
                    .node(tab.page)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                let title = format!("{}  ·  {page}", tab.space_name());
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
        // As wide as the docked sidebar was left.
        let width = egui::containers::panel::PanelState::load(ctx, Id::new(SIDEBAR))
            .map_or(264.0, |s| s.size().x)
            .clamp(200.0, 560.0);
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
                // The card takes the pointer everywhere, gaps and padding included, so no
                // click or drag on it reaches the canvas underneath.
                crate::a11y::hide(&ui.interact(card, Id::new("peek-card"), WINDOW_DRAG));
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
        self.page_width = body.width() + PAGE_GAP;
        let ctx = ui.ctx().clone();
        let slide = match self.swipe_spaces(&ctx, max.expand(10.0)) {
            Some(offset) => offset,
            None => anim::spring(&ctx, Id::new("space-slide"), 0.0, Spring::SMOOTH),
        };
        self.slide = slide;
        if self.swipe.offset().is_some() {
            self.slide_nudge = false;
        }
        let clip = body.expand2(vec2(0.0, 4.0));
        let pages = sidebar_pages(
            self.active,
            self.tabs.len(),
            slide,
            self.slide_nudge,
            self.page_width,
        );
        for (index, x, opacity) in pages {
            // A child that takes no room in the panel: a page moved sideways must not widen
            // the sidebar (egui grows a panel to fit what its content allocates).
            let salt = ("space-page", self.tabs.get(index).map(|t| t.id));
            let mut page = ui.new_child(
                UiBuilder::new()
                    .id_salt(salt)
                    .max_rect(body.translate(vec2(x, 0.0))),
            );
            page.set_clip_rect(ui.clip_rect().intersect(clip));
            page.multiply_opacity(opacity);
            self.sidebar_body(&mut page, index);
        }
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
        self.onboarding.targets.command = Some(rect);
        // The command bar's keys as the person set them (none: no hint).
        let shortcut = self
            .prefs
            .keymap
            .get(crate::keymap::Binding::CommandBar)
            .map(|s| ui.ctx().format_shortcut(&s))
            .unwrap_or_default();
        let label = with_keys("Command bar", &shortcut);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
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
        let response = response
            .on_hover_text("Search commands, files, pages and layers, or paste a Figma link");
        if response.clicked() {
            self.command.show(ui.ctx());
        }
    }

    /// Two-finger horizontal swipes over the sidebar (`area`) move between spaces. Returns
    /// the content's offset while a swipe is on.
    fn swipe_spaces(&mut self, ctx: &egui::Context, area: Rect) -> Option<f32> {
        let now = ctx.input(|i| i.time);
        // A swipe that started over the sidebar is followed until it ends, wherever the
        // pointer goes.
        let over = self.swipe.is_on() || ctx.pointer_hover_pos().is_some_and(|p| area.contains(p));
        let wheel: Vec<(egui::Vec2, egui::TouchPhase)> = if over {
            ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::MouseWheel {
                            unit, delta, phase, ..
                        } => {
                            let scale = match unit {
                                egui::MouseWheelUnit::Point => 1.0,
                                egui::MouseWheelUnit::Line => 40.0,
                                egui::MouseWheelUnit::Page => 400.0,
                            };
                            Some((*delta * scale, *phase))
                        }
                        _ => None,
                    })
                    .collect()
            })
        } else {
            Vec::new()
        };
        let ended = self
            .swipe
            .feed_frame(&wheel, now, ctx.cumulative_frame_nr())
            .or_else(|| self.swipe.tick(now));
        let (has_previous, has_next) = (self.active > 0, self.active + 1 < self.tabs.len());
        let page = self.page_width;
        // At most a page; past the first or last space the content only gives a little.
        let shown = |offset: f32| {
            let allowed = if offset < 0.0 { has_next } else { has_previous };
            if allowed {
                offset.clamp(-page, page)
            } else {
                (offset * 0.25).clamp(-40.0, 40.0)
            }
        };
        if let Some(end) = ended {
            let at = shown(end.offset);
            // The pages carry on from where the fingers left them.
            let slide = match end.turn(page - PAGE_GAP) {
                -1 if has_next => {
                    self.active += 1;
                    at + page
                }
                1 if has_previous => {
                    self.active -= 1;
                    at - page
                }
                _ => at,
            };
            self.shown_file = self.tabs.get(self.active).map(|t| t.id);
            anim::reset(ctx, Id::new("space-slide"), slide);
            self.slide_nudge = false;
            return None;
        }
        let offset = self.swipe.offset()?;
        // A sideways swipe does not scroll the lists under it.
        ctx.input_mut(|i| {
            i.smooth_scroll_delta = egui::Vec2::ZERO;
        });
        ctx.request_repaint();
        Some(shown(offset))
    }

    /// The pages and layers of space `index`.
    fn sidebar_body(&mut self, ui: &mut Ui, index: usize) {
        let p = theme::of(ui);
        let Some(tab) = self.tabs.get_mut(index) else {
            // The Recents page: with files open (the + shows it) they are still there.
            let text = if self.tabs.is_empty() {
                "No file open"
            } else {
                "Recent files"
            };
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(text).color(p.text_dim));
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
        let fill = theme::of(ui).field;
        filter_field(ui, &mut self.layer_filter, "Filter layers", fill);
        ui.add_space(4.0);
        panels::layers(ui, tab, &self.layer_filter);
    }

    /// The bottom of the sidebar: one dot per open file in its colour, like Arc's spaces.
    fn file_switcher(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let row = ui.max_rect();
        let plus = 30.0;
        // The dots, centred when they fit; a sideways scroll when they do not, so they never
        // run under the + button.
        let area = Rect::from_min_max(row.min, pos2(row.right() - plus - 6.0, row.max.y));
        self.dots_clip = area;
        let count = self.tabs.len() + self.loading.len();
        let fits = count as f32 * DOT <= area.width();
        // The tips point at the dots themselves.
        self.onboarding.targets.dots = Some(Rect::from_center_size(
            area.center(),
            vec2((count as f32 * DOT).min(area.width()), DOT),
        ));
        let mut dots = ui.new_child(
            UiBuilder::new()
                .max_rect(area)
                .layout(Layout::left_to_right(Align::Center)),
        );
        dots.spacing_mut().item_spacing.x = 0.0;
        let mut clicked = Dots::default();
        if fits {
            dots.add_space((area.width() - count as f32 * DOT) / 2.0);
            self.dots(&mut dots, &mut clicked);
        } else {
            egui::ScrollArea::horizontal()
                .id_salt("file-dots")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(&mut dots, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        self.dots(ui, &mut clicked);
                    });
                });
        }
        let plus_at = Rect::from_center_size(
            pos2(row.right() - plus / 2.0, row.center().y),
            vec2(28.0, 28.0),
        );
        // The + shows the Recents page, a place after the last space; on that page it opens
        // a file.
        let home = self.tabs.get(self.active).is_none();
        let plus_clicked = ui
            .put(plus_at, |ui: &mut Ui| {
                icons::button(ui, Icon::Plus, "Open file…", false)
            })
            .clicked();
        if let Some(i) = clicked.pick {
            self.active = i;
        }
        if let Some((i, then)) = clicked.action {
            self.active = i;
            self.apply(then, &ctx);
        }
        if let Some(i) = clicked.close {
            self.close(i);
        }
        if plus_clicked && home {
            self.open_dialog(&ctx);
        } else if plus_clicked {
            self.show_recents();
        }
    }

    /// Opens the files a later launch handed over (a jump list item, `figstudio file.fig`)
    /// and brings this window forward. A file gone by now fails to open with the usual
    /// toast, and a listed one is marked missing.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_handed_over(&mut self, ctx: &egui::Context) {
        let Some(inbox) = &self.handed_over else {
            return;
        };
        let files = inbox.take();
        if files.is_empty() {
            return;
        }
        for file in files {
            self.open(file);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        // On Windows the window takes the foreground itself: egui's Focus sends an Alt key
        // press first, which can cancel the right the launch passed on.
        #[cfg(windows)]
        if let Some(window) = &self.window {
            window.come_forward();
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// Shows the Recents page; open files stay open, after it is the first.
    pub(crate) fn show_recents(&mut self) {
        self.recents_from = self.tabs.get(self.active).map(|t| t.id);
        self.active = self.tabs.len();
    }

    /// One dot per open file (Arc's name pill on hover, a menu on right-click; a middle
    /// click closes it) and a spinner per file still loading.
    fn dots(&mut self, ui: &mut Ui, clicked: &mut Dots) {
        let p = theme::of(ui);
        let ctx = ui.ctx().clone();
        self.space_dots.clear();
        let mut resting = false;
        let mut pill = None;
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            // The slot in the row; the dot slides into it when one before it opens or closes,
            // and is clicked, hovered and named where it is drawn, not where it is going.
            let (slot_rect, _) = ui.allocate_exact_size(vec2(DOT, DOT), Sense::hover());
            let slot = slot_rect.left() - ui.max_rect().left();
            let slid = anim::spring(&ctx, Id::new(("dot-x", tab.id)), slot, Spring::SMOOTH);
            let rect = slot_rect.translate(vec2(slid - slot, 0.0));
            let response = ui.interact(rect, Id::new(("space-dot", tab.id)), Sense::click());
            let title = tab.space_name();
            response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, active, &title));
            let (hover, press) = icons::feedback(ui, &response);
            // Every badge the same size, as Arc's; only the colour tells the active one. A new
            // file's dot pops in, and dots slide over when one before them closes.
            let appear = anim::spring_from(
                &ctx,
                Id::new(("dot-appear", tab.id)),
                1.0,
                Some(0.0),
                Spring::BOUNCY,
            );
            let scale = (1.0 + hover * 0.08 - press * 0.12) * appear.max(0.0);
            let lit = ctx.animate_bool_with_time(response.id.with("lit"), active, 0.18);
            let away = (1.0 - lit) * (1.0 - hover);
            let tint = theme::space_tint(tab.colour(), p.dark);
            let c = rect.center();
            let size = dot_size(tab.style.symbol.is_some(), scale);
            let badge = Rect::from_center_size(c, vec2(size, size));
            let colour = drained(tint.accent, away);
            paint_badge(
                ui.painter(),
                badge,
                tab.style.symbol.as_ref(),
                colour,
                colour,
            );
            self.space_dots.push((tab.id, rect));
            // Bring the active file's dot into view when it changes or the footer resizes;
            // otherwise the dots stay where the person scrolled them.
            if active && (tab.id, self.dots_clip.width()) != self.dot_in_view {
                response.scroll_to_me(None);
            }
            if response.hovered() && !response.context_menu_opened() {
                resting = true;
                if pill_due(&ctx, tab.id) {
                    let keys =
                        crate::keymap::Binding::space(i).and_then(|b| self.prefs.keymap.get(b));
                    pill = Some(PillContent {
                        dot: rect,
                        name: title.clone(),
                        keys,
                    });
                }
            }
            if response.clicked() {
                clicked.pick = Some(i);
            }
            if response.middle_clicked() {
                clicked.close = Some(i);
            }
            context_menu(&response, |ui| {
                ui.set_min_width(200.0);
                for (label, then) in [
                    ("Change space icon…", Action::SpaceIcon),
                    ("Rename space…", Action::RenameSpace),
                    ("Edit theme colour…", Action::SpaceColour),
                ] {
                    if ui.button(label).clicked() {
                        clicked.action = Some((i, then));
                    }
                }
                ui.separator();
                if ui.button("Reset space look").clicked() {
                    clicked.action = Some((i, Action::ResetSpace));
                }
                ui.separator();
                if ui.button("Close space").clicked() {
                    clicked.close = Some(i);
                }
            });
        }
        if !resting {
            ctx.data_mut(|d| d.remove::<Rest>(Id::new("space-pill-rest")));
        }
        space_pill(&ctx, pill);
        self.dot_in_view = (
            self.tabs.get(self.active).map_or(0, |t| t.id),
            self.dots_clip.width(),
        );
        for path in self.loading.iter() {
            let (rect, response) = ui.allocate_exact_size(vec2(DOT, DOT), Sense::hover());
            egui::Spinner::new().paint_at(ui, rect.shrink(7.0));
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            crate::a11y::progress(&response, &format!("Opening {name}…"));
            response.on_hover_text(format!("Opening {name}…"));
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
        let mut recent = None;
        match self.tabs.get_mut(self.active) {
            Some(tab) => self.find.update(tab, &ctx),
            None => self.find.update_without_tab(&ctx),
        }
        ui.scope_builder(UiBuilder::new().max_rect(card), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(card));
            match self.tabs.get_mut(self.active) {
                Some(_) if self.render_worker_lost => {
                    self.crossfade = Crossfade::default();
                    ui.centered_and_justified(|ui| {
                        let label = ui.label(
                            RichText::new("Rendering is unavailable; restart the app.")
                                .color(p.warning),
                        );
                        crate::a11y::announce(&label, true);
                    });
                }
                Some(tab) => {
                    let options = canvas::Options {
                        tool: self.tool,
                        space_held,
                        radius: f32::from(CARD_RADIUS),
                        frame_labels: self.prefs.frame_labels,
                        scroll_zooms: self.prefs.scroll_zooms,
                    };
                    // Another file or page fades in over the last one, painted again from the
                    // tile cache: a crossfade, never the bare card for a frame.
                    let shown = anim::arrive(&ctx, Id::new("canvas-in"), (tab.id, tab.page), 0.16);
                    let ppp = ui.pixels_per_point();
                    let under = (card, f32::from(CARD_RADIUS));
                    let underlay =
                        (self.crossfade).under(ui.painter(), &self.tiles, tab, under, shown, ppp);
                    let before = ui.opacity();
                    ui.multiply_opacity(shown);
                    cursor = canvas::show(ui, tab, &mut self.workers, &mut self.tiles, options);
                    ui.set_opacity(before);
                    // Now that the new page has planned its tiles, so the underlay's count as
                    // on screen and are not the first to go.
                    self.tiles.keep_drawn(&underlay);
                    // Only a picture shown in full is one to fade from: going back within a
                    // fade fades from what was on screen, not from a page barely drawn.
                    if shown >= 1.0 {
                        self.crossfade.last = Some(CanvasShot::of(tab));
                    }
                }
                None => {
                    // The next file to open fades in over its own page colour.
                    self.crossfade = Crossfade::default();
                    recent = self.recents_page(ui, card);
                }
            }
            ui.painter().rect_stroke(
                card,
                radius,
                Stroke::new(1.0, p.card_stroke),
                StrokeKind::Inside,
            );
            if !self.render_worker_lost && self.tabs.get(self.active).is_some() {
                let tools = self.tool_pill(ui, card, &mut actions);
                self.status_pill(ui, card, tools);
                let below = self.find_bar(ui, card);
                self.gaps_chip(ui, card, below);
            }
        });
        // A closed file's tiles go once no fade paints them.
        let (crossfade, tiles) = (&self.crossfade, &mut self.tiles);
        self.tiles_closed.retain(|&doc| {
            let needed = crossfade.paints(doc);
            if !needed {
                tiles.forget(doc);
            }
            needed
        });
        for action in actions {
            self.apply(action, &ctx);
        }
        if self.tabs.get(self.active).is_some() {
            self.recents.hidden();
        }
        match recent {
            Some(crate::recents::Pick::OpenDialog) => open_clicked = true,
            Some(pick) => self.recent_picked(&ctx, pick),
            None => {}
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
        let tools_only = pad * 2.0 + b * 3.0;
        let with_zoom = tools_only + sep + zoom_w;
        let full = with_zoom + b * 2.0;
        // A narrow card drops the − and + first (the keys and menus still zoom), then the
        // zoom percentage.
        let (zoom_shown, steps_shown) = (
            card.width() >= with_zoom + 32.0,
            card.width() >= full + 32.0,
        );
        let width = match (zoom_shown, steps_shown) {
            (_, true) => full,
            (true, false) => with_zoom,
            _ => tools_only,
        };
        let rect = Rect::from_center_size(
            pos2(card.center().x, card.bottom() - 32.0),
            vec2(width, 38.0),
        );
        let radius = CornerRadius::same(19);
        // Catches clicks between the buttons so they do not reach the canvas.
        crate::a11y::hide(&ui.interact(rect, Id::new("tool-pill"), Sense::CLICK));
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
                Some(active),
                accent,
            )
            .clicked()
            {
                actions.push(Action::SelectTool(tool));
            }
        }
        if !zoom_shown {
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
        let zoom_left = if steps_shown { out.right() } else { left };
        let zoom = Rect::from_min_size(pos2(zoom_left, rect.top() + 4.0), vec2(zoom_w, b));
        let into = Rect::from_min_size(pos2(zoom.right(), rect.top() + 4.0), vec2(b, b));
        if steps_shown
            && pill_button(
                ui,
                out,
                Id::new("zoom-out"),
                (Some(Icon::Minus), "Zoom out", "Zoom out (-)"),
                None,
                accent,
            )
            .clicked()
        {
            actions.push(Action::ZoomBy(0.8));
        }
        let percent = zoom_percent(self.tabs.get(self.active));
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
        if steps_shown
            && pill_button(
                ui,
                into,
                Id::new("zoom-in"),
                (Some(Icon::Plus), "Zoom in", "Zoom in (+)"),
                None,
                accent,
            )
            .clicked()
        {
            actions.push(Action::ZoomBy(1.25));
        }
        rect
    }

    /// What draws the canvas, the cursor, the render time and agent access, at the bottom
    /// left beside the tool pill (`tools`), or above it when the card is too narrow: it never
    /// goes away, since it holds the MCP address and its Copy token button.
    fn status_pill(&mut self, ui: &mut Ui, card: Rect, tools: Rect) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let p = theme::of(ui);
        let beside = Rect::from_min_max(
            pos2(card.left() + 12.0, card.bottom() - 47.0),
            pos2(tools.left() - 8.0, card.bottom() - 17.0),
        );
        let max = if beside.width() >= 160.0 {
            beside
        } else {
            Rect::from_min_max(
                pos2(card.left() + 12.0, tools.top() - 38.0),
                pos2(card.right() - 12.0, tools.top() - 8.0),
            )
        };
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
    fn gaps_chip(&self, ui: &mut Ui, card: Rect, top: f32) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let gaps = crate::region_png::gaps(&tab.stats);
        if gaps.is_empty() {
            return;
        }
        let p = theme::of(ui);
        let max = Rect::from_min_max(
            pos2(card.center().x, top + 12.0),
            pos2(card.right() - 12.0, top + 42.0),
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
    pub(crate) fn empty_state(&self, ui: &mut Ui, card: Rect) -> bool {
        let ctx = ui.ctx().clone();
        let p = theme::of(ui);
        let tint = self.tint_now(&ctx);
        let c = card.center() - vec2(0.0, card.height() * 0.08);
        let painter = ui.painter();
        paint_mark(painter, &p, tint, c - vec2(0.0, 92.0), 64.0);
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
        let shortcut = self
            .prefs
            .keymap
            .get(crate::keymap::Binding::Open)
            .map(|s| ctx.format_shortcut(&s))
            .unwrap_or_default();
        let label = with_keys("Open file…", &shortcut);
        let response = accent_button(ui, button, "open-file", "Open file…", &label, tint);
        let painter = ui.painter();
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
            // Two toasts can be born in the same frame: the text tells them apart.
            let id = Id::new(("toast", t.born.to_bits(), &t.text));
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
                                let label = ui.add(egui::Label::new(text).wrap());
                                crate::a11y::announce(&label, t.error);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dot_stays_inside_its_slot() {
        for symbol in [false, true] {
            for scale in [0.7, 1.0, 1.08, 1.3, 2.0] {
                let reach = dot_size(symbol, scale) / 2.0;
                assert!(reach <= DOT / 2.0, "symbol {symbol} scale {scale}: {reach}");
            }
        }
    }

    #[test]
    fn a_badge_has_no_disc_behind_its_icon_or_emoji() {
        fn circles(shape: &Shape, out: &mut Vec<f32>) {
            match shape {
                Shape::Circle(c) => out.push(c.radius),
                Shape::Vec(v) => v.iter().for_each(|s| circles(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_center_size(pos2(50.0, 50.0), vec2(20.0, 20.0));
            for symbol in [
                Symbol::Icon(crate::spaces::ICONS[0].0.into()),
                Symbol::Emoji("d".into()),
            ] {
                paint_badge(
                    ui.painter(),
                    rect,
                    Some(&symbol),
                    Color32::RED,
                    Color32::RED,
                );
            }
        });
        // Cleared: epaint asserts (in debug builds) that no texture change is dropped unseen.
        output.textures_delta.clear();
        let mut radii = Vec::new();
        for clipped in &output.shapes {
            circles(&clipped.shape, &mut radii);
        }
        assert!(
            radii.iter().all(|r| *r < 4.0),
            "a disc behind the badge: {radii:?}"
        );
    }

    #[test]
    fn an_inactive_space_s_colour_is_drained_and_faded() {
        let accent = Color32::from_rgb(230, 60, 40);
        assert_eq!(drained(accent, 0.0), accent, "the active one as it is");
        let away = drained(accent, 1.0);
        let spread = |c: Color32| c.r().max(c.g()).max(c.b()) - c.r().min(c.g()).min(c.b());
        assert!(spread(away) < spread(accent) / 3, "nearly grey: {away:?}");
        assert!(away.a() < 160, "faded: {away:?}");
    }

    #[test]
    fn the_gradient_covers_the_window_and_is_see_through_only_over_the_blur() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let tint = theme::tint(0, true);
        let opaque = gradient_mesh(rect, tint, 1.0);
        assert!(opaque.vertices.iter().all(|v| v.color.a() == 255));
        let bounds = opaque.calc_bounds();
        assert_eq!(bounds, rect, "covers the whole window");
        let blurred = gradient_mesh(rect, tint, theme::TRANSLUCENT_TINT);
        assert!(
            blurred
                .vertices
                .iter()
                .all(|v| (100..200).contains(&v.color.a()))
        );
    }
}

#[cfg(test)]
mod pager_tests {
    use super::*;

    #[test]
    fn a_page_turn_keeps_both_pages_until_it_settles_and_only_a_nudge_fades() {
        let (width, count) = (260.0, 3);
        // The last 20 points of a turn to the next space (the slide springs from a page).
        let end_of_turn = sidebar_pages(1, count, 20.0, false, width);
        assert_eq!(
            end_of_turn,
            vec![(1, 20.0, 1.0), (0, 20.0 - width, 1.0)],
            "the page it left is still beside it, and nothing fades"
        );
        assert_eq!(
            sidebar_pages(1, count, 0.0, false, width),
            vec![(1, 0.0, 1.0)]
        );
        // A far jump moves only a little and fades in.
        let nudge = sidebar_pages(2, count, 18.0, true, width);
        assert_eq!(nudge.len(), 1);
        assert!((nudge[0].2 - (1.0 - 0.5 * 0.85)).abs() < 1e-6, "{nudge:?}");
        // Past the last space there is nothing beside it.
        assert_eq!(sidebar_pages(2, count, -30.0, false, width).len(), 1);
    }

    #[test]
    fn the_zoom_shown_is_the_picture_s_while_it_eases() {
        let bytes = include_bytes!("../../../tests/fixtures/structure.fig");
        let doc = figstudio_fig::Document::from_bytes(bytes).expect("fixture");
        let mut tab = crate::tab::Tab::new(
            1,
            "a.fig".into(),
            std::sync::Arc::new(doc),
            None,
            std::time::Duration::ZERO,
        );
        tab.shown.zoom = 1.0;
        tab.view.zoom = 2.0;
        assert_eq!(zoom_percent(Some(&tab)), 100.0, "on its way to 200%");
        assert_eq!(zoom_percent(None), 100.0);
    }
}
