//! First-run onboarding in the Arc style: a welcome sheet on the first launch (three pages:
//! the app, spaces, the command field), then one tip at a time pointing at the command
//! field, the space dots and the sidebar button once a file is open. A tip goes away for
//! good when it is dismissed or when the person does what it shows. What was seen is kept
//! with the window state (`UiState`), and Help → Welcome tour shows everything again.

use egui::{
    Align2, Color32, CornerRadius, Id, Key, Margin, Modifiers, Pos2, Rect, RichText, Sense, Shape,
    Stroke, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::anim::{self, Spring};
use crate::chrome::{accent_button, card_shadow, paint_mark, rounded_gradient};
use crate::icons::{self, Icon};
use crate::keymap::Binding;
use crate::theme::{self, Palette, Tint};
use crate::{APP_NAME, App};

/// How long a file shows before the first tip comes (seconds), so it does not pop up while
/// the file fades in.
const TIP_DELAY: f64 = 0.8;
const SHEET: &str = "welcome-pop";
const TIP_WIDTH: f32 = 252.0;

/// What the person has seen, kept between launches. Unknown fields are ignored and missing
/// ones are `false`, so tips can be added later without losing the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Progress {
    /// The welcome sheet was finished or skipped.
    pub welcomed: bool,
    pub tip_command: bool,
    pub tip_spaces: bool,
    pub tip_sidebar: bool,
}

impl Progress {
    fn done(&self, tip: Tip) -> bool {
        match tip {
            Tip::Command => self.tip_command,
            Tip::Spaces => self.tip_spaces,
            Tip::Sidebar => self.tip_sidebar,
        }
    }

    fn learn(&mut self, tip: Tip) {
        match tip {
            Tip::Command => self.tip_command = true,
            Tip::Spaces => self.tip_spaces = true,
            Tip::Sidebar => self.tip_sidebar = true,
        }
    }
}

/// A one-time tip, in the order they are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tip {
    Command,
    Spaces,
    Sidebar,
}

impl Tip {
    pub const ALL: [Tip; 3] = [Tip::Command, Tip::Spaces, Tip::Sidebar];

    pub fn title(self) -> &'static str {
        match self {
            Tip::Command => "Search everything",
            Tip::Spaces => "One space per file",
            Tip::Sidebar => "Make room",
        }
    }

    /// The text, with the person's own shortcut when the action has one. `sidebar_hidden`:
    /// the sidebar tip says how to bring it back.
    fn body(self, keys: Option<&str>, sidebar_hidden: bool) -> String {
        match (self, keys) {
            (Tip::Sidebar, Some(k)) if sidebar_hidden => format!(
                "Click here or press {k} to show the sidebar. Point at the window's left \
                 edge to peek at it while it is hidden."
            ),
            (Tip::Sidebar, None) if sidebar_hidden => "Click here to show the sidebar. Point \
                                                       at the window's left edge to peek at \
                                                       it while it is hidden."
                .to_owned(),
            (Tip::Command, Some(k)) => format!(
                "Click here or press {k} to find pages, layers and commands, or paste a Figma \
                 link to jump to it."
            ),
            (Tip::Command, None) => "Click here to find pages, layers and commands, or paste \
                                     a Figma link to jump to it."
                .to_owned(),
            (Tip::Spaces, Some(k)) => format!(
                "Each open file has a dot here. Click one, press {k}, or swipe sideways with \
                 two fingers over the sidebar. Right-click a dot to rename or recolour it."
            ),
            (Tip::Spaces, None) => "Each open file has a dot here. Click one, or swipe \
                                    sideways with two fingers over the sidebar. Right-click a \
                                    dot to rename or recolour it."
                .to_owned(),
            (Tip::Sidebar, Some(k)) => format!(
                "Click here or press {k} to hide the sidebar. Point at the window's left edge \
                 to peek at it."
            ),
            (Tip::Sidebar, None) => "Click here to hide the sidebar. Point at the window's \
                                     left edge to peek at it."
                .to_owned(),
        }
    }

    fn binding(self) -> Binding {
        match self {
            Tip::Command => Binding::CommandBar,
            Tip::Spaces => Binding::NextSpace,
            Tip::Sidebar => Binding::ToggleSidebar,
        }
    }

    fn index(self) -> usize {
        Tip::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// Where it falls among the tips as the person meets them (one waiting for a hidden
    /// control can come first): one more than the others already learned.
    fn number(self, progress: &Progress) -> usize {
        1 + Tip::ALL
            .iter()
            .filter(|t| **t != self && progress.done(**t))
            .count()
    }
}

/// Where the tips point: the controls as drawn this frame (`None`: not on screen).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Targets {
    pub command: Option<Rect>,
    pub dots: Option<Rect>,
    pub sidebar: Option<Rect>,
}

impl Targets {
    fn get(&self, tip: Tip) -> Option<Rect> {
        match tip {
            Tip::Command => self.command,
            Tip::Spaces => self.dots,
            Tip::Sidebar => self.sidebar,
        }
    }
}

/// What the window looked like last frame, to notice what the person did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Seen {
    pub active: Option<u64>,
    pub tabs: usize,
    pub show_left: bool,
    pub command_open: bool,
}

/// The pages of the welcome sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Welcome,
    Spaces,
    Find,
}

impl Page {
    const ALL: [Page; 3] = [Page::Welcome, Page::Spaces, Page::Find];

    fn title(self) -> String {
        match self {
            Page::Welcome => format!("Welcome to {APP_NAME}"),
            Page::Spaces => "Every file is a space".to_owned(),
            Page::Find => "Find anything".to_owned(),
        }
    }

    fn body(self, keys: impl Fn(Binding) -> Option<String>) -> String {
        match self {
            Page::Welcome => "Open Figma .fig local copies and look through every page, layer \
                              and property, offline. Nothing leaves this computer unless you \
                              turn on agent access."
                .to_owned(),
            Page::Spaces => {
                let key = keys(Binding::NextSpace)
                    .map(|k| format!(", {k}"))
                    .unwrap_or_default();
                format!(
                    "Each file opens in its own space with its own colour. Switch with the \
                     dots at the bottom of the sidebar{key} or a two-finger swipe. Right-click \
                     a dot to rename or recolour it."
                )
            }
            Page::Find => {
                let key = keys(Binding::CommandBar)
                    .map(|k| format!(" ({k})"))
                    .unwrap_or_default();
                format!(
                    "The field at the top of the sidebar{key} searches pages, layers and \
                     commands. Paste a Figma link there to jump to the node in your local copy."
                )
            }
        }
    }
}

/// The welcome sheet and the tips.
#[derive(Debug, Default)]
pub(crate) struct Onboarding {
    pub progress: Progress,
    /// Off in tests (they turn it on); on in the app.
    pub enabled: bool,
    sheet_open: bool,
    page: usize,
    /// Drawn this frame by the sidebar; cleared at the start of each frame.
    pub targets: Targets,
    /// Where each tip pointed last, so one that was learned can fade out there.
    last_target: [Option<Rect>; 3],
    /// When a file was first showing with tips to give (egui time).
    tips_from: Option<f64>,
    seen: Option<Seen>,
}

impl Onboarding {
    /// Shows the welcome sheet now if `progress` says it was never seen.
    pub fn new(progress: Progress, enabled: bool) -> Self {
        Self {
            sheet_open: enabled && !progress.welcomed,
            progress,
            enabled,
            ..Self::default()
        }
    }

    /// The sheet is open or still fading out (it covers the window until it is gone).
    pub fn sheet_up(&self, ctx: &egui::Context) -> bool {
        self.sheet_open || anim::on_screen(ctx, Id::new(SHEET))
    }

    /// The sheet is open: modal. Fading out it no longer takes input.
    pub fn sheet_open(&self) -> bool {
        self.sheet_open
    }

    /// Help → Welcome tour: the sheet from its first page, and every tip again.
    pub fn show_again(&mut self, ctx: &egui::Context) {
        self.enabled = true;
        self.progress = Progress::default();
        self.sheet_open = true;
        self.page = 0;
        self.tips_from = None;
        anim::reset(ctx, Id::new(SHEET), 0.0);
    }

    /// Closes the sheet for good (finished, skipped, or the person went on to something).
    pub fn finish_welcome(&mut self) {
        if self.sheet_open {
            self.sheet_open = false;
            self.progress.welcomed = true;
        }
    }

    /// Marks tips done when the person does what they show: opening the command bar,
    /// switching spaces (the same files, another one active), or showing or hiding the
    /// sidebar.
    pub fn watch(&mut self, now: Seen) {
        if now.command_open {
            self.progress.learn(Tip::Command);
        }
        if let Some(before) = self.seen {
            if before.show_left != now.show_left {
                self.progress.learn(Tip::Sidebar);
            }
            if before.tabs == now.tabs
                && before.active.is_some()
                && now.active.is_some()
                && before.active != now.active
            {
                self.progress.learn(Tip::Spaces);
            }
        }
        self.seen = Some(now);
    }

    /// The tip to show at `time` (egui seconds): the first one not done, once the welcome
    /// is over and a file has been showing for [`TIP_DELAY`].
    /// Tips whose control is not on screen (`shown` false: the sidebar is hidden) wait;
    /// the next one that is takes its turn.
    pub fn next_tip(
        &mut self,
        has_tab: bool,
        time: f64,
        shown: impl Fn(Tip) -> bool,
    ) -> Option<Tip> {
        if !self.enabled || !self.progress.welcomed || self.sheet_open || !has_tab {
            self.tips_from = None;
            return None;
        }
        let tip = Tip::ALL
            .into_iter()
            .find(|t| !self.progress.done(*t) && shown(*t))?;
        let from = *self.tips_from.get_or_insert(time);
        (time - from >= TIP_DELAY).then_some(tip)
    }

    /// How long until the first tip is due (to wake the UI then).
    fn tip_wait(&self, time: f64) -> Option<f64> {
        self.tips_from
            .map(|from| from + TIP_DELAY - time)
            .filter(|w| *w > 0.0)
    }

    fn skip_tips(&mut self) {
        for tip in Tip::ALL {
            self.progress.learn(tip);
        }
    }
}

/// The person's keys for `binding`, as the platform writes them.
fn keys_for(app: &App, ctx: &egui::Context, binding: Binding) -> Option<String> {
    app.prefs
        .keymap
        .get(binding)
        .map(|s| ctx.format_shortcut(&s))
}

/// A word that works as a button: dim, brighter on hover.
pub(crate) fn text_button(
    ui: &mut Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    p: &Palette,
) -> bool {
    let response = ui.interact(rect, Id::new(id), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), text));
    crate::a11y::author(&response, format!("welcome.{}", crate::a11y::slug(text)));
    let (hover, _) = icons::feedback(ui, &response);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), p.hover.gamma_multiply(hover));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        p.text_dim.lerp_to_gamma(p.text, hover),
    );
    response.clicked()
}

/// A shortcut on a small rounded cap.
fn keycap(painter: &egui::Painter, p: &Palette, text: &str, right: Pos2) -> Rect {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        p.text_dim,
    );
    let w = galley.size().x + 14.0;
    let cap = Rect::from_min_size(pos2(right.x - w, right.y - 11.0), vec2(w, 22.0));
    painter.rect(
        cap,
        CornerRadius::same(5),
        p.well,
        Stroke::new(1.0, p.card_stroke),
        egui::StrokeKind::Inside,
    );
    painter.galley(cap.center() - galley.size() / 2.0, galley, p.text_dim);
    cap
}

/// The picture at the top of a sheet page, drawn around `c`.
fn illustration(
    painter: &egui::Painter,
    p: &Palette,
    tint: Tint,
    page: Page,
    c: Pos2,
    keys: Option<&str>,
) {
    match page {
        Page::Welcome => paint_mark(painter, p, tint, c, 84.0),
        Page::Spaces => {
            // The sidebar's footer, larger: a dot per space, the active one ringed.
            let bar = Rect::from_center_size(c, vec2(236.0, 56.0));
            painter.add(card_shadow(p, 18).as_shape(bar, CornerRadius::same(16)));
            painter.rect_filled(bar, CornerRadius::same(16), p.card);
            let colours = [0, 3, 5, 6].map(|i| theme::tint(i, p.dark).accent);
            let step = 40.0;
            let first = bar.left() + 34.0;
            for (i, colour) in colours.into_iter().enumerate() {
                let at = pos2(first + i as f32 * step, bar.center().y);
                if i == 1 {
                    painter.circle_filled(at, 15.0, colour.gamma_multiply(0.22));
                    painter.circle_filled(at, 10.0, colour);
                } else {
                    painter.circle_filled(at, 7.0, colour);
                }
            }
            let plus =
                Rect::from_center_size(pos2(bar.right() - 30.0, bar.center().y), vec2(16.0, 16.0));
            icons::paint(painter, Icon::Plus, plus, p.text_dim);
        }
        Page::Find => {
            // The command field, larger, with its keys.
            let field = Rect::from_center_size(c, vec2(320.0, 48.0));
            painter.add(card_shadow(p, 18).as_shape(field, CornerRadius::same(12)));
            painter.rect_filled(field, CornerRadius::same(12), p.card);
            let icon = Rect::from_center_size(pos2(field.left() + 24.0, c.y), vec2(16.0, 16.0));
            icons::paint(painter, Icon::Search, icon, p.text_dim);
            painter.text(
                pos2(field.left() + 44.0, c.y),
                Align2::LEFT_CENTER,
                "Search or paste a Figma link",
                egui::FontId::proportional(14.0),
                p.text_faint,
            );
            if let Some(k) = keys {
                keycap(painter, p, k, pos2(field.right() - 12.0, c.y));
            }
        }
    }
}

impl App {
    /// The sheet's keys, taken before any shortcut sees them: Escape skips, Enter and → go
    /// on, ← goes back.
    pub(crate) fn welcome_keys(&mut self, ctx: &egui::Context) {
        if !self.onboarding.sheet_open {
            return;
        }
        // Nothing under the sheet hears the keyboard, the wheel or a pinch: a field that
        // had focus loses it, a swipe over the sidebar does not switch spaces. Pointer moves
        // and clicks stay for the sheet's own buttons.
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        ctx.input_mut(|i| {
            i.smooth_scroll_delta = egui::Vec2::ZERO;
            i.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Ime(_)
                        | egui::Event::MouseWheel { .. }
                        | egui::Event::Zoom(_)
                        | egui::Event::Rotate(_)
                        | egui::Event::Touch { .. }
                )
            });
        });
        let (escape, enter, right, left) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
            )
        });
        let o = &mut self.onboarding;
        if escape {
            o.finish_welcome();
        } else if enter || right {
            if o.page + 1 < Page::ALL.len() {
                o.page += 1;
            } else if enter {
                o.finish_welcome();
            }
        } else if left {
            o.page = o.page.saturating_sub(1);
        }
    }

    /// The welcome sheet while it is open or fading out.
    pub(crate) fn welcome_sheet(&mut self, ctx: &egui::Context) {
        let open = self.onboarding.sheet_open;
        let pop = anim::presence(ctx, Id::new(SHEET), open);
        if !open && pop <= 0.001 {
            return;
        }
        let shown = pop.clamp(0.0, 1.0);
        let p = theme::palette(ctx);
        let screen = ctx.content_rect();
        // The dimmed window takes clicks while the sheet is up; they do nothing.
        egui::Area::new(Id::new("welcome-backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let sense = if open { Sense::CLICK } else { Sense::hover() };
                crate::a11y::hide(&ui.allocate_rect(screen, sense));
                if open {
                    crate::a11y::modal(ui);
                }
                let dim = if p.dark { 120.0 } else { 56.0 };
                ui.painter().rect_filled(
                    screen,
                    0.0,
                    Color32::from_black_alpha((dim * shown) as u8),
                );
            });
        let page_index = self.onboarding.page.min(Page::ALL.len() - 1);
        let page = Page::ALL[page_index];
        let tint = self.tint_now(ctx);
        let title = page.title();
        let body = page.body(|b| keys_for(self, ctx, b));
        let command_keys = keys_for(self, ctx, Binding::CommandBar);
        let width = (screen.width() - 48.0).clamp(300.0, 480.0);
        let inner = width - 16.0;
        let rise = (1.0 - pop) * 16.0;
        // Each page's words fade in.
        let words = anim::arrive(ctx, Id::new("welcome-page"), page_index, 0.2);
        let mut next = false;
        let mut back = false;
        let mut done = false;
        egui::Area::new(Id::new("welcome-sheet"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(screen.center() + vec2(0.0, rise))
            .fade_in(false)
            .show(ctx, |ui| {
                if !open {
                    anim::inert(ui);
                }
                ui.multiply_opacity(shown);
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .corner_radius(CornerRadius::same(18))
                    .stroke(Stroke::new(1.0, p.card_stroke))
                    .shadow(egui::Shadow {
                        offset: [0, 24],
                        blur: 64,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 160 } else { 60 }),
                    })
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(inner);
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        // The hero: the window's own gradient with the page's picture.
                        let (hero, _) = ui.allocate_exact_size(vec2(inner, 196.0), Sense::hover());
                        ui.painter().add(Shape::mesh(rounded_gradient(
                            hero,
                            12.0,
                            [tint.glow, tint.top, tint.bottom, tint.top],
                        )));
                        let c = hero.center() + vec2((1.0 - words) * 24.0, 0.0);
                        let painter = ui.painter().clone().with_clip_rect(hero);
                        // Multiplied, not set: the sheet's own fade-out (`shown`) applies too.
                        let mut faded = painter.clone();
                        faded.multiply_opacity(words);
                        illustration(&faded, &p, tint, page, c, command_keys.as_deref());
                        ui.add_space(22.0);
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            ui.vertical(|ui| {
                                ui.set_width(inner - 40.0);
                                ui.set_min_height(112.0);
                                ui.multiply_opacity(words);
                                ui.label(
                                    RichText::new(&title)
                                        .font(theme::semibold(21.0))
                                        .color(p.text),
                                );
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&body).size(13.5).color(p.text_dim),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                        ui.add_space(10.0);
                        let (footer, _) = ui.allocate_exact_size(vec2(inner, 52.0), Sense::hover());
                        // Where you are: the current page's dot is a wider capsule.
                        let mut x = footer.left() + 20.0;
                        for i in 0..Page::ALL.len() {
                            let w = anim::spring(
                                ui.ctx(),
                                Id::new(("welcome-dot", i)),
                                if i == page_index { 18.0 } else { 6.0 },
                                Spring::SMOOTH,
                            );
                            let dot =
                                Rect::from_min_size(pos2(x, footer.center().y - 3.0), vec2(w, 6.0));
                            let ink = if i == page_index {
                                p.text
                            } else {
                                p.text_faint
                            };
                            ui.painter().rect_filled(dot, CornerRadius::same(3), ink);
                            x += w + 6.0;
                        }
                        let last = page_index + 1 == Page::ALL.len();
                        let primary = Rect::from_min_size(
                            pos2(footer.right() - 16.0 - 128.0, footer.center().y - 18.0),
                            vec2(128.0, 36.0),
                        );
                        let label = if last { "Get started" } else { "Continue" };
                        if accent_button(ui, primary, ("welcome-next", label), label, label, tint)
                            .clicked()
                        {
                            if last {
                                done = true;
                            } else {
                                next = true;
                            }
                        }
                        let second = Rect::from_min_size(
                            pos2(primary.left() - 8.0 - 76.0, primary.top()),
                            vec2(76.0, 36.0),
                        );
                        if page_index == 0 {
                            done |= text_button(ui, second, "welcome-skip", "Skip", &p);
                        } else {
                            back |= text_button(ui, second, "welcome-back", "Back", &p);
                        }
                    });
            });
        let o = &mut self.onboarding;
        if done {
            o.finish_welcome();
        } else if next {
            o.page = (page_index + 1).min(Page::ALL.len() - 1);
        } else if back {
            o.page = page_index.saturating_sub(1);
        }
    }

    /// Learns from what the person did this frame and draws the tip that is due, next to
    /// what it is about.
    pub(crate) fn tips(&mut self, ctx: &egui::Context) {
        // Nothing done under the sheet counts as learned (it is modal; a menu that closes
        // it is the person moving on, seen from the next frame).
        if !self.onboarding.sheet_up(ctx) {
            self.onboarding.watch(Seen {
                active: self.tabs.get(self.active).map(|t| t.id),
                tabs: self.tabs.len(),
                show_left: self.show_left,
                command_open: self.command.is_open(),
            });
        }
        let time = ctx.input(|i| i.time);
        let targets = self.onboarding.targets;
        let due = self
            .onboarding
            .next_tip(self.tabs.get(self.active).is_some(), time, |t| {
                targets.get(t).is_some()
            });
        if let Some(wait) = self.onboarding.tip_wait(time) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        // Not over another window, nor while something is being picked or typed.
        let quiet = self.command.is_up(ctx)
            || self.settings.is_up(ctx)
            || self.space_popover_up()
            || self.onboarding.sheet_up(ctx)
            || self.link.window
            || self.render_worker_lost;
        let current = due
            .filter(|_| !quiet)
            .and_then(|tip| self.onboarding.targets.get(tip).map(|r| (tip, r)));
        if let Some((tip, rect)) = current {
            self.onboarding.last_target[tip.index()] = Some(rect);
        }
        let p = theme::palette(ctx);
        let tint = self.tint_now(ctx);
        for tip in Tip::ALL {
            let open = current.is_some_and(|(t, _)| t == tip);
            let pop = anim::presence(ctx, Id::new(("tip-pop", tip)), open);
            let Some(target) = self.onboarding.last_target[tip.index()] else {
                continue;
            };
            if !open && pop <= 0.001 {
                continue;
            }
            let keys = keys_for(self, ctx, tip.binding());
            match self.tip_card(ctx, &p, tint, tip, target, pop, open, keys.as_deref()) {
                Some(TipChoice::GotIt) => self.onboarding.progress.learn(tip),
                Some(TipChoice::SkipAll) => self.onboarding.skip_tips(),
                None => {}
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one card's look and place; a struct would only rename them"
    )]
    fn tip_card(
        &self,
        ctx: &egui::Context,
        p: &Palette,
        tint: Tint,
        tip: Tip,
        target: Rect,
        pop: f32,
        open: bool,
        keys: Option<&str>,
    ) -> Option<TipChoice> {
        let shown = pop.clamp(0.0, 1.0);
        let screen = ctx.content_rect();
        // To the right of the control, its arrow at the control's middle: near the card's
        // top for a control in the window's upper half, near its bottom otherwise, so the
        // card is in place from its first frame (before its height is known).
        let x = target.right() + 14.0 + (1.0 - pop) * 10.0;
        let y = target.center().y;
        let (pivot, anchor) = if y < screen.center().y {
            (Align2::LEFT_TOP, pos2(x, y - 28.0))
        } else {
            (Align2::LEFT_BOTTOM, pos2(x, y + 28.0))
        };
        let mut choice = None;
        let area = egui::Area::new(Id::new(("tip", tip)))
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(anchor)
            .constrain_to(screen.shrink(8.0))
            .fade_in(false)
            .show(ctx, |ui| {
                if !open {
                    anim::inert(ui);
                }
                ui.multiply_opacity(shown);
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .corner_radius(CornerRadius::same(12))
                    .stroke(Stroke::new(1.0, p.card_stroke))
                    .shadow(egui::Shadow {
                        offset: [0, 10],
                        blur: 30,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 140 } else { 45 }),
                    })
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(TIP_WIDTH - 28.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        ui.label(
                            RichText::new(format!(
                                "Tip {} of {}",
                                tip.number(&self.onboarding.progress),
                                Tip::ALL.len()
                            ))
                            .size(11.0)
                            .color(tint.accent),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(tip.title())
                                .font(theme::semibold(14.0))
                                .color(p.text),
                        );
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(tip.body(keys, !self.show_left))
                                    .size(12.5)
                                    .color(p.text_dim),
                            )
                            .wrap(),
                        );
                        ui.add_space(12.0);
                        let (row, _) =
                            ui.allocate_exact_size(vec2(TIP_WIDTH - 28.0, 30.0), Sense::hover());
                        let got = Rect::from_min_size(
                            pos2(row.right() - 76.0, row.top()),
                            vec2(76.0, 30.0),
                        );
                        if accent_button(ui, got, ("tip-got-it", tip), "Got it", "Got it", tint)
                            .clicked()
                        {
                            choice = Some(TipChoice::GotIt);
                        }
                        let skip = Rect::from_min_size(row.min - vec2(6.0, 0.0), vec2(80.0, 30.0));
                        if text_button(ui, skip, ("tip-skip", tip), "Skip tips", p) {
                            choice = Some(TipChoice::SkipAll);
                        }
                    });
            });
        // The arrow on the card's left edge, pointing at the control.
        let card = area.response.rect;
        let y = target.center().y.clamp(
            card.top() + 16.0,
            (card.bottom() - 16.0).max(card.top() + 16.0),
        );
        let tip_at = pos2(card.left() - 7.0, y);
        let fill = ctx.global_style().visuals.window_fill;
        let painter = ctx.layer_painter(area.response.layer_id);
        let arrow = vec![
            tip_at,
            pos2(card.left() + 1.0, y - 8.0),
            pos2(card.left() + 1.0, y + 8.0),
        ];
        painter.add(Shape::convex_polygon(
            arrow,
            fill.gamma_multiply(shown),
            Stroke::NONE,
        ));
        painter.line_segment(
            [tip_at, pos2(card.left() + 0.5, y - 8.0)],
            Stroke::new(1.0, p.card_stroke.gamma_multiply(shown)),
        );
        painter.line_segment(
            [tip_at, pos2(card.left() + 0.5, y + 8.0)],
            Stroke::new(1.0, p.card_stroke.gamma_multiply(shown)),
        );
        choice
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TipChoice {
    GotIt,
    SkipAll,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(active: Option<u64>, tabs: usize, show_left: bool) -> Seen {
        Seen {
            active,
            tabs,
            show_left,
            command_open: false,
        }
    }

    #[test]
    fn a_first_launch_shows_the_sheet_and_a_finished_one_does_not() {
        let fresh = Onboarding::new(Progress::default(), true);
        assert!(fresh.sheet_open);
        let welcomed = Progress {
            welcomed: true,
            ..Progress::default()
        };
        assert!(!Onboarding::new(welcomed, true).sheet_open);
        // Tests and hosts that turn it off never see it.
        assert!(!Onboarding::new(Progress::default(), false).sheet_open);
    }

    #[test]
    fn tips_wait_for_the_welcome_a_file_and_the_delay_then_come_in_order() {
        let all = |_| true;
        let mut o = Onboarding::new(Progress::default(), true);
        assert_eq!(o.next_tip(true, 0.0, all), None, "the sheet is still open");
        o.finish_welcome();
        assert!(o.progress.welcomed);
        assert_eq!(o.next_tip(false, 1.0, all), None, "no file open");
        assert_eq!(
            o.next_tip(true, 2.0, all),
            None,
            "the file only just showed"
        );
        assert_eq!(o.next_tip(true, 2.1 + TIP_DELAY, all), Some(Tip::Command));
        o.progress.learn(Tip::Command);
        assert_eq!(o.next_tip(true, 3.0, all), Some(Tip::Spaces));
        o.progress.learn(Tip::Spaces);
        assert_eq!(o.next_tip(true, 3.0, all), Some(Tip::Sidebar));
        o.skip_tips();
        assert_eq!(o.next_tip(true, 4.0, all), None);
    }

    #[test]
    fn a_tip_whose_control_is_hidden_waits_and_the_next_one_shows() {
        let mut o = Onboarding::new(Progress::default(), true);
        o.finish_welcome();
        let sidebar_hidden = |t| t == Tip::Sidebar;
        assert_eq!(
            o.next_tip(true, 0.0, sidebar_hidden),
            None,
            "the delay first"
        );
        assert_eq!(
            o.next_tip(true, 1.0 + TIP_DELAY, sidebar_hidden),
            Some(Tip::Sidebar)
        );
        o.progress.learn(Tip::Sidebar);
        assert_eq!(o.next_tip(true, 2.0 + TIP_DELAY, sidebar_hidden), None);
        assert_eq!(
            o.next_tip(true, 2.0 + TIP_DELAY, |_| true),
            Some(Tip::Command),
            "the sidebar back: the waiting tips come"
        );
    }

    #[test]
    fn doing_the_thing_a_tip_shows_marks_it_learned() {
        let mut o = Onboarding::new(Progress::default(), true);
        o.watch(seen(Some(1), 2, true));
        // Opening another file changes the active one but is not a switch.
        o.watch(seen(Some(3), 3, true));
        assert!(!o.progress.tip_spaces);
        o.watch(seen(Some(1), 3, true));
        assert!(o.progress.tip_spaces);
        assert!(!o.progress.tip_sidebar);
        o.watch(seen(Some(1), 3, false));
        assert!(o.progress.tip_sidebar);
        assert!(!o.progress.tip_command);
        o.watch(Seen {
            command_open: true,
            ..seen(Some(1), 3, false)
        });
        assert!(o.progress.tip_command);
    }

    #[test]
    fn the_first_frame_learns_nothing_from_the_starting_layout() {
        let mut o = Onboarding::new(Progress::default(), true);
        o.watch(seen(Some(1), 1, false));
        assert_eq!(o.progress, Progress::default());
    }

    #[test]
    fn progress_survives_older_and_newer_saved_states() {
        // Saved before onboarding existed: nothing seen.
        let old: Progress = serde_json::from_str("{}").expect("empty");
        assert_eq!(old, Progress::default());
        // Saved by a later version with a tip this one does not know: the rest is kept.
        let newer: Progress =
            serde_json::from_str(r#"{"welcomed":true,"tip_spaces":true,"tip_future":true}"#)
                .expect("newer");
        assert!(newer.welcomed && newer.tip_spaces && !newer.tip_command);
    }

    #[test]
    fn show_again_resets_everything_and_opens_the_sheet() {
        let ctx = egui::Context::default();
        let mut o = Onboarding::new(
            Progress {
                welcomed: true,
                tip_command: true,
                tip_spaces: true,
                tip_sidebar: true,
            },
            false,
        );
        o.page = 2;
        o.show_again(&ctx);
        assert!(o.enabled && o.sheet_open && o.page == 0);
        assert_eq!(o.progress, Progress::default());
    }

    #[test]
    fn every_tip_and_page_has_words_with_and_without_keys() {
        for tip in Tip::ALL {
            assert!(!tip.title().is_empty());
            for hidden in [false, true] {
                assert!(tip.body(Some("⌘K"), hidden).contains("⌘K"));
                assert!(!tip.body(None, hidden).contains("press"));
            }
        }
        for page in Page::ALL {
            assert!(!page.title().is_empty());
            assert!(!page.body(|_| None).is_empty());
        }
        assert!(Page::Find.body(|_| Some("⌘L".into())).contains("⌘L"));
    }
}
