//! figstudio — desktop and browser app for Figma `.fig` local copies.

mod anim;
mod canvas;
mod canvas_pick;
mod canvas_text;
mod chrome;
#[cfg(any(test, target_arch = "wasm32"))]
mod cjk;
mod code_connect;
mod command;
mod copy;
mod copy_menu;
#[cfg(target_os = "macos")]
mod dock_mac;
#[cfg(target_arch = "wasm32")]
mod emoji_canvas;
#[cfg(any(target_arch = "wasm32", test))]
mod emoji_canvas_bounds;
mod find;
#[cfg(windows)]
mod foreground_win;
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
#[cfg(not(target_arch = "wasm32"))]
mod handoff;
mod icons;
mod inspect;
mod instance_pick;
#[cfg(any(windows, test))]
mod jump_list;
mod keymap;
mod labels;
mod link_open;
mod marquee;
#[cfg(any(test, target_arch = "wasm32"))]
mod mcp_backoff;
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
mod onboarding;
mod panels;
mod pick;
mod pick_cache;
mod recents;
#[cfg(not(target_arch = "wasm32"))]
mod recents_disk;
#[cfg(any(target_arch = "wasm32", test))]
mod recents_pictures;
#[cfg(target_arch = "wasm32")]
mod recents_web;
mod recordings;
mod region_clipboard;
mod region_copy;
mod region_png;
mod session;
mod session_store;
mod settings;
mod source;
mod space_editor;
mod spaces;
mod state;
mod swipe;
#[cfg(any(test, target_arch = "wasm32"))]
mod symbols;
mod tab;
mod tab_selection;
mod text_select;
mod theme;
mod tile_cache;
mod tile_queue;
mod tiles;
mod tiles_view;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod ui_tests;
#[cfg(target_os = "macos")]
mod vibrancy;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
mod web_wake;
mod worker;

use std::path::PathBuf;

use canvas::Tool;
use egui::{Key, KeyboardShortcut};
use state::{OpenAction, Pending, Poll, classify, open_action};
use tab::Tab;
use worker::{Loaded, Workers};

const APP_NAME: &str = "figstudio";

/// The window and Dock icon (512 px PNG). Without one eframe shows the egui logo.
#[cfg(not(target_arch = "wasm32"))]
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// [`ICON_PNG`] decoded; `None` (eframe's default) if it ever fails to decode.
#[cfg(not(target_arch = "wasm32"))]
fn app_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(ICON_PNG)
        .inspect_err(|error| tracing::warn!("app icon not decoded: {error}"))
        .ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    init_tracing();
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    // Before any window: the first figstudio of this person keeps a lock; a later launch
    // with files hands them to it, which opens them in its window (`handoff.rs`).
    let instance = match handoff::folder().map_or(handoff::Start::Alone, |d| handoff::claim(&d)) {
        handoff::Start::First(instance) => Some(instance),
        handoff::Start::Running(inbox) => {
            if handoff::hand_over(&inbox, &files, handoff::WAIT, handoff::STARTING) {
                return std::process::ExitCode::SUCCESS;
            }
            None
        }
        handoff::Start::Alone => None,
    };
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
    if let Some(icon) = app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    // Arc-style frameless window: the traffic lights float over the sidebar, and the window
    // is transparent so the blur behind it (`vibrancy.rs`) shows through the gradient.
    #[cfg(target_os = "macos")]
    {
        options.viewport = options
            .viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_transparent(true);
    }
    options.wgpu_options.wgpu_setup = gpu::setup();
    let run = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc, files);
            let ctx = cc.egui_ctx.clone();
            app.handed_over = instance.map(|i| i.watch(move || ctx.request_repaint()));
            #[cfg(windows)]
            {
                app.window = foreground_win::Window::of(cc);
            }
            app.attach_menu(cc);
            #[cfg(target_os = "macos")]
            {
                app.vibrancy = vibrancy::Vibrancy::attach(cc);
            }
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

#[cfg(any(test, target_arch = "wasm32"))]
mod canvas_font_order;
#[cfg(target_arch = "wasm32")]
mod canvas_fonts;

/// Configure pixels once for canvas workers and both export paths, before recording.
fn renderer(images: figstudio_render::images::Images) -> figstudio_render::Renderer {
    #[cfg(target_arch = "wasm32")]
    {
        canvas_fonts::renderer(images).1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        figstudio_render::Renderer::with_images(images)
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
    renderer(images).render_fit_complete(doc, root, scale, 0.0, None, |want| {
        source.and_then(|s| s.read(&want.entry).ok())
    })
}

/// Largest SVG export: the markup of a huge page is dropped past this (an error, not a cut file).
const MAX_SVG_EXPORT_BYTES: usize = 256 * 1024 * 1024;

/// Encodes `root` in `format` for export, reading every image it needs at full sharpness.
fn export_bytes(
    images: figstudio_render::images::Images,
    doc: &figstudio_fig::Document,
    source: Option<&source::Source>,
    root: figstudio_fig::Guid,
    format: worker::ExportFormat,
) -> anyhow::Result<Vec<u8>> {
    use anyhow::Context;
    match format {
        worker::ExportFormat::Png(scale) => export_frame(images, doc, source, root, scale)
            .context("nothing to export")?
            .pixmap
            .into_png()
            .context("encode the PNG"),
        worker::ExportFormat::Svg => {
            let svg = renderer(images)
                .render_fit_svg_complete(doc, root, 1.0, 0.0, None, MAX_SVG_EXPORT_BYTES, |want| {
                    source.and_then(|s| s.read(&want.entry).ok())
                })
                .context("nothing to export")?;
            anyhow::ensure!(
                !svg.over_limit,
                "the SVG is over 256 MiB; export a smaller layer"
            );
            Ok(svg.markup.into_bytes())
        }
    }
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

/// The window's layout, kept between launches in eframe's storage (with the window size and
/// the panel widths, which eframe and egui keep themselves).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct UiState {
    show_left: bool,
    show_right: bool,
    pages_share: f32,
    /// Settings preferences (absent in states saved before them).
    #[serde(default)]
    prefs: Prefs,
    /// The welcome sheet and tips seen (absent before them: shown once more).
    #[serde(default)]
    onboarding: onboarding::Progress,
    /// Retain existing legacy paths only while their private migration cannot commit.
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(
        rename = "workspace",
        skip_deserializing,
        skip_serializing_if = "session::Session::is_empty"
    )]
    legacy_workspace: session::Session,
}

/// Settings preferences (the rest lives in egui's options and the panels).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Prefs {
    /// macOS: the blurred, translucent window background.
    translucent: bool,
    /// The renderer for the next start.
    renderer: settings::Renderer,
    /// Names above top-level frames on the canvas.
    frame_labels: bool,
    /// A plain scroll wheel zooms (and pans with ⌘ / Ctrl).
    scroll_zooms: bool,
    /// Shortcuts changed in Settings → Shortcuts.
    keymap: keymap::Keymap,
    /// The Recents page lists files in rows instead of cards.
    recents_list: bool,
    /// Desktop access only; browser bridge/relay connection rules are separate.
    #[cfg_attr(target_arch = "wasm32", serde(skip))]
    agents: settings::AgentPrefs,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            translucent: true,
            renderer: settings::Renderer::Automatic,
            frame_labels: true,
            scroll_zooms: false,
            keymap: keymap::Keymap::default(),
            recents_list: false,
            agents: settings::AgentPrefs::default(),
        }
    }
}

const UI_STATE_KEY: &str = "figstudio.ui";

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
    /// ⌘, (`settings.rs`).
    settings: settings::Settings,
    prefs: Prefs,
    /// File > Open Figma link and pending link requests.
    link: link_open::LinkOpen,
    #[cfg(any(target_os = "macos", windows))]
    native_menu: Option<menu_native::NativeMenu>,
    /// The shortcuts the native menu shows (`None` inside: none, while recording).
    #[cfg(any(target_os = "macos", windows))]
    native_keys: Option<Option<keymap::Keymap>>,
    cursor: Option<egui::Pos2>,
    /// `FIGSTUDIO_PROFILE=1` prints per-frame UI time and render times to stderr.
    profile: bool,
    /// macOS: no title bar; the traffic lights sit over the sidebar (`chrome.rs`).
    frameless: bool,
    /// ⌘L: search commands, files, pages and layers (`command.rs`).
    command: command::CommandBar,
    appearance: menu::Appearance,
    reduce_motion: bool,
    /// The file the sidebar showed last frame, to slide it when the file changes.
    shown_file: Option<u64>,
    /// The canvas card from the last frame (toasts and the drop overlay sit on it).
    card_rect: Option<egui::Rect>,
    /// What the canvas showed, so another file or page fades in over it (`chrome.rs`).
    crossfade: chrome::Crossfade,
    /// Closed files whose tiles a fade may still paint; dropped once none does.
    tiles_closed: Vec<u64>,
    /// The hidden sidebar is peeking out over the canvas (`chrome.rs`).
    peek: bool,
    /// Saved colours, symbols and names of spaces (`spaces.rs`).
    spaces: spaces::Store,
    /// A two-finger swipe over the sidebar (`swipe.rs`).
    swipe: swipe::Swipe,
    /// How far the sidebar's pages are moved (points, last frame) and how wide a page is.
    slide: f32,
    /// The slide is a far jump's short nudge (it fades), not a page turn (side by side).
    slide_nudge: bool,
    page_width: f32,
    /// Where each space's dot was drawn last frame (popovers open above it), the part of
    /// the footer they scroll in, and the file whose dot was last scrolled into view.
    space_dots: Vec<(u64, egui::Rect)>,
    /// The window title last sent (a viewport command asks for another frame, so it is
    /// sent only when it changes).
    window_title: String,
    dots_clip: egui::Rect,
    dot_in_view: (u64, f32),
    /// ⌘F: find text on the page (`find.rs`).
    find: find::FindBar,
    /// The space being edited (`space_editor.rs`).
    space_editing: Option<space_editor::SpaceEditor>,
    /// A popover that closed, drawn while it fades away.
    space_leaving: Option<space_editor::SpaceEditor>,
    /// macOS: the blur behind the transparent window; `None` keeps the gradient opaque.
    #[cfg(target_os = "macos")]
    vibrancy: Option<vibrancy::Vibrancy>,
    #[cfg(target_arch = "wasm32")]
    web: web::Inbox,
    /// The window size cap last sent (`gpu::cap_window`).
    #[cfg(not(target_arch = "wasm32"))]
    window_cap: Option<f32>,
    /// What draws the canvas tiles ("GPU (Metal)", "CPU"), for the status bar.
    renderer: String,
    /// Agent access: the MCP server (`mcp_host.rs`).
    mcp: mcp_host::McpHost,
    /// The welcome sheet and the one-time tips (`onboarding.rs`).
    onboarding: onboarding::Onboarding,
    /// Files opened before: the Recents page when none is open (`recents.rs`).
    recents: recents::Recents,
    /// The file the Recents page was shown from with the + (still the workspace's).
    recents_from: Option<u64>,
    /// Files later launches handed to this figstudio (`handoff.rs`).
    #[cfg(not(target_arch = "wasm32"))]
    handed_over: Option<handoff::Inbox>,
    /// The window, to bring it forward when files are handed over (`foreground_win.rs`).
    #[cfg(windows)]
    window: Option<foreground_win::Window>,
    /// Startup files still arriving from the decode workers.
    restoring: Option<session::Restore>,
    workspace_store: session_store::Store,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let store = if cfg!(test) {
            session_store::Store::memory()
        } else {
            session_store::Store::load()
        };
        Self::with_workspace_store(cc, files, store)
    }

    fn with_workspace_store(
        cc: &eframe::CreationContext<'_>,
        files: Vec<PathBuf>,
        mut workspace_store: session_store::Store,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        theme::install_fonts(&cc.egui_ctx);
        // What the window looked like when it was last closed (eframe's storage).
        let saved = cc
            .storage
            .and_then(|s| eframe::get_value::<UiState>(s, UI_STATE_KEY));
        #[cfg(not(target_arch = "wasm32"))]
        let (workers, renderer, tiles) = {
            let mut tiles = tiles_view::Tiles::default();
            let state = cc.wgpu_render_state.as_ref();
            // The environment wins over Settings → Canvas.
            let env = std::env::var("FIGSTUDIO_RENDERER").ok().or_else(|| {
                saved
                    .as_ref()
                    .and_then(|s| s.prefs.renderer.env_value())
                    .map(str::to_owned)
            });
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
        let workspace = workspace_store.read(cc.storage);
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
            settings: Default::default(),
            prefs: saved.as_ref().map(|s| s.prefs.clone()).unwrap_or_default(),
            link: Default::default(),
            #[cfg(any(target_os = "macos", windows))]
            native_menu: None,
            #[cfg(any(target_os = "macos", windows))]
            native_keys: None,
            cursor: None,
            profile: std::env::var_os("FIGSTUDIO_PROFILE").is_some(),
            frameless: cfg!(target_os = "macos"),
            command: Default::default(),
            // egui keeps the theme preference with its memory; the menu follows it.
            appearance: menu::Appearance::from(cc.egui_ctx.options(|o| o.theme_preference)),
            reduce_motion: anim::reduced(&cc.egui_ctx),
            shown_file: None,
            card_rect: None,
            crossfade: chrome::Crossfade::default(),
            tiles_closed: Vec::new(),
            peek: false,
            // Tests never read or write the user's saved styles.
            spaces: if cfg!(test) {
                spaces::Store::memory()
            } else {
                spaces::Store::load()
            },
            space_editing: None,
            space_leaving: None,
            swipe: Default::default(),
            slide: 0.0,
            slide_nudge: false,
            page_width: 280.0,
            space_dots: Vec::new(),
            window_title: String::new(),
            dots_clip: egui::Rect::NOTHING,
            dot_in_view: (0, 0.0),
            find: Default::default(),
            #[cfg(target_os = "macos")]
            vibrancy: None,
            #[cfg(target_arch = "wasm32")]
            web: web::Inbox::new(),
            #[cfg(not(target_arch = "wasm32"))]
            window_cap: None,
            mcp: Default::default(),
            // Tests turn it on themselves; it would cover every other test's window.
            onboarding: onboarding::Onboarding::new(
                saved
                    .as_ref()
                    .map(|s| s.onboarding.clone())
                    .unwrap_or_default(),
                !cfg!(test),
            ),
            // Tests never read or write the user's list.
            recents_from: None,
            #[cfg(not(target_arch = "wasm32"))]
            handed_over: None,
            #[cfg(windows)]
            window: None,
            recents: if cfg!(test) {
                recents::Recents::memory()
            } else {
                recents::Recents::load()
            },
            restoring: None,
            workspace_store,
        };
        if let Some(saved) = saved {
            app.show_left = saved.show_left;
            app.show_right = saved.show_right;
            app.pages_share = saved.pages_share.clamp(0.05, 0.95);
        }
        if let Some(access) = app.mcp.access() {
            *access.on = app.prefs.agents.on;
            *access.remote = app.prefs.agents.remote;
            access.remote_addr.clone_from(&app.prefs.agents.remote_addr);
        }
        app.restore_workspace(workspace, files);
        app
    }

    fn open(&mut self, path: PathBuf) {
        if let Some(restore) = &mut self.restoring {
            restore.explicitly_opened(&path);
        }
        self.open_file(path);
    }

    fn open_file(&mut self, path: PathBuf) {
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

    /// Whether the window background is see-through (macOS with the blur attached).
    fn translucent(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.vibrancy.is_some() && self.prefs.translucent
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    fn tab(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    fn close(&mut self, i: usize) {
        if i < self.tabs.len() {
            let previous = self.tabs.get(self.active).map(|t| t.id);
            // A space popover on this file saves first: its style lives only on the tab.
            if self.space_editing.as_ref().map(|e| e.tab) == Some(self.tabs[i].id) {
                self.commit_space_editor();
            }
            // On the Recents page (after the last space) it stays.
            let home = self.active >= self.tabs.len();
            let closed = self.tabs.remove(i);
            #[cfg(target_arch = "wasm32")]
            web::forget_upload(closed.id);
            self.workers.close_tab(closed.id);
            let instance = closed.doc.instance;
            if !self.tabs.iter().any(|t| t.doc.instance == instance) {
                self.workers.close_document(instance);
                // Its picture may be what the next file fades in over (`canvas_card`).
                self.tiles_closed.push(instance);
            }
            if home {
                self.active = self.tabs.len();
                // The file the page came from closed: the one beside it, as a shown one would.
                if self.recents_from == Some(closed.id) {
                    let beside = i.min(self.tabs.len().saturating_sub(1));
                    self.recents_from = self.tabs.get(beside).map(|t| t.id);
                }
            } else if self.active >= self.tabs.len() {
                self.active = self.tabs.len().saturating_sub(1);
            } else if self.active > i {
                self.active -= 1;
            }
            self.workspace_closed(&closed, previous);
        }
    }

    fn export(&mut self, ctx: &egui::Context, selection_only: bool, format: worker::ExportFormat) {
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
            let file = format!("{name}.{}", format.extension());
            let images = self.workers.images();
            let started = self
                .workers
                .run_export(self.web.exports_tx.clone(), move || {
                    let bytes = export_bytes(images, &doc, source.as_ref(), root, format);
                    (file, bytes)
                });
            match started {
                Ok(()) => self.toast(ctx, "Exporting…", false),
                Err(e) => self.toast(ctx, e, true),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.save_export(ctx, doc, source, root, &name, format);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_export(
        &mut self,
        ctx: &egui::Context,
        doc: std::sync::Arc<figstudio_fig::Document>,
        source: Option<source::Source>,
        root: figstudio_fig::Guid,
        name: &str,
        format: worker::ExportFormat,
    ) {
        let ext = format.extension();
        let Some(path) = rfd::FileDialog::new()
            .add_filter(format.label(), &[ext])
            .set_file_name(format!("{name}.{ext}"))
            .save_file()
        else {
            return;
        };
        // The render and encode run off the UI thread on an immutable snapshot of the
        // document. The result comes back through `poll`, which reports it once.
        if let Err(e) = self.workers.export(doc, root, format, path, source) {
            self.toast(ctx, e, true);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(problem) = self.workspace_store.take_problem() {
            self.toast(ctx, problem, true);
        }
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
                    real_path,
                    source,
                    elapsed,
                    cjk,
                } => {
                    self.loading.finish(&path);
                    #[cfg(target_arch = "wasm32")]
                    self.web
                        .symbols
                        .load_if_needed(ctx, &doc, &path.to_string_lossy());
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = cjk;
                    if self.reuse_loaded_file(ctx, &path, real_path.as_deref()) {
                        continue;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    let automatic = self
                        .restoring
                        .as_ref()
                        .is_some_and(|restore| restore.is_automatic(&path));
                    let id = self.next_id;
                    self.next_id += 1;
                    #[cfg(target_arch = "wasm32")]
                    web::bind_upload(&path, id);
                    #[cfg(target_arch = "wasm32")]
                    self.web.load_cjk_for(ctx, cjk, id);
                    let mut tab = Tab::new(id, path.clone(), doc, source, elapsed)
                        .with_doc_key(doc_key)
                        .with_real_path(real_path);
                    tab.tint = theme::pick_tint(&tab.title(), self.tabs.iter().map(|t| t.tint));
                    let title = tab.title();
                    tab.style = self
                        .spaces
                        .lookup(spaces::FileId::of(&tab, &title))
                        .unwrap_or_default();
                    let previous = self.tabs.get(self.active).map(|t| t.id);
                    self.tabs.push(tab);
                    self.active = self.tabs.len() - 1;
                    self.workspace_finished(&path, previous, None);
                    if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                        self.link_loaded(ctx, &path, i);
                        // After the link: the picture is of the page it shows. A file the
                        // workspace reopened by itself is not listed again (the browser lists
                        // only files picked or dropped there).
                        #[cfg(not(target_arch = "wasm32"))]
                        let listed = !automatic;
                        #[cfg(target_arch = "wasm32")]
                        let listed = true;
                        if listed {
                            self.note_recent(ctx, i);
                        }
                    }
                }
                Loaded::Err {
                    path,
                    real_path,
                    message,
                } => {
                    self.loading.finish(&path);
                    #[cfg(target_arch = "wasm32")]
                    web::fail_upload(&path);
                    #[cfg(not(target_arch = "wasm32"))]
                    self.recents.load_failed(&path);
                    self.link_failed(&path, real_path);
                    self.workspace_finished(&path, self.tabs.get(self.active).map(|t| t.id), None);
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
        self.recents.sync(ctx);
        #[cfg(target_arch = "wasm32")]
        if let Some(n) = web::take_page_request()
            && let Some(tab) = self.tab()
            && let Some(page) = tab.page_at(n)
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
        if canvas_fonts::sync(ctx) {
            self.workers.fonts_changed();
            self.tiles.fonts_changed(self.workers.images_arrived());
            for tab in &mut self.tabs {
                self.tiles.forget(tab.doc.instance);
                tab.tile_requests.clear();
            }
            for i in 0..self.tabs.len() {
                self.refresh_recent_thumbnail(ctx, i);
            }
        }
        let mut cjk_updated = false;
        while let Ok((variant, font)) = self.web.fonts.try_recv() {
            if variant != self.web.font_variant() {
                continue;
            }
            match font {
                Ok(bytes) => {
                    theme::add_cjk_font(ctx, bytes);
                    cjk_updated = true;
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
        // set_fonts applies next frame; don't overwrite a pending CJK update with old definitions.
        if !cjk_updated && let Some(error) = self.web.symbols.poll(ctx) {
            self.toast(
                ctx,
                format!(
                    "Could not load symbol fonts: {}",
                    figstudio_fig::one_line(error.as_ref())
                ),
                true,
            );
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
                        self.workers.decode(name, false, move || {
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
                web::Picked::Unavailable { name, handle, why } => {
                    let shown = name.display().to_string();
                    let text = match why {
                        crate::recents_web::Unavailable::Missing => {
                            let id = recents::Id {
                                path: name,
                                handle: Some(handle),
                            };
                            self.recents.set_missing(&id, true);
                            format!("{shown} was moved or deleted. Pick it where it is now.")
                        }
                        crate::recents_web::Unavailable::Denied => format!(
                            "figstudio was not allowed to open {shown}. Click it again to allow it."
                        ),
                    };
                    self.toast(ctx, text, true);
                }
            }
        }
        while let Ok((name, bytes)) = self.web.exports.try_recv() {
            let mime = if name.ends_with(".svg") {
                "image/svg+xml"
            } else {
                "image/png"
            };
            match bytes.and_then(|bytes| web::download(&name, mime, &bytes)) {
                Ok(()) => self.toast(ctx, format!("Exported {name}"), false),
                Err(e) => self.toast(ctx, figstudio_fig::one_line(e.as_ref()), true),
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Recording a new shortcut in Settings: every key goes there.
        if self.settings.recording().is_some() {
            return;
        }
        // The welcome sheet is modal while open: its own keys only (fading out it takes
        // none, so a shortcut pressed right after closing it works).
        if self.onboarding.sheet_open() {
            self.welcome_keys(ctx);
            return;
        }
        let keys = |b: keymap::Binding| self.prefs.keymap.get(b);
        // The command bar is modal, also while it fades out (it still covers the app): only
        // its own shortcut (which closes or reopens it) gets through, so nothing changes the
        // file under its results.
        if self.command.is_up(ctx) {
            if !self.native_takes(menu::Action::CommandBar, keys(keymap::Binding::CommandBar))
                && let Some(s) = keys(keymap::Binding::CommandBar)
                && ctx.input_mut(|i| i.consume_shortcut(&s))
            {
                self.command.toggle(ctx);
            }
            return;
        }
        // Keys without ⌘ / Ctrl / Alt wait while text is typed or a panel is up (or fading).
        let quiet =
            ctx.egui_wants_keyboard_input() || self.settings.is_up(ctx) || self.space_popover_up();
        // Most modifiers first: a Ctrl+Tab pattern also matches Ctrl+Shift+Tab.
        let mut bindings: Vec<(keymap::Binding, KeyboardShortcut)> = keymap::Binding::ALL
            .into_iter()
            .filter_map(|b| keys(b).map(|s| (b, s)))
            .collect();
        bindings.sort_by_key(|(_, s)| {
            let m = s.modifiers;
            std::cmp::Reverse(
                u8::from(m.command) + u8::from(m.ctrl) + u8::from(m.alt) + u8::from(m.shift),
            )
        });
        let mut run = Vec::new();
        for (binding, shortcut) in bindings {
            use keymap::Binding as B;
            let action = binding.action();
            let plain = keymap::Binding::is_plain(&shortcut);
            if (matches!(binding, B::SelectChild | B::SelectParent)
                && (quiet || self.find.is_open() || self.command.is_up(ctx)))
                || ((plain || binding == B::Copy) && quiet)
                || (binding == B::SelectAll
                    && (quiet
                        || !self
                            .tabs
                            .get(self.active)
                            .is_some_and(|t| t.text_select.is_some())))
                || self.native_takes(action, Some(shortcut))
                || (matches!(binding, B::NextMatch | B::PreviousMatch) && !self.find.is_open())
            {
                continue;
            }
            let default = keymap::same(&shortcut, &binding.default_keys());
            let pressed = ctx.input_mut(|i| match binding {
                // Shift+digit: the logical key depends on the layout (Shift+1 is `!` on US
                // layouts), so the default also matches the physical key.
                B::ZoomFit if default => shifted_digit(i, Key::Num1, Some(Key::Exclamationmark)),
                B::ActualSize if default => shifted_digit(i, Key::Num0, None),
                // Platform clipboard integrations send Copy without a key event.
                B::Copy if default => {
                    let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                    i.events.retain(|e| !matches!(e, egui::Event::Copy));
                    i.consume_shortcut(&shortcut) || copy
                }
                // `+` where it needs no Shift (a keypad) zooms in too.
                B::ZoomIn if default => {
                    i.consume_shortcut(&shortcut)
                        || (i.key_pressed(Key::Plus) && !i.modifiers.shift)
                }
                _ => i.consume_shortcut(&shortcut),
            });
            if pressed {
                run.push(action);
            }
        }
        for action in run {
            self.apply(action, ctx);
        }
        // Escape closes the find bar first, also when its field no longer has focus (a
        // click on the canvas took it); the next one clears the selection.
        if !quiet && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            if self.find.is_open() {
                self.find.hide();
            } else {
                self.apply(menu::Action::ClearSelection, ctx);
            }
        }
    }

    /// Numbers only (no names or content), for automated browser checks: see
    /// `window.figstudioStatus` in `web.rs`.
    #[cfg(target_arch = "wasm32")]
    fn status_json(&self) -> String {
        let tab = self.tabs.get(self.active);
        let s = tab.map(|t| &t.stats);
        let page = tab.and_then(Tab::page_index);
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
            tab.map_or(0, |t| t.doc.pages.len() + t.doc.internal_pages.len()),
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
    /// Runs before every `ui`, and also when the window is not drawn (minimized: eframe
    /// skips `ui` then), so files a later launch hands over are taken either way.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(not(target_arch = "wasm32"))]
        self.open_handed_over(ctx);
        // A hidden browser tab runs only this, on a throttled timer: keep agent access
        // alive there (keepalive, reconnect, reveals, image reads). `ui` does it otherwise.
        #[cfg(target_arch = "wasm32")]
        if web::page_hidden() {
            self.web.serve_ranges(ctx);
            if let Some(error) = self.mcp.frame(&mut self.tabs, &mut self.active) {
                self.toast(ctx, error, true);
            }
            self.mcp.sync(ctx, &self.tabs, self.active);
        }
    }

    /// Transparent only where the window blurs what is behind it (macOS with the blur
    /// attached); everywhere else an opaque colour under the gradient.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.translucent() {
            [0.0; 4]
        } else {
            visuals.window_fill.to_normalized_gamma_f32()
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let mut prefs = self.prefs.clone();
        if let Some(access) = self.mcp.access() {
            prefs.agents = settings::AgentPrefs {
                on: *access.on,
                remote: *access.remote,
                remote_addr: access.remote_addr.clone(),
            };
        }
        let workspace = session::Session::capture(self);
        self.workspace_store.save(&workspace);
        let state = UiState {
            show_left: self.show_left,
            show_right: self.show_right,
            pages_share: self.pages_share,
            prefs,
            onboarding: self.onboarding.progress.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            legacy_workspace: self.workspace_store.legacy_backup(),
        };
        eframe::set_value(storage, UI_STATE_KEY, &state);
    }

    fn on_exit(&mut self) {
        self.commit_space_editor();
        self.mcp.stop();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.workspace_store.take_migration()
            && let Some(storage) = frame.storage_mut()
        {
            self.save(storage);
            storage.flush();
        }
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
        // A shortcut being recorded takes the keys before anything else sees them.
        self.record_shortcut(&ctx);
        // Polled first, so a listing that came back this frame is not still counted as
        // blocked for the links taken next.
        self.poll_link_searches(&ctx);
        for ask in self.mcp.take_links() {
            self.go_to_link_at(&ctx, &ask.link, Some(ask.done), ask.asked);
        }
        self.native_events(&ctx);
        // Ahead of the shortcuts: the dialog's Escape must be consumed first.
        self.mcp.confirm(&ctx);
        self.shortcuts(&ctx);
        self.menu_windows(&ctx);
        let space_held = ctx.input(|i| i.key_down(Key::Space)) && !ctx.egui_wants_keyboard_input();
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());

        self.note_active_file(&ctx);
        // The sidebar records where the tips point as it draws.
        self.onboarding.targets = Default::default();
        // The menu's check mark follows the real setting (a host may turn motion off too).
        self.reduce_motion = anim::reduced(&ctx);
        let p = theme::palette(&ctx);
        #[cfg(target_os = "macos")]
        if let Some(blur) = &mut self.vibrancy {
            blur.set_dark(p.dark);
        }
        chrome::paint_gradient(
            &ctx.layer_painter(egui::LayerId::background()),
            ctx.content_rect(),
            self.tint_now(&ctx),
            if self.translucent() {
                theme::TRANSLUCENT_TINT
            } else {
                1.0
            },
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
        egui::Panel::left(chrome::SIDEBAR)
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
            .default_size(232.0)
            .size_range(196.0..=560.0)
            .show_collapsible(ui, &mut show_left, |ui| self.sidebar(ui));
        // The sidebar's own button may have hidden it this frame; a drag past its edge too.
        self.show_left = self.show_left && show_left;

        let has_tab = self.tabs.get(self.active).is_some();
        let mut show_right = has_tab && self.show_right;
        // The Properties card follows the space's colour, crossfading with the window.
        let card_p = p.on_tint(self.tint_now(&ctx));
        let card = egui::Frame::new()
            .fill(card_p.card)
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
            .size_range(220.0..=560.0)
            .show_collapsible(ui, &mut show_right, |ui| {
                // The section wells (`panels::section`) read this.
                ui.visuals_mut().faint_bg_color = card_p.well;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    // Another selection's properties fade in.
                    ui.multiply_opacity(anim::arrive(
                        ui.ctx(),
                        egui::Id::new("properties-in"),
                        (tab.id, tab.selection),
                        0.15,
                    ));
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
        self.space_popover(&ctx);
        self.drop_overlay(&ctx, hovering_files);
        self.tips(&ctx);
        self.command_bar(&ctx);
        self.welcome_sheet(&ctx);
        self.toasts(&ctx);
        let answers = self.take_link_answers();
        self.mcp
            .sync_then_answer(&ctx, &self.tabs, self.active, answers);
        let title = self.tabs.get(self.active).map_or_else(
            || APP_NAME.to_owned(),
            |tab| format!("{} — {APP_NAME}", tab.title()),
        );
        if title != self.window_title {
            self.window_title.clone_from(&title);
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_app_icon_decodes_as_a_square_of_at_least_256_px() {
        let icon = super::app_icon().expect("assets/icon.png decodes");
        assert_eq!(icon.width, icon.height);
        assert!(icon.width >= 256, "{}", icon.width);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }

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
