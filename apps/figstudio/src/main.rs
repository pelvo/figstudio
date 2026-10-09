//! figstudio — desktop app for Figma `.fig` local copies.

mod canvas;
mod panels;
mod state;
mod tab;
mod theme;
mod worker;

use std::path::PathBuf;

use canvas::Tool;
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Sense, vec2};
use state::{OpenAction, Pending, Poll, classify, open_action};
use tab::Tab;
use worker::{Loaded, Workers};

const APP_NAME: &str = "figstudio";

fn main() -> eframe::Result {
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id(APP_NAME)
            .with_inner_size([1400.0, 880.0])
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, files)))),
    )
}

struct Toast {
    text: String,
    error: bool,
    until: f64,
}

struct App {
    workers: Workers,
    render_worker_lost: bool,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    tool: Tool,
    loading: Pending,
    /// A background worker is gone; reported once so the toast does not repeat every frame.
    worker_lost: state::Once,
    toasts: Vec<Toast>,
    layer_filter: String,
    show_left: bool,
    show_right: bool,
    cursor: Option<egui::Pos2>,
    /// `FIGSTUDIO_PROFILE=1` prints per-frame UI time and render times to stderr.
    profile: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        theme::install_fonts(&cc.egui_ctx);
        let workers = Workers::new(&cc.egui_ctx);
        let mut app = App {
            workers,
            render_worker_lost: false,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            tool: Tool::Select,
            loading: Pending::default(),
            worker_lost: state::Once::default(),
            toasts: Vec::new(),
            layer_filter: String::new(),
            show_left: true,
            show_right: true,
            cursor: None,
            profile: std::env::var_os("FIGSTUDIO_PROFILE").is_some(),
        };
        for f in files {
            app.open(f);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        match open_action(
            &self.tabs.iter().map(|t| &t.path).collect::<Vec<_>>(),
            &mut self.loading,
            &path,
        ) {
            OpenAction::Focus(i) => self.active = i,
            OpenAction::Load => self.workers.load(path),
            OpenAction::Ignore => {}
        }
    }

    fn open_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("Figma file", &["fig"])
            .pick_files()
        {
            for f in files {
                self.open(f);
            }
        }
    }

    /// Tells the user a background worker died. Once only: `poll` runs every frame and a dead
    /// channel stays dead.
    fn report_worker_lost(&mut self, ctx: &egui::Context) {
        if self.worker_lost.first_time() {
            self.toast(ctx, "A background worker stopped; reopen the file.", true);
        }
    }

    fn toast(&mut self, ctx: &egui::Context, text: impl Into<String>, error: bool) {
        let now = ctx.input(|i| i.time);
        self.toasts.push(Toast {
            text: text.into(),
            error,
            until: now + if error { 8.0 } else { 3.0 },
        });
    }

    fn tab(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    fn close(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.tabs.remove(i);
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len().saturating_sub(1);
            } else if self.active > i {
                self.active -= 1;
            }
        }
    }

    fn export_png(&mut self, ctx: &egui::Context, selection_only: bool, scale: f64) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let root = if selection_only {
            tab.selection.unwrap_or(tab.page)
        } else {
            tab.page
        };
        let name = tab
            .doc
            .node(root)
            .and_then(|n| n.data.text("name"))
            .filter(|n| !n.is_empty())
            .unwrap_or("export")
            .replace(['/', '\\'], "_");
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{name}.png"))
            .save_file()
        else {
            return;
        };
        // The render and PNG encode run off the UI thread on an immutable snapshot of the
        // document. The result comes back through `poll`, which reports it once.
        let doc = tab.doc.clone();
        if let Err(e) = self.workers.export(doc, root, scale, path) {
            self.toast(ctx, e, true);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        loop {
            let msg = match classify(self.workers.load_rx.try_recv()) {
                Poll::Msg(msg) => msg,
                Poll::Empty => break,
                Poll::Disconnected => {
                    self.report_worker_lost(ctx);
                    break;
                }
            };
            match msg {
                Loaded::Ok { path, doc, elapsed } => {
                    self.loading.finish(&path);
                    let id = self.next_id;
                    self.next_id += 1;
                    self.tabs.push(Tab::new(id, path, doc, elapsed));
                    self.active = self.tabs.len() - 1;
                }
                Loaded::Err { path, message } => {
                    self.loading.finish(&path);
                    self.toast(
                        ctx,
                        format!("Could not open {}: {message}", path.display()),
                        true,
                    );
                }
            }
        }
        loop {
            match classify(self.workers.render_rx.try_recv()) {
                Poll::Msg(done) => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == done.tab) {
                        canvas::accept(ctx, tab, done);
                    }
                }
                Poll::Empty => break,
                Poll::Disconnected => {
                    self.render_worker_lost = true;
                    self.report_worker_lost(ctx);
                    break;
                }
            }
        }
        while let Ok(done) = self.workers.export_rx.try_recv() {
            match done.result {
                Ok(()) => self.toast(ctx, format!("Exported {}", done.path.display()), false),
                Err(e) => self.toast(ctx, format!("Export failed: {e}"), true),
            }
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let path = f.path().to_path_buf();
            if path.is_absolute() && path.exists() {
                self.open(path);
            } else {
                match f.bytes() {
                    Ok(bytes) => match open_action(
                        &self.tabs.iter().map(|t| &t.path).collect::<Vec<_>>(),
                        &mut self.loading,
                        &path,
                    ) {
                        OpenAction::Load => self.workers.load_bytes(path, bytes),
                        OpenAction::Focus(i) => self.active = i,
                        OpenAction::Ignore => {}
                    },
                    Err(e) => self.toast(ctx, format!("Could not read dropped file: {e}"), true),
                }
            }
        }
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| t.until > now);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::O))) {
            self.open_dialog();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::W))) {
            self.close(self.active);
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::E,
            ))
        }) {
            self.export_png(ctx, true, 2.0);
        }
        let typing = ctx.egui_wants_keyboard_input();
        if typing {
            return;
        }
        let (v, h, z, esc, one, zero, plus, minus, shift) = ctx.input(|i| {
            (
                i.key_pressed(Key::V),
                i.key_pressed(Key::H),
                i.key_pressed(Key::Z) && i.modifiers.is_none(),
                i.key_pressed(Key::Escape),
                shifted_digit(i, Key::Num1, Some(Key::Exclamationmark)),
                shifted_digit(i, Key::Num0, None),
                i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals),
                i.key_pressed(Key::Minus),
                i.modifiers.shift,
            )
        });
        if v {
            self.tool = Tool::Select;
        }
        if h {
            self.tool = Tool::Hand;
        }
        if z {
            self.tool = Tool::Zoom;
        }
        let Some(tab) = self.tab() else { return };
        if esc {
            tab.select(None);
        }
        if one {
            tab.fit_pending = true;
        }
        if zero {
            tab.zoom_centre(1.0);
        }
        if (plus || minus) && !shift {
            let f = if plus { 1.25 } else { 0.8 };
            tab.zoom_centre(tab.view.zoom * f);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(
                        egui::Button::new("Open…").shortcut_text(
                            ui.ctx().format_shortcut(&KeyboardShortcut::new(
                                Modifiers::COMMAND,
                                Key::O,
                            )),
                        ),
                    )
                    .clicked()
                {
                    ui.close();
                    self.open_dialog();
                }
                let has_tab = !self.tabs.is_empty();
                let mut export = None;
                ui.add_enabled_ui(has_tab, |ui| {
                    ui.menu_button("Export page as PNG", |ui| {
                        for scale in [1.0, 2.0, 3.0] {
                            if ui.button(format!("@{scale:.0}x…")).clicked() {
                                ui.close();
                                export = Some((false, scale));
                            }
                        }
                    });
                });
                let has_sel = self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| t.selection.is_some());
                ui.add_enabled_ui(has_sel, |ui| {
                    ui.menu_button("Export selection as PNG", |ui| {
                        for scale in [1.0, 2.0, 3.0] {
                            let mut b = egui::Button::new(format!("@{scale:.0}x…"));
                            if scale == 2.0 {
                                b = b.shortcut_text(ui.ctx().format_shortcut(
                                    &KeyboardShortcut::new(
                                        Modifiers::COMMAND | Modifiers::SHIFT,
                                        Key::E,
                                    ),
                                ));
                            }
                            if ui.add(b).clicked() {
                                ui.close();
                                export = Some((true, scale));
                            }
                        }
                    });
                });
                if let Some((selection_only, scale)) = export {
                    self.export_png(ui.ctx(), selection_only, scale);
                }
                ui.separator();
                if ui
                    .add_enabled(has_tab, egui::Button::new("Close tab"))
                    .clicked()
                {
                    ui.close();
                    self.close(self.active);
                }
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Zoom to fit   ⇧1").clicked() {
                    ui.close();
                    if let Some(t) = self.tab() {
                        t.fit_pending = true;
                    }
                }
                if ui.button("Actual size   ⇧0").clicked() {
                    ui.close();
                    if let Some(t) = self.tab() {
                        t.zoom_centre(1.0);
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.show_left, "Layers panel");
                ui.checkbox(&mut self.show_right, "Properties panel");
            });
            ui.menu_button("Help", |ui| {
                ui.label(
                    RichText::new(format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION"))).strong(),
                );
                ui.label("Opens Figma .fig local copies offline.");
                ui.label(
                    RichText::new("V select · H hand · Z zoom · Space drag to pan")
                        .color(theme::TEXT_DIM),
                );
                ui.label(
                    RichText::new("Ctrl+scroll or pinch to zoom · ⇧1 fit").color(theme::TEXT_DIM),
                );
            });
        });
    }

    fn tool_strip(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            for (tool, glyph, tip) in [
                (Tool::Select, "⬉", "Select (V)"),
                (Tool::Hand, "✋", "Hand (H)"),
                (Tool::Zoom, "🔍", "Zoom (Z), Alt to zoom out"),
            ] {
                let selected = self.tool == tool;
                let b = egui::Button::new(RichText::new(glyph).size(16.0))
                    .selected(selected)
                    .min_size(vec2(30.0, 30.0))
                    .frame_when_inactive(false);
                if ui.add(b).on_hover_text(tip).clicked() {
                    self.tool = tool;
                }
            }
        });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        egui::ScrollArea::horizontal()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let active = i == self.active;
                        let fill = if active {
                            theme::CHROME
                        } else {
                            theme::TAB_STRIP
                        };
                        egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin::symmetric(10, 5))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let title = RichText::new(tab.title()).color(if active {
                                        theme::TEXT
                                    } else {
                                        theme::TEXT_DIM
                                    });
                                    if ui
                                        .add(egui::Label::new(title).sense(Sense::click()))
                                        .clicked()
                                    {
                                        self.active = i;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(RichText::new("×").size(14.0))
                                                .frame_when_inactive(false),
                                        )
                                        .on_hover_text("Close")
                                        .clicked()
                                    {
                                        close = Some(i);
                                    }
                                });
                            });
                    }
                    for p in self.loading.iter() {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label(RichText::new(name).color(theme::TEXT_FAINT));
                    }
                });
            });
        if let Some(i) = close {
            self.close(i);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let Some(tab) = self.tabs.get(self.active) else {
                ui.label(RichText::new("No file open").color(theme::TEXT_FAINT));
                return;
            };
            ui.label(format!("{:.0}%", tab.view.zoom * 100.0));
            ui.separator();
            if let Some(p) = self.cursor {
                ui.label(RichText::new(format!("{:.0}, {:.0}", p.x, p.y)).color(theme::TEXT_DIM));
                ui.separator();
            }
            ui.label(RichText::new(format!("{} layers drawn · {:.0} ms", tab.stats.nodes, tab.render_time.as_secs_f64() * 1000.0)).color(theme::TEXT_DIM));
            let notes = tab.stats.to_string();
            let gaps: Vec<&str> = notes.split(", ").filter(|s| s.contains("skipped") || s.contains("missing") || s.contains("approximated")).collect();
            if !gaps.is_empty() {
                ui.separator();
                ui.label(RichText::new(format!("⚠ {}", gaps.join(", "))).color(theme::WARNING))
                    .on_hover_text("Parts of this page use features figstudio does not draw yet, so it may differ from Figma.");
            }
        });
    }

    fn toasts(&self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new("toasts".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-16.0, -36.0))
            .show(ctx, |ui| {
                for t in &self.toasts {
                    let color = if t.error { theme::WARNING } else { theme::TEXT };
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.label(RichText::new(&t.text).color(color));
                    });
                }
            });
    }
}

/// Shown when no file is open. Returns true when "Open file…" was clicked.
fn empty_state(ui: &mut egui::Ui) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.32);
        ui.label(RichText::new(APP_NAME).size(28.0).color(theme::TEXT_DIM));
        ui.add_space(8.0);
        ui.label(RichText::new("Drop a .fig file here, or").color(theme::TEXT_FAINT));
        ui.add_space(6.0);
        clicked = ui.button(RichText::new("Open file…").size(15.0)).clicked();
        ui.add_space(10.0);
        ui.label(
            RichText::new("Figma → File → Save local copy… gives you a .fig")
                .color(theme::TEXT_FAINT)
                .small(),
        );
    });
    clicked
}

/// Shift+digit. The logical key depends on the keyboard layout (Shift+1 is `!` on US layouts),
/// so match the physical key too.
fn shifted_digit(i: &egui::InputState, digit: Key, shifted: Option<Key>) -> bool {
    i.events.iter().any(|e| match e {
        egui::Event::Key {
            key,
            physical_key,
            pressed: true,
            modifiers,
            ..
        } => {
            (modifiers.shift && (*key == digit || *physical_key == Some(digit)))
                || shifted == Some(*key)
        }
        _ => false,
    })
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        self.frame_ui(ui);
        if self.profile {
            let tab = self.tabs.get(self.active);
            eprintln!(
                "profile ui {:.1} ms; last render {:.1} ms, {} drawn, {} culled",
                started.elapsed().as_secs_f64() * 1000.0,
                tab.map_or(0.0, |t| t.render_time.as_secs_f64() * 1000.0),
                tab.map_or(0, |t| t.stats.nodes),
                tab.map_or(0, |t| t.stats.culled),
            );
        }
    }
}

impl App {
    fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.shortcuts(&ctx);
        let space_held = ctx.input(|i| i.key_down(Key::Space)) && !ctx.egui_wants_keyboard_input();
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());

        let bar = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(6, 2))
        };
        egui::Panel::top("menu")
            .frame(bar(theme::CHROME))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("tabs")
            .frame(egui::Frame::new().fill(theme::TAB_STRIP))
            .exact_size(28.0)
            .show(ui, |ui| self.tab_strip(ui));
        egui::Panel::bottom("status")
            .frame(bar(theme::CHROME))
            .exact_size(24.0)
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tools")
            .frame(egui::Frame::new().fill(theme::CHROME))
            .exact_size(42.0)
            .show(ui, |ui| self.tool_strip(ui));

        let has_tab = !self.tabs.is_empty();
        let dock = egui::Frame::new()
            .fill(theme::DOCK)
            .inner_margin(egui::Margin::same(8))
            .stroke(egui::Stroke::new(1.0, theme::SEPARATOR));
        if has_tab && self.show_left {
            let filter = &mut self.layer_filter;
            let tab = &mut self.tabs[self.active];
            egui::Panel::left("layers")
                .frame(dock)
                .resizable(true)
                .default_size(250.0)
                .size_range(180.0..=520.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("PAGES")
                            .color(theme::TEXT_DIM)
                            .small()
                            .strong(),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("pages")
                        .max_height(120.0)
                        .show(ui, |ui| panels::pages(ui, tab));
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("LAYERS")
                                .color(theme::TEXT_DIM)
                                .small()
                                .strong(),
                        );
                        ui.add(
                            egui::TextEdit::singleline(filter)
                                .hint_text("Search")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    ui.add_space(2.0);
                    panels::layers(ui, tab, filter);
                });
        }
        if has_tab && self.show_right {
            let tab = &self.tabs[self.active];
            egui::Panel::right("properties")
                .frame(dock)
                .resizable(true)
                .default_size(260.0)
                .size_range(200.0..=480.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("PROPERTIES")
                            .color(theme::TEXT_DIM)
                            .small()
                            .strong(),
                    );
                    ui.add_space(2.0);
                    panels::properties(ui, tab);
                });
        }

        let worker_unavailable = self.render_worker_lost;
        let tool = self.tool;
        let mut cursor = None;
        let mut open_clicked = false;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0.0, theme::CANVAS);
            match self.tabs.get_mut(self.active) {
                Some(_tab) if worker_unavailable => {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Rendering is unavailable; restart the app.")
                                .color(theme::WARNING),
                        );
                    });
                }
                Some(tab) => cursor = canvas::show(ui, tab, &mut self.workers, tool, space_held),
                None => open_clicked = empty_state(ui),
            }
        });
        self.cursor = cursor;
        if open_clicked {
            self.open_dialog();
        }
        if hovering_files {
            let rect = ctx.content_rect();
            let painter =
                ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, "drop".into()));
            painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(140));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop to open",
                egui::FontId::proportional(24.0),
                theme::TEXT,
            );
        }
        self.toasts(&ctx);
        if let Some(tab) = self.tabs.get(self.active) {
            let title = format!("{} — {APP_NAME}", tab.title());
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}
