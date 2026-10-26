//! The update popup (the release notes of every newer release, and the button that installs
//! it) and what the update buttons in it and in Settings → About do. Desktop only; the check
//! and the install are in `update.rs`.

use std::path::PathBuf;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, CornerRadius, Id, Key, Layout, Margin, Modifiers, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::App;
use crate::anim::{self, Spring};
use crate::chrome::accent_button;
use crate::icons;
use crate::onboarding::text_button;
use crate::theme::{self, Palette, Tint};
use crate::update::{self, Block, Release, Route, Span, State};

/// What a click on an update button asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    Check,
    /// Show the release notes popup.
    Notes,
    Install(PathBuf),
    Open(String),
    CopyBrew,
    Restart(PathBuf),
}

/// How this copy gets a release.
#[derive(Debug)]
pub(crate) struct Offer {
    /// Settings → About's note under "Version X is available".
    pub note: String,
    /// The popup's line, which ends with a link to the release page.
    pub lead: &'static str,
    /// The accent button and what it does (none when only the release page can).
    pub action: Option<(&'static str, Ask)>,
}

/// The wording at the end of every [`Offer::lead`], which is the release page's link.
pub(crate) const RELEASE_PAGE: &str = "release page";

pub(crate) fn offer(route: &Route, release: &Release) -> Offer {
    match (route, &release.app) {
        (Route::Bundle(bundle), Some(_)) => Offer {
            note: "Downloads it from GitHub, checks it and replaces this app".to_owned(),
            lead: "Or download it yourself from the ",
            action: Some(("Download and install", Ask::Install(bundle.clone()))),
        },
        (Route::Homebrew, _) => Offer {
            note: format!("Installed with Homebrew: run {}", update::BREW_UPGRADE),
            lead: "Installed with Homebrew: copy the brew command, or download it from the ",
            action: Some(("Copy brew command", Ask::CopyBrew)),
        },
        _ => Offer {
            note: "Download it from the release page".to_owned(),
            lead: "Download it from the ",
            action: None,
        },
    }
}

impl App {
    pub(crate) fn update_ask(&mut self, ask: Ask, ctx: &egui::Context) {
        match ask {
            Ask::Check => self.updater.check(ctx),
            Ask::Notes => self.updater.notes_open = true,
            Ask::Install(bundle) => self.updater.install(bundle, ctx),
            Ask::Open(page) => {
                if let Err(e) = opener::open_browser(&page) {
                    self.toast(ctx, format!("Could not open the release page: {e}"), true);
                }
            }
            Ask::CopyBrew => {
                ctx.copy_text(update::BREW_UPGRADE.to_owned());
                self.toast(ctx, "Copied the Homebrew command", false);
            }
            Ask::Restart(bundle) => match update::relaunch(&bundle) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.toast(ctx, format!("{e:#}"), true),
            },
        }
    }

    /// Runs the check at start and takes a finished check or install. A newer release opens
    /// the popup (`update::Updater::poll`); the end of an install is a toast when neither
    /// the popup nor Settings → About shows it.
    pub(crate) fn poll_update(&mut self, ctx: &egui::Context) {
        self.updater
            .at_start(self.prefs.check_updates, &self.prefs.skip_update, ctx);
        let installing = matches!(self.updater.state, State::Installing(_));
        let Some(state) = self.updater.poll(ctx) else {
            return;
        };
        let note = match state {
            State::Installed(r, _) => Some((
                format!("figstudio {} is installed: restart to use it", r.version),
                false,
            )),
            State::Failed(why) if installing => {
                Some((format!("The update did not work: {why}"), true))
            }
            _ => None,
        };
        let shown = self.updater.notes_open
            || (self.settings.is_up(ctx) && self.settings.tab == crate::settings::Tab::About);
        if let Some((text, error)) = note
            && !shown
        {
            self.toast(ctx, text, error);
        }
    }

    /// The release notes popup, drawn as the welcome sheet's card is: the dimmed window, a
    /// card with the title, the notes in a well, a line with the
    /// release page's link, and a footer with "Don't remind me" on the left and the buttons
    /// on the right. One layer (the backdrop and the card), raised above Settings.
    pub(crate) fn update_notes(&mut self, ctx: &egui::Context) {
        let open = self.updater.notes_open;
        let pop = anim::presence(ctx, Id::new(SHEET), open);
        if !open && pop <= 0.001 {
            return;
        }
        let release = match &self.updater.state {
            State::Available(r) | State::Installing(r) | State::Installed(r, _) => r.clone(),
            // Nothing to show (a new check started, or an install failed: Settings → About
            // and a toast say why).
            _ => {
                self.updater.notes_open = false;
                return;
            }
        };
        let state = self.updater.state.clone();
        let progress = self.updater.progress();
        let offer = offer(&update::route(), &release);
        let version = release.version.to_string();
        let mut skip = self.prefs.skip_update == version;
        let shown = pop.clamp(0.0, 1.0);
        let p = theme::palette(ctx);
        let tint = self.tint_now(ctx);
        let screen = ctx.content_rect();
        let width = (screen.width() - 48.0).clamp(360.0, 560.0);
        let inner = width - 16.0;
        let rise = (1.0 - pop) * 16.0;
        let remembered = Id::new((SHEET, "height"));
        let height = ctx.data(|d| d.get_temp::<f32>(remembered)).unwrap_or(440.0);
        let escape = open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let mut ask = None;
        let mut later = false;
        let mut skip_clicked = false;
        let area = egui::Area::new(Id::new(SHEET))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .fade_in(false)
            .show(ctx, |ui| {
                if !open {
                    anim::inert(ui);
                }
                // The dimmed window takes clicks while the card is up; they do nothing.
                let sense = if open { Sense::click() } else { Sense::hover() };
                ui.allocate_rect(screen, sense);
                let dim = if p.dark { 120.0 } else { 56.0 };
                ui.painter().rect_filled(
                    screen,
                    0.0,
                    Color32::from_black_alpha((dim * shown) as u8),
                );
                let top = (screen.center().y - height / 2.0).max(screen.top() + 16.0) + rise;
                let mut card = ui.new_child(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(
                            pos2(screen.center().x - width / 2.0, top),
                            vec2(width, screen.height()),
                        ))
                        .layout(Layout::top_down(egui::Align::Min)),
                );
                card.multiply_opacity(shown);
                let drawn = egui::Frame::new()
                    .fill(card.visuals().window_fill)
                    .corner_radius(CornerRadius::same(18))
                    .stroke(Stroke::new(1.0, p.card_stroke))
                    .shadow(egui::Shadow {
                        offset: [0, 24],
                        blur: 64,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 160 } else { 60 }),
                    })
                    .inner_margin(Margin::same(8))
                    .show(&mut card, |ui| {
                        ui.set_width(inner);
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        // The title's line box has room above its capitals: less space here keeps
                        // the top inset matching the sides (28 points with the frame's margin).
                        ui.add_space(INSET - 6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(INSET);
                            ui.vertical(|ui| {
                                ui.set_width(inner - 2.0 * INSET);
                                ui.label(
                                    RichText::new(format!("figstudio {version} is available"))
                                        .font(theme::semibold(21.0))
                                        .color(p.text),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(format!("You have {}", update::current()))
                                        .size(13.5)
                                        .color(p.text_dim),
                                );
                                ui.add_space(20.0);
                                let notes_height = (screen.height() - 420.0).clamp(140.0, 340.0);
                                notes_well(ui, &p, &release.notes, notes_height);
                                ui.add_space(16.0);
                                match &state {
                                    State::Installing(_) => {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.add_space(8.0);
                                            let text = match progress {
                                                Some(f) if f < 1.0 => {
                                                    format!("Downloading… {:.0}%", f * 100.0)
                                                }
                                                _ => "Checking and replacing the app…".to_owned(),
                                            };
                                            ui.label(
                                                RichText::new(text).size(12.5).color(p.text_dim),
                                            );
                                        });
                                    }
                                    State::Installed(..) => {
                                        ui.label(
                                            RichText::new(
                                                "Installed. Restart figstudio to use it.",
                                            )
                                            .size(12.5)
                                            .color(p.text_dim),
                                        );
                                    }
                                    _ => {
                                        if release_page_line(ui, &p, tint, offer.lead) {
                                            ask = Some(Ask::Open(release.page.clone()));
                                        }
                                    }
                                }
                            });
                        });
                        ui.add_space(INSET);
                        let (footer, _) =
                            ui.allocate_exact_size(vec2(inner, 36.0 + INSET), Sense::hover());
                        if matches!(state, State::Available(_)) {
                            skip_clicked = checkbox(
                                ui,
                                &p,
                                tint,
                                pos2(footer.left() + INSET, footer.top() + 18.0),
                                &mut skip,
                                "Don't remind me of this version again",
                            );
                        }
                        // Right to left: the accent button, then Later.
                        let mut right = footer.right() - INSET;
                        let quiet = |ui: &mut Ui, text: &str, right: &mut f32| {
                            let w = text_width(ui, text, egui::FontId::proportional(13.0)) + 28.0;
                            let rect =
                                Rect::from_min_size(pos2(*right - w, footer.top()), vec2(w, 36.0));
                            *right -= w + 4.0;
                            text_button(ui, rect, (SHEET, text), text, &p)
                        };
                        let accent = |ui: &mut Ui, text: &str, right: &mut f32| {
                            let w = (text_width(ui, text, theme::semibold(14.0)) + 36.0).max(112.0);
                            let rect =
                                Rect::from_min_size(pos2(*right - w, footer.top()), vec2(w, 36.0));
                            *right -= w + 8.0;
                            accent_button(ui, rect, (SHEET, text), text, text, tint).clicked()
                        };
                        match &state {
                            State::Installing(_) => later |= quiet(ui, "Hide", &mut right),
                            State::Installed(_, bundle) => {
                                if accent(ui, "Restart now", &mut right) {
                                    ask = Some(Ask::Restart(bundle.clone()));
                                }
                                later |= quiet(ui, "Later", &mut right);
                            }
                            _ => {
                                if let Some((text, then)) = offer.action.clone()
                                    && accent(ui, text, &mut right)
                                {
                                    ask = Some(then);
                                }
                                later |= quiet(ui, "Later", &mut right);
                            }
                        }
                    });
                drawn.response.rect.height()
            });
        let drawn = area.inner;
        if (drawn - height).abs() > 0.5 {
            ctx.data_mut(|d| d.insert_temp(remembered, drawn));
            ctx.request_repaint();
        }
        // Above Settings, which stays where it is while this is open (`settings_window`).
        if open {
            ctx.move_to_top(area.response.layer_id);
        }
        if skip_clicked {
            self.prefs.skip_update = if skip { version } else { String::new() };
        }
        if escape || later {
            self.updater.notes_open = false;
        }
        if let Some(ask) = ask {
            self.update_ask(ask, ctx);
        }
    }
}

/// The popup's id (its layer, its animation, its buttons).
const SHEET: &str = "update-notes";

/// Inside the card's 8-point margin: to the text, the checkbox and the buttons on every
/// side (28 points from the card's edge).
const INSET: f32 = 20.0;

fn text_width(ui: &Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
        .size()
        .x
}

/// `lead` followed by the release page's link and a full stop, wrapping as one sentence.
/// Returns whether the link was clicked.
fn release_page_line(ui: &mut Ui, p: &Palette, tint: Tint, lead: &str) -> bool {
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        let size = 13.0;
        ui.label(RichText::new(lead).size(size).color(p.text_dim));
        // In the space's accent and underlined, so it reads as a link before the pointer
        // finds it; brighter under the pointer (egui's `link` takes the label colour here
        // instead of its own).
        let hover_id = Id::new((SHEET, "release-page-hovered"));
        let hovered = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(hover_id))
            .unwrap_or(false);
        let colour = if hovered {
            tint.accent.lerp_to_gamma(Color32::WHITE, 0.3)
        } else {
            tint.accent
        };
        let text = RichText::new(RELEASE_PAGE)
            .size(size)
            .color(colour)
            .underline();
        let response = ui.add(
            egui::Label::new(text)
                .selectable(false)
                .sense(Sense::click()),
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, true, RELEASE_PAGE));
        if response.hovered() != hovered {
            ui.ctx()
                .data_mut(|d| d.insert_temp(hover_id, response.hovered()));
            ui.ctx().request_repaint();
        }
        clicked = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
        ui.label(RichText::new(".").size(size).color(p.text_dim));
    });
    clicked
}

/// A small rounded checkbox with its label to the right, starting at `left` (vertically
/// centred there), in the app's accent when on. Returns whether it was clicked.
fn checkbox(
    ui: &mut Ui,
    p: &Palette,
    tint: Tint,
    left: egui::Pos2,
    on: &mut bool,
    label: &str,
) -> bool {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(12.5),
        p.text_dim,
    );
    let size = 16.0;
    let rect = Rect::from_min_size(
        pos2(left.x, left.y - size / 2.0),
        vec2(size + 8.0 + galley.size().x, size),
    );
    let response = ui.interact(rect.expand(4.0), Id::new((SHEET, "skip")), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, *on, label));
    if response.clicked() {
        *on = !*on;
    }
    let (hover, _) = icons::feedback(ui, &response);
    let t = anim::spring(
        ui.ctx(),
        response.id.with("on"),
        if *on { 1.0 } else { 0.0 },
        Spring::SMOOTH,
    )
    .clamp(0.0, 1.0);
    let bx = Rect::from_min_size(rect.min, vec2(size, size));
    let fill = p.well.lerp_to_gamma(tint.accent, t);
    let edge = p
        .card_stroke
        .lerp_to_gamma(p.text_dim, hover * 0.5)
        .lerp_to_gamma(tint.accent, t);
    ui.painter().rect(
        bx,
        CornerRadius::same(4),
        fill,
        Stroke::new(1.0, edge),
        StrokeKind::Inside,
    );
    if t > 0.01 {
        let ink = theme::on_accent(tint.accent).gamma_multiply(t);
        let c = bx.center();
        ui.painter().line(
            vec![
                c + vec2(-3.6, 0.2),
                c + vec2(-1.0, 2.8),
                c + vec2(3.8, -2.6),
            ],
            Stroke::new(1.8, ink),
        );
    }
    ui.painter().galley(
        pos2(bx.right() + 8.0, left.y - galley.size().y / 2.0),
        galley,
        p.text_dim.lerp_to_gamma(p.text, hover),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// The release notes in a well that scrolls: each release's heading, its groups as small
/// titles, its changes as bulleted lines with the scope in the text colour.
fn notes_well(ui: &mut Ui, p: &Palette, notes: &[Block], max_height: f32) {
    egui::Frame::new()
        .fill(p.well)
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, p.card_stroke))
        .inner_margin(Margin {
            left: 18,
            right: 10,
            top: 16,
            bottom: 16,
        })
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(max_height)
                // Inside the text column the space left looks short; long notes still get
                // the whole well (short ones shrink to fit).
                .min_scrolled_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    if notes.is_empty() {
                        ui.label(
                            RichText::new("This release has no notes.")
                                .size(13.0)
                                .color(p.text_dim),
                        );
                    }
                    for (i, block) in notes.iter().enumerate() {
                        match block {
                            Block::Version(text) => {
                                if i > 0 {
                                    ui.add_space(20.0);
                                }
                                ui.label(
                                    RichText::new(text)
                                        .font(theme::semibold(14.5))
                                        .color(p.text),
                                );
                                ui.add_space(4.0);
                            }
                            Block::Section(text) => {
                                ui.add_space(14.0);
                                ui.label(
                                    RichText::new(text)
                                        .font(theme::semibold(12.0))
                                        .color(p.text_dim),
                                );
                                ui.add_space(6.0);
                            }
                            Block::Item(spans) => {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                                    ui.label(RichText::new("•").size(13.5).color(p.text_faint));
                                    ui.add(egui::Label::new(spans_job(ui, p, spans)).wrap());
                                });
                                ui.add_space(8.0);
                            }
                            Block::Text(spans) => {
                                ui.add(egui::Label::new(spans_job(ui, p, spans)).wrap());
                                ui.add_space(6.0);
                            }
                        }
                    }
                });
        });
}

/// A change's words: the bold scope in the text colour, the rest dimmer.
fn spans_job(ui: &Ui, p: &Palette, spans: &[Span]) -> LayoutJob {
    let mut job = LayoutJob::default();
    for span in spans {
        let (font, color) = if span.strong {
            (theme::semibold(13.5), p.text)
        } else {
            (egui::FontId::proportional(13.5), p.text_dim)
        };
        job.append(&span.text, 0.0, TextFormat::simple(font, color));
    }
    job.wrap.max_width = ui.available_width();
    job
}
