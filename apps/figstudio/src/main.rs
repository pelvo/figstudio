//! figstudio — desktop and browser app for Figma `.fig` local copies.

mod anim;
mod canvas;
mod chrome;
#[cfg(any(test, target_arch = "wasm32"))]
mod cjk;
mod command;
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
mod icons;
mod labels;
mod marquee;
#[cfg(any(test, target_arch = "wasm32"))]
mod mcp_heartbeat;
#[cfg_attr(
    all(not(feature = "mcp"), not(target_arch = "wasm32")),
    path = "mcp_off.rs"
)]
#[cfg_attr(target_arch = "wasm32", path = "mcp_web.rs")]
mod mcp_host;
#[cfg(any(target_arch = "wasm32", test))]
mod mcp_join;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "mcp_join_dialog_tests.rs"]
mod mcp_join_dialog_tests;
#[cfg(any(test, target_arch = "wasm32"))]
mod mcp_pair;
#[cfg(any(test, target_arch = "wasm32"))]
mod mcp_relay;
#[cfg(any(feature = "mcp", test, target_arch = "wasm32"))]
mod mcp_state;
#[cfg(target_arch = "wasm32")]
mod mcp_upload;
#[cfg(any(test, target_arch = "wasm32"))]
mod mcp_wire;
mod menu;
mod menu_dispatch;
mod menu_egui;
#[cfg(any(target_os = "macos", windows))]
mod menu_native;
mod menu_windows;
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
use egui::{Key, KeyboardShortcut, Modifiers};
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
    // Arc-style frameless window: the traffic lights float over the sidebar.
    #[cfg(target_os = "macos")]
    {
        options.viewport = options
            .viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    options.wgpu_options.wgpu_setup = gpu::setup();
    let run = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc, files);
            app.attach_menu(cc);
            Ok(Box::new(app))
        }),
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
    /// When it appeared and when it goes (egui time, seconds).
    born: f64,
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
    agent_window: bool,
    about_window: bool,
    #[cfg(any(target_os = "macos", windows))]
    native_menu: Option<menu_native::NativeMenu>,
    cursor: Option<egui::Pos2>,
    /// `FIGSTUDIO_PROFILE=1` prints per-frame UI time and render times to stderr.
    profile: bool,
    /// macOS: no title bar; the traffic lights sit over the sidebar (`chrome.rs`).
    frameless: bool,
    /// ⌘K: search commands, files, pages and layers (`command.rs`).
    command: command::CommandBar,
    appearance: menu::Appearance,
    reduce_motion: bool,
    /// The file the sidebar showed last frame, to slide it when the file changes.
    shown_file: Option<u64>,
    /// The canvas card from the last frame (toasts and the drop overlay sit on it).
    card_rect: Option<egui::Rect>,
    /// The hidden sidebar is peeking out over the canvas (`chrome.rs`).
    peek: bool,
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
            agent_window: false,
            about_window: false,
            #[cfg(any(target_os = "macos", windows))]
            native_menu: None,
            cursor: None,
            profile: std::env::var_os("FIGSTUDIO_PROFILE").is_some(),
            frameless: cfg!(target_os = "macos"),
            command: Default::default(),
            appearance: Default::default(),
            reduce_motion: anim::reduced(&cc.egui_ctx),
            shown_file: None,
            card_rect: None,
            peek: false,
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
            born: now,
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
                    doc_key,
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
                    let mut tab = Tab::new(id, path, doc, source, elapsed).with_doc_key(doc_key);
                    tab.tint = theme::pick_tint(&tab.title(), self.tabs.iter().map(|t| t.tint));
                    self.tabs.push(tab);
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
                    doc_key,
                } => {
                    // Made first: on a duplicate open, dropping it releases the `File`.
                    let source = self.web.source(file);
                    if self.loading.begin(&name) {
                        web::start_upload(&name, pick);
                        self.workers.decode(name, move || {
                            Ok((
                                web::decode_lazy(entries, eager)?,
                                Some(source),
                                Some(doc_key),
                            ))
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
        for action in [menu::Action::CommandBar, menu::Action::ToggleLayers] {
            if !self.native_owns(action)
                && let Some(shortcut) = action.shortcut()
                && ctx.input_mut(|i| i.consume_shortcut(&shortcut))
            {
                self.apply(action, ctx);
            }
        }
        // Ctrl+Tab / Ctrl+Shift+Tab: the next or previous open file. The Shift one first:
        // a plain Ctrl+Tab pattern also matches with Shift held.
        let (previous, next) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab),
                i.consume_key(Modifiers::CTRL, Key::Tab),
            )
        });
        let n = self.tabs.len();
        if n > 0 && (next || previous) {
            let step = if previous { n - 1 } else { 1 };
            self.active = (self.active + step) % n;
        }
        if !self.native_owns(menu::Action::Open)
            && ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::O)))
        {
            self.apply(menu::Action::Open, ctx);
        }
        if !self.native_owns(menu::Action::CloseTab)
            && ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::W)))
        {
            self.apply(menu::Action::CloseTab, ctx);
        }
        if !self.native_owns(menu::Action::ExportSelection(2.0))
            && ctx.input_mut(|i| {
                i.consume_shortcut(&KeyboardShortcut::new(
                    Modifiers::COMMAND | Modifiers::SHIFT,
                    Key::E,
                ))
            })
        {
            self.apply(menu::Action::ExportSelection(2.0), ctx);
        }
        let typing = ctx.egui_wants_keyboard_input();
        if typing || self.command.is_open() {
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
        for (pressed, action) in [
            (v, menu::Action::SelectTool(Tool::Select)),
            (h, menu::Action::SelectTool(Tool::Hand)),
            (z, menu::Action::SelectTool(Tool::Zoom)),
            (esc, menu::Action::ClearSelection),
            (one, menu::Action::ZoomFit),
            (zero, menu::Action::ActualSize),
            (
                (plus || minus) && !shift,
                menu::Action::ZoomBy(if plus { 1.25 } else { 0.8 }),
            ),
        ] {
            if pressed {
                self.apply(action, ctx);
            }
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
        self.native_events(&ctx);
        // Ahead of the shortcuts: the dialog's Escape must be consumed first.
        self.mcp.confirm(&ctx);
        self.shortcuts(&ctx);
        self.menu_windows(&ctx);
        let space_held = ctx.input(|i| i.key_down(Key::Space)) && !ctx.egui_wants_keyboard_input();
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());

        self.note_active_file(&ctx);
        // The menu's check mark follows the real setting (a host may turn motion off too).
        self.reduce_motion = anim::reduced(&ctx);
        let p = theme::palette(&ctx);
        chrome::paint_gradient(
            &ctx.layer_painter(egui::LayerId::background()),
            ctx.content_rect(),
            self.tint_now(&ctx),
        );

        // Hidden sidebar: a thin strip keeps the window controls and the way back.
        let mut strip = !self.show_left;
        egui::Panel::top("title-strip")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 0,
            }))
            .exact_size(36.0)
            .resizable(false)
            .show_separator_line(false)
            .show_collapsible(ui, &mut strip, |ui| self.title_strip(ui));
        let mut show_left = self.show_left;
        egui::Panel::left("sidebar")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                left: 10,
                right: 2,
                top: 6,
                bottom: 4,
            }))
            .resizable(true)
            // The window's left edge peeks the hidden sidebar instead (`chrome::peek_sidebar`).
            .drag_to_open(false)
            .show_separator_line(false)
            .default_size(256.0)
            .size_range(200.0..=440.0)
            .show_collapsible(ui, &mut show_left, |ui| self.sidebar(ui));
        // The sidebar's own button may have hidden it this frame; a drag past its edge too.
        self.show_left = self.show_left && show_left;

        let has_tab = !self.tabs.is_empty();
        let mut show_right = has_tab && self.show_right;
        let card = egui::Frame::new()
            .fill(p.card)
            .corner_radius(egui::CornerRadius::same(theme::CARD_RADIUS))
            .stroke(egui::Stroke::new(1.0, p.card_stroke))
            .shadow(chrome::card_shadow(&p, 16))
            .inner_margin(egui::Margin::same(14))
            .outer_margin(egui::Margin {
                left: 0,
                right: theme::GAP as i8,
                top: theme::GAP as i8,
                bottom: theme::GAP as i8,
            });
        egui::Panel::right("properties")
            .frame(card)
            .resizable(true)
            .show_separator_line(false)
            .default_size(276.0)
            .size_range(220.0..=480.0)
            .show_collapsible(ui, &mut show_right, |ui| {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.pending_copy = panels::properties(ui, tab).or(tab.pending_copy.take());
                }
            });
        if has_tab {
            self.show_right = show_right;
        }

        let mut cursor = None;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            cursor = self.canvas_card(ui, space_held);
        });
        self.cursor = cursor;
        let tab = self.tabs.get_mut(self.active);
        for (text, error) in region_png::drive(&ctx, &self.workers, &self.png, tab) {
            self.toast(&ctx, text, error);
        }
        self.peek_sidebar(&ctx);
        self.drop_overlay(&ctx, hovering_files);
        self.command_bar(&ctx);
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
    use crate::chrome::{MIN_LAYERS, MIN_PAGES, split_height};

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
