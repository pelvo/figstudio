//! figstudio — desktop and browser app for Figma `.fig` local copies.

mod canvas;
#[cfg(any(test, target_arch = "wasm32"))]
mod cjk;
mod copy;
mod copy_menu;
mod gpu;
mod gpu_common;
#[cfg(any(test, target_arch = "wasm32"))]
mod gpu_credits;
mod gpu_registry;
#[cfg(any(test, target_arch = "wasm32"))]
mod gpu_scenes;
#[cfg(not(target_arch = "wasm32"))]
mod gpu_tiles;
#[cfg(target_arch = "wasm32")]
mod gpu_tiles_web;
mod labels;
mod marquee;
#[cfg_attr(
    all(not(feature = "mcp"), not(target_arch = "wasm32")),
    path = "mcp_off.rs"
)]
#[cfg_attr(target_arch = "wasm32", path = "mcp_web.rs")]
mod mcp_host;
#[cfg(any(feature = "mcp", test, target_arch = "wasm32"))]
mod mcp_state;
#[cfg(target_arch = "wasm32")]
mod mcp_upload;
#[cfg(any(test, target_arch = "wasm32"))]
mod mcp_wire;
mod panels;
mod recordings;
mod region_copy;
mod region_png;
mod source;
mod state;
mod tab;
mod theme;
mod tile_cache;
mod tile_queue;
mod tiles;
mod tiles_view;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod ui_tests;
#[cfg(target_arch = "wasm32")]
mod web;
mod worker;

use std::path::PathBuf;

use canvas::Tool;
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Sense, vec2};
use state::{OpenAction, Pending, Poll, classify, open_action};
use tab::Tab;
use worker::{Loaded, Workers};

const APP_NAME: &str = "figstudio";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    init_tracing();
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id(APP_NAME)
            .with_inner_size([1400.0, 880.0])
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    options.wgpu_options.wgpu_setup = gpu::setup();
    let run = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, files)))),
    );
    // No usable GPU adapter (or no window) ends here: say why instead of a bare Debug dump.
    match run {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{APP_NAME}: could not start the window: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Renders `root` for export, reading every image it needs at full sharpness.
fn export_frame(
    images: figstudio_render::images::Images,
    doc: &figstudio_fig::Document,
    source: Option<&source::Source>,
    root: figstudio_fig::Guid,
    scale: f64,
) -> Option<figstudio_render::Frame> {
    figstudio_render::Renderer::with_images(images).render_fit_complete(
        doc,
        root,
        scale,
        0.0,
        None,
        |want| source.and_then(|s| s.read(&want.entry).ok()),
    )
}

/// Logs go to stderr; stdout stays free for data. The default level is `warn`, `RUST_LOG`
/// overrides it. `FIGSTUDIO_PROFILE` also turns on the app's own `info` lines (the frame timings).
#[cfg(not(target_arch = "wasm32"))]
fn init_tracing() {
    use std::io::IsTerminal;
    use tracing_subscriber::filter::{EnvFilter, LevelFilter};
    let mut filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .from_env_lossy();
    if std::env::var_os("FIGSTUDIO_PROFILE").is_some()
        && let Ok(directive) = "figstudio=info".parse()
    {
        filter = filter.add_directive(directive);
    }
    use tracing_subscriber::prelude::*;
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal());
    // The hosted MCP server's rmcp dumps requests and responses at debug: cap it.
    #[cfg(feature = "mcp")]
    let filter = figstudio_mcp::cap_rmcp(filter, std::env::var("RUST_LOG").ok().as_deref());
    tracing_subscriber::registry()
        .with(fmt.with_filter(filter))
        .init();
}

/// The browser starts the app from JavaScript (`web::start`) once the thread pool is up.
#[cfg(target_arch = "wasm32")]
fn main() {}

struct Toast {
    text: String,
    error: bool,
    until: f64,
}

struct App {
    workers: Workers,
    tiles: tiles_view::Tiles,
    render_worker_lost: bool,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    tool: Tool,
    loading: Pending,
    /// A background worker is gone; reported once so the toast does not repeat every frame.
    worker_lost: state::Once,
    toasts: Vec<Toast>,
    /// Finished "Copy as PNG" renders (`region_png.rs`).
    png: region_png::Inbox,
    layer_filter: String,
    /// Share of the left dock's height given to the page list (the rest is layers).
    pages_share: f32,
    show_left: bool,
    show_right: bool,
    cursor: Option<egui::Pos2>,
    /// `FIGSTUDIO_PROFILE=1` prints per-frame UI time and render times to stderr.
    profile: bool,
    #[cfg(target_arch = "wasm32")]
    web: web::Inbox,
    /// The window size cap last sent (`gpu::cap_window`).
    #[cfg(not(target_arch = "wasm32"))]
    window_cap: Option<f32>,
    /// What draws the canvas tiles ("GPU (Metal)", "CPU"), for the status bar.
    renderer: String,
    /// Agent access: the MCP server, off at every launch (`mcp_host.rs`).
    mcp: mcp_host::McpHost,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        theme::install_fonts(&cc.egui_ctx);
        #[cfg(not(target_arch = "wasm32"))]
        let (workers, renderer, tiles) = {
            let mut tiles = tiles_view::Tiles::default();
            let state = cc.wgpu_render_state.as_ref();
            let env = std::env::var("FIGSTUDIO_RENDERER").ok();
            let side = state.map(|s| s.device.limits().max_texture_dimension_2d);
            let kind = state.map(|s| s.adapter.get_info().device_type);
            let choice = gpu_tiles::choose(env.as_deref(), side, kind);
            let backend = state.map(|s| s.adapter.get_info().backend);
            tiles.gpu = state.map(gpu_tiles::Registry::new);
            let gpu = gpu_tiles::TileGpu::new(choice, state);
            let label = gpu_tiles::label(choice, backend);
            (Workers::with_gpu(&cc.egui_ctx, gpu), label, tiles)
        };
        #[cfg(target_arch = "wasm32")]
        let (workers, renderer, tiles) = {
            let mut tiles = tiles_view::Tiles::default();
            let (gpu, web_gpu, label) = gpu_tiles_web::start(cc.wgpu_render_state.as_ref());
            tiles.gpu = web_gpu;
            (Workers::with_gpu(&cc.egui_ctx, gpu), label, tiles)
        };
        let mut app = App {
            workers,
            renderer,
            tiles,
            render_worker_lost: false,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            tool: Tool::Select,
            loading: Pending::default(),
            worker_lost: state::Once::default(),
            toasts: Vec::new(),
            png: Default::default(),
            layer_filter: String::new(),
            pages_share: 0.55,
            show_left: true,
            show_right: true,
            cursor: None,
            profile: std::env::var_os("FIGSTUDIO_PROFILE").is_some(),
            #[cfg(target_arch = "wasm32")]
            web: web::Inbox::new(),
            #[cfg(not(target_arch = "wasm32"))]
            window_cap: None,
            mcp: Default::default(),
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
            // The browser opens files through `web::Inbox` (no paths there).
            #[cfg(not(target_arch = "wasm32"))]
            OpenAction::Load => self.workers.load(path),
            #[cfg(target_arch = "wasm32")]
            OpenAction::Load => self.loading.finish(&path),
            OpenAction::Ignore => {}
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self, ctx: &egui::Context) {
        self.web.pick_files(ctx);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_dialog(&mut self, _ctx: &egui::Context) {
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
            let closed = self.tabs.remove(i);
            #[cfg(target_arch = "wasm32")]
            web::forget_upload(closed.id);
            self.workers.close_tab(closed.id);
            let instance = closed.doc.instance;
            if !self.tabs.iter().any(|t| t.doc.instance == instance) {
                self.workers.close_document(instance);
                self.tiles.forget(instance);
            }
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
        let (doc, source) = (tab.doc.clone(), tab.source.clone());
        #[cfg(target_arch = "wasm32")]
        {
            let file = format!("{name}.png");
            let images = self.workers.images();
            let started = self
                .workers
                .run_export(self.web.exports_tx.clone(), move || {
                    let png = export_frame(images, &doc, source.as_ref(), root, scale)
                        .ok_or_else(|| anyhow::anyhow!("nothing to export"))
                        .and_then(|frame| frame.pixmap.into_png().map_err(anyhow::Error::from));
                    (file, png)
                });
            match started {
                Ok(()) => self.toast(ctx, "Exporting…", false),
                Err(e) => self.toast(ctx, e, true),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.save_png(ctx, doc, source, root, &name, scale);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_png(
        &mut self,
        ctx: &egui::Context,
        doc: std::sync::Arc<figstudio_fig::Document>,
        source: Option<source::Source>,
        root: figstudio_fig::Guid,
        name: &str,
        scale: f64,
    ) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{name}.png"))
            .save_file()
        else {
            return;
        };
        // The render and PNG encode run off the UI thread on an immutable snapshot of the
        // document. The result comes back through `poll`, which reports it once.
        if let Err(e) = self.workers.export(doc, root, scale, path, source) {
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
                Loaded::Ok {
                    path,
                    doc,
                    source,
                    elapsed,
                    cjk,
                } => {
                    self.loading.finish(&path);
                    #[cfg(target_arch = "wasm32")]
                    match cjk {
                        theme::CjkNeed::None => {}
                        theme::CjkNeed::HangulOnly => self.web.load_hangul_font(ctx),
                        theme::CjkNeed::Shared | theme::CjkNeed::Any => {
                            self.web.load_cjk_font(ctx);
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = cjk;
                    let id = self.next_id;
                    self.next_id += 1;
                    #[cfg(target_arch = "wasm32")]
                    web::bind_upload(&path, id);
                    self.tabs.push(Tab::new(id, path, doc, source, elapsed));
                    self.active = self.tabs.len() - 1;
                }
                Loaded::Err { path, message } => {
                    self.loading.finish(&path);
                    #[cfg(target_arch = "wasm32")]
                    web::fail_upload(&path);
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
                        self.tiles.accept(ctx, tab, done);
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
        #[cfg(target_arch = "wasm32")]
        self.tiles.pump_gpu(&mut self.tabs);
        #[cfg(not(target_arch = "wasm32"))]
        while let Ok(done) = self.workers.export_rx.try_recv() {
            match done.result {
                Ok(()) => self.toast(ctx, format!("Exported {}", done.path.display()), false),
                Err(e) => self.toast(ctx, figstudio_fig::one_line(e.as_ref()), true),
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.poll_web(ctx);
        #[cfg(target_arch = "wasm32")]
        if let Some(n) = web::take_page_request()
            && let Some(tab) = self.tab()
            && let Some(&page) = tab.doc.pages.get(n)
        {
            tab.set_page(page);
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            #[cfg(target_arch = "wasm32")]
            {
                self.web.read_dropped(ctx, f);
                continue;
            }
            #[cfg(not(target_arch = "wasm32"))]
            let path = f.path().to_path_buf();
            #[cfg(not(target_arch = "wasm32"))]
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

    #[cfg(target_arch = "wasm32")]
    fn poll_web(&mut self, ctx: &egui::Context) {
        self.web.serve_ranges(ctx);
        while let Ok((variant, font)) = self.web.fonts.try_recv() {
            if variant != self.web.font_variant() {
                continue;
            }
            match font {
                Ok(bytes) => {
                    theme::add_cjk_font(ctx, bytes);
                    self.web.font_state = web::FontState::Loaded;
                }
                Err(e) => {
                    self.web.font_state = web::FontState::Failed;
                    self.toast(
                        ctx,
                        format!(
                            "Could not load the CJK font: {}",
                            figstudio_fig::one_line(e.as_ref())
                        ),
                        true,
                    );
                }
            }
        }
        while let Ok(picked) = self.web.files.try_recv() {
            match picked {
                web::Picked::Bytes(path, bytes, pick) => {
                    if self.loading.begin(&path) {
                        web::start_upload(&path, pick);
                        self.workers.load_bytes(path, bytes);
                    } else {
                        web::discard_upload(pick);
                    }
                }
                web::Picked::Lazy {
                    name,
                    file,
                    entries,
                    eager,
                    pick,
                } => {
                    // Made first: on a duplicate open, dropping it releases the `File`.
                    let source = self.web.source(file);
                    if self.loading.begin(&name) {
                        web::start_upload(&name, pick);
                        self.workers.decode(name, move || {
                            Ok((web::decode_lazy(entries, eager)?, Some(source)))
                        });
                    } else {
                        web::discard_upload(pick);
                    }
                }
                web::Picked::Failed(e) => self.toast(ctx, e, true),
            }
        }
        while let Ok((name, png)) = self.web.exports.try_recv() {
            match png.and_then(|png| web::download(&name, "image/png", &png)) {
                Ok(()) => self.toast(ctx, format!("Exported {name}"), false),
                Err(e) => self.toast(ctx, figstudio_fig::one_line(e.as_ref()), true),
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::O))) {
            self.open_dialog(ctx);
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
                    let ctx = ui.ctx().clone();
                    self.open_dialog(&ctx);
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
                #[cfg(target_arch = "wasm32")]
                ui.menu_button("CJK font", |ui| {
                    for variant in cjk::Variant::ALL {
                        let chosen = self.web.font_choice == variant;
                        if ui.radio(chosen, variant.label()).clicked() {
                            self.web.select_cjk_font(variant, ui.ctx());
                        }
                    }
                    if self.web.korean_override {
                        ui.separator();
                        ui.label(
                            RichText::new("Korean (KR) in use: a file has only Hangul")
                                .color(theme::TEXT_DIM),
                        );
                    }
                });
                self.mcp.menu(ui);
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

    /// Numbers only (no names or content), for automated browser checks: see
    /// `window.figstudioStatus` in `web.rs`.
    #[cfg(target_arch = "wasm32")]
    fn status_json(&self) -> String {
        let tab = self.tabs.get(self.active);
        let s = tab.map(|t| &t.stats);
        let page = tab.and_then(|t| t.doc.pages.iter().position(|p| *p == t.page));
        format!(
            concat!(
                r#"{{"tabs":{},"loading":{},"page":{},"pages":{},"zoom":{},"drawn":{},"#,
                r#""culled":{},"images_loading":{},"images_missing":{},"render_ms":{:.1},"#,
                r#""image_cache_bytes":{},"images_arrived":{},"busy":{},"errors":{},"#,
                r#""cjk_font":"{:?}","cjk_variant":"{}","renderer":"{}","gpu_fallback_tiles":{}}}"#
            ),
            self.tabs.len(),
            self.loading.len(),
            page.map_or(-1, |p| p as i64),
            tab.map_or(0, |t| t.doc.pages.len()),
            tab.map_or(0.0, |t| t.view.zoom),
            s.map_or(0, |s| s.nodes),
            s.map_or(0, |s| s.culled),
            s.map_or(0, |s| s.images_loading),
            s.map_or(0, |s| s.missing_images),
            tab.map_or(0.0, |t| t.render_time.as_secs_f64() * 1000.0),
            self.workers.images().bytes(),
            self.workers.images_arrived(),
            self.workers.busy(),
            self.toasts.iter().filter(|t| t.error).count(),
            self.web.font_state,
            self.web.font_variant().code(),
            // A fixed name ("GPU (WebGPU)", "CPU"): no device or driver names.
            self.renderer,
            s.map_or(0, |s| s.gpu_fallback_tiles),
        )
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.mcp.status(ui);
            let Some(tab) = self.tabs.get(self.active) else {
                ui.label(RichText::new("No file open").color(theme::TEXT_FAINT));
                return;
            };
            ui.label(format!("{:.0}%", tab.view.zoom * 100.0));
            ui.separator();
            let cpu_tiles = match tab.stats.gpu_fallback_tiles {
                0 => String::new(),
                n => format!(" · {n} tiles on the CPU"),
            };
            ui.label(RichText::new(format!("{}{cpu_tiles}", self.renderer)).color(theme::TEXT_DIM))
                .on_hover_text("What draws the canvas (FIGSTUDIO_RENDERER=cpu|gpu). Tiles on the CPU use something the GPU path cannot draw yet.");
            ui.separator();
            if let Some(p) = self.cursor {
                ui.label(RichText::new(format!("{:.0}, {:.0}", p.x, p.y)).color(theme::TEXT_DIM));
                ui.separator();
            }
            ui.label(RichText::new(format!("{} layers drawn · {:.0} ms", tab.stats.nodes, tab.render_time.as_secs_f64() * 1000.0)).color(theme::TEXT_DIM));
            if tab.stats.images_loading > 0 {
                ui.separator();
                ui.label(RichText::new(format!("loading {} images…", tab.stats.images_loading)).color(theme::TEXT_DIM));
            }
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
/// Space kept for each side of the Pages/Layers split, in points.
const MIN_PAGES: f32 = 60.0;
const MIN_LAYERS: f32 = 120.0;

/// Height of the page list for `share` of `avail`, keeping both sides usable.
fn split_height(avail: f32, share: f32) -> f32 {
    let max = (avail - MIN_LAYERS).max(MIN_PAGES);
    (avail * share).clamp(MIN_PAGES, max)
}

/// The draggable line between Pages and Layers. Dragging moves the split.
fn split_handle(ui: &mut egui::Ui, share: &mut f32) {
    let total = ui.max_rect().height().max(1.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if response.dragged() {
        *share = (*share + response.drag_delta().y / total).clamp(0.05, 0.95);
    }
    let color = if response.hovered() || response.dragged() {
        theme::ACCENT
    } else {
        theme::SEPARATOR
    };
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, color),
    );
}

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
    fn on_exit(&mut self) {
        self.mcp.stop();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let started = web_time::Instant::now();
        self.frame_ui(ui);
        #[cfg(not(target_arch = "wasm32"))]
        gpu::cap_window(ui.ctx(), &mut self.window_cap);
        #[cfg(target_arch = "wasm32")]
        {
            web::cap_canvas(ui.ctx().input(|i| i.max_texture_side));
            web::publish_status(&self.status_json());
        }
        if self.profile {
            let tab = self.tabs.get(self.active);
            tracing::info!(
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
        if let Some(error) = self.mcp.frame(&mut self.tabs, &mut self.active) {
            self.toast(&ctx, error, true);
        }
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
            let pages_share = &mut self.pages_share;
            let tab = &mut self.tabs[self.active];
            egui::Panel::left("layers")
                .frame(dock)
                .resizable(true)
                .default_size(250.0)
                .size_range(180.0..=520.0)
                .show(ui, |ui| {
                    let pages_height = split_height(ui.available_height(), *pages_share);
                    ui.label(
                        RichText::new("PAGES")
                            .color(theme::TEXT_DIM)
                            .small()
                            .strong(),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("pages")
                        .auto_shrink([false, false])
                        .max_height(pages_height)
                        .show(ui, |ui| panels::pages(ui, tab));
                    split_handle(ui, pages_share);
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
            let tab = &mut self.tabs[self.active];
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
                    tab.pending_copy = panels::properties(ui, tab).or(tab.pending_copy.take());
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
                Some(tab) => {
                    cursor = canvas::show(
                        ui,
                        tab,
                        &mut self.workers,
                        &mut self.tiles,
                        tool,
                        space_held,
                    )
                }
                None => open_clicked = empty_state(ui),
            }
        });
        self.cursor = cursor;
        let tab = self.tabs.get_mut(self.active);
        for (text, error) in region_png::drive(&ctx, &self.workers, &self.png, tab) {
            self.toast(&ctx, text, error);
        }
        if open_clicked {
            self.open_dialog(&ctx);
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
        self.mcp.sync(&ctx, &self.tabs, self.active);
        if let Some(tab) = self.tabs.get(self.active) {
            let title = format!("{} — {APP_NAME}", tab.title());
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        // Browser workers cannot wake the UI (see worker.rs), so poll while they work.
        #[cfg(target_arch = "wasm32")]
        if self.workers.busy() || self.tiles.gpu_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_LAYERS, MIN_PAGES, split_height};

    #[test]
    fn pages_get_more_room_than_layers_by_default() {
        let pages = split_height(800.0, 0.55);
        assert!(pages > 800.0 - pages, "pages {pages}");
    }

    #[test]
    fn the_split_keeps_both_lists_usable() {
        assert_eq!(split_height(800.0, 0.0), MIN_PAGES);
        assert_eq!(split_height(800.0, 1.0), 800.0 - MIN_LAYERS);
        // Too short for both: pages keep their minimum.
        assert_eq!(split_height(100.0, 0.5), MIN_PAGES);
    }
}
