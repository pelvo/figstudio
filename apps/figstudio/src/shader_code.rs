//! Shader code for open files. Figma keeps a shader's code in its cloud, not in the `.fig`,
//! so a shader layer is drawn only with the code Figma's "View code" → "Download files" saves:
//! zips the user picks or drops (this session), the folder next to the file
//! (`FILE.fig.shaders/`, desktop) and the shader folder in Settings. A file using shaders
//! without code gets a notice with the steps, and markers on those layers to choose the zip.

use std::cell::Cell;
use std::sync::Arc;

use egui::{Pos2, RichText, Sense, Stroke, vec2};
use figstudio_fig::Guid;
use figstudio_mcp_core::shaders::{self, Missing, ShaderSource};
use figstudio_render::shader::ShaderFills;

use crate::tab::Tab;
use crate::theme;

/// The shader code of one open file.
#[derive(Default)]
pub struct TabShaders {
    /// Zips picked or dropped for this file (this session).
    pub picked: Vec<ShaderSource>,
    /// The folder next to the file.
    pub beside: Vec<ShaderSource>,
    /// Shaders its visible layers use without code.
    pub missing: Vec<Missing>,
    /// The notice was closed.
    pub notice_closed: bool,
    pub steps_open: bool,
    /// The user asked to choose a zip: for this shader's ID (`Some(None)`: any shader).
    pub choose: Cell<Option<Option<String>>>,
}

impl TabShaders {
    /// Every source for this file: picked first (they win a tie), then beside it, then the
    /// shader folder's.
    pub fn sources(&self, folder: &[ShaderSource]) -> Vec<ShaderSource> {
        self.picked
            .iter()
            .chain(&self.beside)
            .chain(folder)
            .cloned()
            .collect()
    }

    /// Adds a picked source, replacing an earlier one with the same label.
    pub fn add(&mut self, source: ShaderSource) {
        self.picked.retain(|s| s.label != source.label);
        self.picked.push(source);
    }

    pub fn is_missing(&self, id: &str) -> bool {
        self.missing.iter().any(|m| m.id == id)
    }
}

/// What draws a file's shaders from `sources`: on desktop the shader runtime on wgpu. `None`
/// where shaders cannot run (the browser build has no runtime yet).
pub fn host(sources: Vec<ShaderSource>) -> Option<Arc<dyn ShaderFills>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(Arc::new(figstudio_shader::Fills::new(sources)))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = sources;
        None
    }
}

/// Recomputes which shaders of `tab` lack code and gives its document a host for `sources`.
/// The caller redraws the document.
pub fn apply(tab: &mut Tab, sources: Vec<ShaderSource>, images: &figstudio_render::images::Images) {
    tab.shaders.missing = shaders::missing(&tab.doc, &sources);
    if let Some(host) = host(sources) {
        images.set_shaders(tab.doc.instance, host);
    }
}

/// Largest shader zip read.
pub const MAX_ZIP_BYTES: usize = 20_000_000;

/// A zip's bytes as a source, labelled with the shader it was chosen for (else its name).
pub fn source_from_zip(
    name: &str,
    shader: Option<&str>,
    bytes: &[u8],
) -> Result<ShaderSource, String> {
    if bytes.len() > MAX_ZIP_BYTES {
        return Err(format!("{name} is too big for a shader's code"));
    }
    let label = shader.map_or_else(
        || {
            name.strip_suffix(".zip")
                .or_else(|| name.strip_suffix(".ZIP"))
                .unwrap_or(name)
                .to_owned()
        },
        str::to_owned,
    );
    let source = ShaderSource::from_zip(label, bytes)
        .ok_or_else(|| format!("{name} is not a zip of a shader's code"))?;
    if !source.has_code() {
        return Err(format!("{name} has no main.ts (not a shader's code)"));
    }
    Ok(source)
}

/// A zip read off the UI thread (browser picks and drops), for the tab `tab`.
pub struct Zip {
    pub tab: u64,
    pub name: String,
    /// The shader it was chosen for.
    pub shader: Option<String>,
    pub bytes: Vec<u8>,
}

/// App-wide shader code: the shader folder's sources and zips on their way in.
pub struct State {
    /// Sources in the shader folder set in Settings.
    pub folder: Vec<ShaderSource>,
    #[cfg(not(target_arch = "wasm32"))]
    watch: watch::Watch,
    #[cfg_attr(
        all(not(target_arch = "wasm32"), not(test)),
        expect(dead_code, reason = "only the browser reads zips in the background")
    )]
    pub(crate) tx: std::sync::mpsc::Sender<Zip>,
    rx: std::sync::mpsc::Receiver<Zip>,
}

impl Default for State {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        State {
            folder: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            watch: watch::Watch::default(),
            tx,
            rx,
        }
    }
}

impl crate::App {
    /// Gives tab `i` its shader host and missing list from its sources, and redraws its
    /// document.
    pub(crate) fn apply_shaders(&mut self, i: usize) {
        let Some(tab) = self.tabs.get_mut(i) else {
            return;
        };
        let sources = tab.shaders.sources(&self.shader_code.folder);
        apply(tab, sources, &self.workers.images());
        let instance = tab.doc.instance;
        self.workers.redraw_document(instance);
        self.tiles.forget(instance);
        for tab in self.tabs.iter_mut().filter(|t| t.doc.instance == instance) {
            tab.tile_requests.clear();
        }
    }

    /// Adds a zip to tab `id` and redraws it; a toast tells the user what happened.
    fn add_zip(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        name: &str,
        shader: Option<&str>,
        bytes: &[u8],
    ) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        match source_from_zip(name, shader, bytes) {
            Ok(source) => {
                let label = source.name.clone().unwrap_or_else(|| name.to_owned());
                let before = self.tabs[i].shaders.missing.len();
                self.tabs[i].shaders.add(source);
                self.apply_shaders(i);
                let after = self.tabs[i].shaders.missing.len();
                if after < before {
                    self.toast(ctx, format!("Added the code of {label}"), false);
                } else {
                    self.toast(
                        ctx,
                        format!("{label} is not the code of a shader this file is missing"),
                        true,
                    );
                }
            }
            Err(e) => self.toast(ctx, e, true),
        }
    }

    /// Dropped files that are zips go to the active tab; returns the rest.
    pub(crate) fn take_shader_zips(
        &mut self,
        ctx: &egui::Context,
        dropped: Vec<egui::DroppedFileHandle>,
    ) -> Vec<egui::DroppedFileHandle> {
        let (zips, rest): (Vec<_>, Vec<_>) = dropped.into_iter().partition(|f| {
            f.path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        });
        if zips.is_empty() {
            return rest;
        }
        let Some(id) = self.tabs.get(self.active).map(|t| t.id) else {
            self.toast(ctx, "Open the .fig first, then drop its shader zips", true);
            return rest;
        };
        for f in zips {
            let name = f
                .path()
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            #[cfg(not(target_arch = "wasm32"))]
            match f.bytes() {
                Ok(bytes) => self.add_zip(ctx, id, &name, None, &bytes),
                Err(e) => self.toast(ctx, format!("Could not read {name}: {e}"), true),
            }
            #[cfg(target_arch = "wasm32")]
            {
                let tx = self.shader_code.tx.clone();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // The browser reads files asynchronously; a failed read is dropped here and
                    // reported as an empty zip.
                    let bytes = f.bytes_async().await.unwrap_or_default();
                    let _ = tx.send(Zip {
                        tab: id,
                        name,
                        shader: None,
                        bytes,
                    });
                    ctx.request_repaint();
                });
            }
        }
        rest
    }

    /// Runs what the notice, the markers and the properties asked for, and takes zips read
    /// in the background. Call every frame.
    pub(crate) fn poll_shader_code(&mut self, ctx: &egui::Context) {
        while let Ok(zip) = self.shader_code.rx.try_recv() {
            self.add_zip(ctx, zip.tab, &zip.name, zip.shader.as_deref(), &zip.bytes);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.watch_shader_folders(ctx);
        // Tests read the request instead of opening a file dialog.
        if cfg!(test) {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(shader) = tab.shaders.choose.take() else {
            return;
        };
        let id = tab.id;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(files) = rfd::FileDialog::new()
                .set_title("Choose shader code (Figma's View code → Download files)")
                .add_filter("Shader code", &["zip"])
                .pick_files()
            else {
                return;
            };
            // A zip chosen for one shader is that shader's, even under another name.
            let one = files.len() == 1;
            for path in files {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                match read_zip(&path) {
                    Ok(bytes) => {
                        self.add_zip(ctx, id, &name, shader.as_deref().filter(|_| one), &bytes)
                    }
                    Err(e) => self.toast(ctx, e, true),
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        crate::web::pick_zips(ctx, id, shader, self.shader_code.tx.clone());
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::App {
    /// Watches the shader folder in Settings and each open file's `FILE.fig.shaders/`; a
    /// change gives the files using them their new code.
    fn watch_shader_folders(&mut self, ctx: &egui::Context) {
        let global = self.prefs.shader_folder.clone();
        let mut dirs: Vec<std::path::PathBuf> = global.iter().cloned().collect();
        dirs.extend(
            self.tabs
                .iter()
                .filter(|t| t.real_path.is_absolute())
                .map(|t| figstudio_mcp_core::shader_sources::folder(&t.real_path)),
        );
        self.shader_code.watch.set(ctx, dirs);
        if global.is_none() && !self.shader_code.folder.is_empty() {
            self.shader_code.folder.clear();
            for i in 0..self.tabs.len() {
                self.apply_shaders(i);
            }
        }
        for (dir, sources) in self.shader_code.watch.changed() {
            if global.as_ref() == Some(&dir) {
                if self.shader_code.folder != sources {
                    self.shader_code.folder = sources.clone();
                    for i in 0..self.tabs.len() {
                        self.apply_shaders(i);
                    }
                }
                continue;
            }
            for i in 0..self.tabs.len() {
                let tab = &mut self.tabs[i];
                if figstudio_mcp_core::shader_sources::folder(&tab.real_path) == dir
                    && tab.shaders.beside != sources
                {
                    tab.shaders.beside = sources.clone();
                    self.apply_shaders(i);
                }
            }
        }
    }
}

/// Polls shader folders on a thread: a folder whose entries (names, sizes, times) changed
/// is read again and sent to the UI.
#[cfg(not(target_arch = "wasm32"))]
mod watch {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use figstudio_mcp_core::shaders::ShaderSource;

    /// How often folders are looked at.
    const EVERY: Duration = Duration::from_secs(2);

    type Changed = (PathBuf, Vec<ShaderSource>);

    #[derive(Default)]
    pub struct Watch {
        dirs: Arc<Mutex<Vec<PathBuf>>>,
        rx: Option<Receiver<Changed>>,
    }

    impl Watch {
        /// The folders to watch; the thread starts with the first.
        pub fn set(&mut self, ctx: &egui::Context, dirs: Vec<PathBuf>) {
            {
                let mut current = self.dirs.lock().unwrap_or_else(PoisonError::into_inner);
                if *current != dirs {
                    *current = dirs;
                }
            }
            if self.rx.is_none()
                && !self
                    .dirs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_empty()
            {
                let (tx, rx) = channel();
                self.rx = Some(rx);
                let (dirs, ctx) = (self.dirs.clone(), ctx.clone());
                std::thread::Builder::new()
                    .name("shader-folders".into())
                    .spawn(move || run(&dirs, &tx, &ctx))
                    .map_err(|e| tracing::warn!("shader folder watch not started: {e}"))
                    .ok();
            }
        }

        /// Folders whose code changed since the last call.
        pub fn changed(&self) -> Vec<Changed> {
            self.rx
                .as_ref()
                .map(|rx| rx.try_iter().collect())
                .unwrap_or_default()
        }
    }

    fn run(dirs: &Mutex<Vec<PathBuf>>, tx: &Sender<Changed>, ctx: &egui::Context) {
        let mut seen: HashMap<PathBuf, Vec<(String, u64, Option<std::time::SystemTime>)>> =
            HashMap::new();
        loop {
            let now: Vec<PathBuf> = dirs.lock().unwrap_or_else(PoisonError::into_inner).clone();
            seen.retain(|d, _| now.contains(d));
            for dir in now {
                let listing = listing(&dir);
                if seen.get(&dir) != Some(&listing) {
                    let sources = figstudio_mcp_core::shader_sources::load_dir(&dir);
                    seen.insert(dir.clone(), listing);
                    if tx.send((dir, sources)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
            std::thread::sleep(EVERY);
        }
    }

    /// A folder's entries: name, size and change time (empty when it does not exist).
    fn listing(dir: &std::path::Path) -> Vec<(String, u64, Option<std::time::SystemTime>)> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<_> = entries
            .filter_map(|e| {
                let e = e.ok()?;
                let meta = e.metadata().ok()?;
                Some((
                    e.file_name().to_string_lossy().into_owned(),
                    meta.len(),
                    meta.modified().ok(),
                ))
            })
            .collect();
        out.sort();
        out
    }
}

/// A zip on disk, read when it is small enough to be a shader's code.
#[cfg(not(target_arch = "wasm32"))]
fn read_zip(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let len = std::fs::metadata(path)
        .map_err(|e| format!("Could not read {name}: {e}"))?
        .len();
    if len > MAX_ZIP_BYTES as u64 {
        return Err(format!("{name} is too big for a shader's code"));
    }
    std::fs::read(path).map_err(|e| format!("Could not read {name}: {e}"))
}

/// What the notice asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum NoticeAction {
    Choose,
    Close,
}

/// The notice over the canvas (`rect`) for a file using shaders without code.
pub fn notice(ui: &egui::Ui, rect: egui::Rect, tab: &mut Tab) -> Option<NoticeAction> {
    if tab.shaders.missing.is_empty() || tab.shaders.notice_closed {
        return None;
    }
    let n = tab.shaders.missing.len();
    let layers: usize = tab.shaders.missing.iter().map(|m| m.layers.len()).sum();
    let mut action = None;
    let p = theme::of(ui);
    egui::Area::new(egui::Id::new(("shader-notice", tab.id)))
        .order(egui::Order::Foreground)
        // Under the status pills at the top of the canvas.
        .fixed_pos(rect.center_top() + vec2(-210.0, 56.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("◆").color(theme::ACCENT));
                    ui.label(
                        RichText::new(format!(
                            "{n} shader{} without {} code: {layers} layer{} not drawn",
                            if n == 1 { "" } else { "s" },
                            if n == 1 { "its" } else { "their" },
                            if layers == 1 { " is" } else { "s are" },
                        ))
                        .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::icons::sized_button(
                            ui,
                            crate::icons::Icon::Close,
                            "Hide",
                            false,
                            20.0,
                        )
                        .clicked()
                        {
                            action = Some(NoticeAction::Close);
                        }
                    });
                });
                ui.label(
                    RichText::new(
                        "Figma keeps a shader's code online, not in the .fig. Add it to draw \
                         these layers; they are marked on the canvas.",
                    )
                    .color(p.text_faint)
                    .small(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Choose shader zips…").clicked() {
                        action = Some(NoticeAction::Choose);
                    }
                    let label = if tab.shaders.steps_open {
                        "Hide steps"
                    } else {
                        "How to get them"
                    };
                    if ui.button(label).clicked() {
                        tab.shaders.steps_open = !tab.shaders.steps_open;
                    }
                });
                if tab.shaders.steps_open {
                    for (i, step) in steps().iter().enumerate() {
                        ui.label(RichText::new(format!("{}. {step}", i + 1)).small());
                    }
                }
            });
        });
    action
}

/// How to get a shader's code, as a list of steps.
fn steps() -> Vec<String> {
    let mut steps = vec![
        "In Figma, select a marked layer.".to_owned(),
        "In its Fill or Effects, open the shader, then … → View code → Download files.".to_owned(),
        "Drop the downloaded zip here, or click the layer's marker to choose it.".to_owned(),
    ];
    #[cfg(not(target_arch = "wasm32"))]
    steps.push(
        "To keep it: put the zips in the folder FILE.fig.shaders next to the file, or in the \
         shader folder set in Settings."
            .to_owned(),
    );
    #[cfg(target_arch = "wasm32")]
    steps.push("Zips you add are used until the tab is closed.".to_owned());
    steps
}

/// Most markers drawn at once.
const MAX_MARKERS: usize = 200;

/// A marker at the top-left of each layer on screen whose shader has no code; clicking one
/// asks to choose that shader's zip. `corners` gives a layer's screen corners on the page.
pub fn markers(
    ui: &egui::Ui,
    painter: &egui::Painter,
    tab: &Tab,
    corners: impl Fn(Guid) -> Option<[Pos2; 4]>,
) {
    if tab.shaders.missing.is_empty() {
        return;
    }
    let clip = painter.clip_rect();
    let mut drawn = 0;
    for missing in &tab.shaders.missing {
        for &layer in &missing.layers {
            if drawn >= MAX_MARKERS {
                return;
            }
            let Some(c) = corners(layer) else { continue };
            let bounds = egui::Rect::from_points(&c);
            if !bounds.intersects(clip) {
                continue;
            }
            drawn += 1;
            let text = if missing.kind == "effect" {
                "◆ Shader effect: code missing"
            } else {
                "◆ Shader: code missing"
            };
            let full = painter.layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(11.0),
                egui::Color32::WHITE,
            );
            // A layer narrower than the label gets the diamond alone (its label on hover).
            let galley = if bounds.width() < full.size().x + 24.0 {
                painter.layout_no_wrap(
                    "◆".to_owned(),
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                )
            } else {
                full
            };
            let pill = egui::Rect::from_min_size(
                bounds.min.max(clip.min) + vec2(6.0, 6.0),
                galley.size() + vec2(12.0, 6.0),
            );
            let response = ui.interact(
                pill,
                egui::Id::new(("shader-marker", tab.id, layer)),
                Sense::click(),
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
            let response = response
                .on_hover_text(format!(
                    "{}. Click to choose its code: the zip of Figma's View code → Download files",
                    text.trim_start_matches("◆ ")
                ))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let fill = if response.hovered() {
                theme::ACCENT
            } else {
                theme::ACCENT.gamma_multiply(0.85)
            };
            painter.rect(pill, 6.0, fill, Stroke::NONE, egui::StrokeKind::Inside);
            painter.galley(pill.min + vec2(6.0, 3.0), galley, egui::Color32::WHITE);
            if response.clicked() {
                tab.shaders.choose.set(Some(Some(missing.id.clone())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_zip_replaces_one_with_the_same_label_and_comes_first() {
        let source =
            |label: &str| ShaderSource::new(label.into(), vec![("main.ts".into(), String::new())]);
        let mut shaders = TabShaders {
            beside: vec![source("a")],
            ..Default::default()
        };
        shaders.add(source("a"));
        shaders.add(source("b"));
        shaders.add(source("a"));
        let labels: Vec<_> = shaders
            .sources(&[source("c")])
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert_eq!(labels, ["b", "a", "a", "c"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_watched_folder_reports_new_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut watch = watch::Watch::default();
        let ctx = egui::Context::default();
        watch.set(&ctx, vec![dir.path().to_path_buf()]);
        let wait = |watch: &watch::Watch| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let changed = watch.changed();
                if !changed.is_empty() || std::time::Instant::now() > deadline {
                    return changed;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        };
        // The first look reports the empty folder.
        assert_eq!(wait(&watch)[0].1, Vec::new());
        let code = dir.path().join("Concentric");
        std::fs::create_dir(&code).unwrap();
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/shader-sources/concentric"
        );
        for f in ["features.json", "main.ts"] {
            std::fs::copy(format!("{fixture}/{f}"), code.join(f)).unwrap();
        }
        let changed = wait(&watch);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].1[0].name.as_deref(), Some("Concentric patterns"));
    }

    #[test]
    fn a_bad_zip_says_why() {
        assert!(
            source_from_zip("x.zip", None, b"nope")
                .unwrap_err()
                .contains("not a zip")
        );
        assert!(
            source_from_zip("x.zip", None, &vec![0; MAX_ZIP_BYTES + 1])
                .unwrap_err()
                .contains("too big")
        );
    }
}
