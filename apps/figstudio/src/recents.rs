//! Recently opened files (desktop only): the Recents page the canvas card shows when no file
//! is open, as Figma's file browser, in the Arc look. Each file is a card in its space's
//! gradient with a picture of the page it opened on.
//!
//! The list is `<config>/figstudio/recents.json` (full paths: it is how a file is opened
//! again), the pictures `<config>/figstudio/thumbs/<hash of the path>.png`. Both are written
//! owner-only and never leave this computer. The browser has no paths to open again, so it
//! keeps the plain empty state.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use egui::{
    Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui,
    UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};
use serde::{Deserialize, Serialize};

use crate::App;
use crate::chrome::{accent_button, card_shadow, context_menu, elided, paint_badge, with_keys};
use crate::icons::{self, Icon};
use crate::spaces::{self, Style};
use crate::theme::{self, Tint};

/// Most files listed; the oldest go first (with their pictures).
const MAX_ENTRIES: usize = 60;
/// Largest list read (bytes); a bigger one is ignored, not read.
const MAX_BYTES: u64 = 1 << 20;
/// Largest picture read back (bytes).
const MAX_THUMB_BYTES: u64 = 4 << 20;
/// A picture fits in this many pixels (two points per pixel on a card about 300 wide).
pub const THUMB_SIZE: (f64, f64) = (640.0, 400.0);
/// Pictures decoded per frame: the first frame of a long list does not stall.
const LOADS_PER_FRAME: usize = 3;
/// Narrowest card in the grid, and the gap between cards.
const CARD_MIN: f32 = 210.0;
const CARD_GAP: f32 = 20.0;
/// A list row's height.
const ROW: f32 = 54.0;
/// Seconds between looks for moved or deleted files while the page is up (at most 60
/// `stat` calls).
const RECHECK: f64 = 1.0;
/// The header: room kept for the title, Open file…'s width (and the narrowest it gets
/// before it goes; ⌘O and the sidebar's + still open files), the view toggle's width.
const HEADER_TITLE: f32 = 150.0;
const OPEN_W: f32 = 124.0;
const OPEN_MIN: f32 = 72.0;
const TOGGLE_W: f32 = 66.0;

/// One file opened before.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    /// When it was last opened, in seconds since the Unix epoch.
    pub opened: u64,
    /// Its content key (`Tab::doc_key`): finds its space's look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl Entry {
    /// The file name, as a tab shows it.
    pub fn file_name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    fn style(&self, spaces: &spaces::Store) -> Style {
        let file = self.file_name();
        let folder = spaces::folder_of(&self.path);
        spaces
            .lookup(spaces::FileId {
                key: self.key.as_deref(),
                file: &file,
                folder: folder.as_deref(),
            })
            .unwrap_or_default()
    }
}

/// A picture to draw: the page a file opened on. It has no image cache of its own to
/// hand over: the picture is drawn with a fresh one (`thumb_png`), so it never advances the
/// canvas cache's clock, waits on its reads, or marks its images failed.
pub struct ThumbJob {
    pub doc: Arc<figstudio_fig::Document>,
    pub page: figstudio_fig::Guid,
    pub source: Option<crate::source::Source>,
    pub out: PathBuf,
}

/// What was clicked on the Recents page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Open(PathBuf),
    OpenDialog,
    Reveal(PathBuf),
    Remove(PathBuf),
}

/// The looking thread's two ends: looks asked for (by generation, with the paths) and
/// its answers (the generation, and the paths not found).
struct Looker {
    ask: mpsc::Sender<(u64, Vec<PathBuf>)>,
    answers: mpsc::Receiver<(u64, HashSet<PathBuf>)>,
}

/// The list, its pictures and the page's state.
pub struct Recents {
    /// Newest first.
    entries: Vec<Entry>,
    /// Where the list is saved; `None` keeps it in memory (tests, an unreadable list).
    file: Option<PathBuf>,
    /// Where the pictures are; `None` draws none.
    thumbs: Option<PathBuf>,
    painter: Option<mpsc::Sender<(u64, ThumbJob)>>,
    /// Pictures written since the last frame: their textures are loaded again.
    fresh: Arc<Mutex<Vec<PathBuf>>>,
    /// Picture files still wanted, each with its newest job: a job finishing after its file
    /// was removed from the list, or after a newer job for it, writes nothing (checked and
    /// written under this lock), so Remove sticks and the last open's page wins.
    wanted: Arc<Mutex<HashMap<PathBuf, u64>>>,
    /// The number the next picture job gets.
    next_job: u64,
    /// By picture file; `None` for none (yet).
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// Files not found when the page last looked.
    missing: HashSet<PathBuf>,
    /// The page was drawn in the last frame.
    shown: bool,
    /// When the page's last look for moved or deleted files returned (`egui` time).
    checked: f64,
    /// The one thread that looks for moved or deleted files (a slow disk or network mount
    /// holds it up, not the window), spawned with the first look.
    looker: Option<Looker>,
    /// The generation of the look the thread is on. No other is asked for until it
    /// answers, so a `stat` that hangs holds up that one thread and nothing piles up.
    asked: Option<u64>,
    /// Goes up whenever the page learns about the disk outside a look (a click, a load, a
    /// failed open) and when the page appears: an answer asked for under an older one is
    /// dropped, as it may be from before.
    generation: u64,
    /// Look as soon as the thread is free (set with each new generation).
    stale: bool,
    /// Whether a listed file is there: `Path::is_file`, as a click checks (tests put in a
    /// slow one).
    probe: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
    pub filter: String,
    /// The space colour of the card under the pointer: the window takes it (`App::tint_now`).
    pub hover: Option<theme::Colour>,
}

impl std::fmt::Debug for Recents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recents")
            .field("entries", &self.entries.len())
            .field("saved", &self.file.is_some())
            .finish_non_exhaustive()
    }
}

impl Recents {
    /// A list that only lives in memory, without pictures.
    pub fn memory() -> Self {
        Self {
            entries: Vec::new(),
            file: None,
            thumbs: None,
            painter: None,
            fresh: Arc::default(),
            wanted: Arc::default(),
            next_job: 0,
            textures: HashMap::new(),
            missing: HashSet::new(),
            shown: false,
            checked: 0.0,
            looker: None,
            asked: None,
            generation: 0,
            stale: false,
            probe: Arc::new(Path::is_file),
            filter: String::new(),
            hover: None,
        }
    }

    /// The list of this computer, in the config folder.
    pub fn load() -> Self {
        match dirs::config_dir() {
            Some(dir) => {
                let dir = dir.join("figstudio");
                Self::at(dir.join("recents.json"), Some(dir.join("thumbs")))
            }
            None => Self::memory(),
        }
    }

    /// The list saved at `file`, pictures in `thumbs`. A list that cannot be read or does
    /// not parse is left as it is, and this session's list stays in memory.
    pub fn at(file: PathBuf, thumbs: Option<PathBuf>) -> Self {
        use std::io::Read as _;
        let read = std::fs::File::open(&file).and_then(|f| {
            let mut bytes = Vec::new();
            f.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(std::io::Error::other("too big"));
            }
            Ok(bytes)
        });
        let entries = match read {
            Ok(bytes) => parse(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
            Err(_) => None,
        };
        let mut recents = Self::memory();
        match entries {
            Some(entries) => {
                recents.entries = entries;
                recents.file = Some(file);
                recents.thumbs = thumbs;
            }
            None => tracing::warn!("recent files not read, kept in memory this time"),
        }
        recents
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_missing(&self, path: &Path) -> bool {
        self.missing.contains(path)
    }

    /// `path` was opened at `now`: it moves to the front.
    pub fn note(&mut self, path: &Path, key: Option<&str>, now: u64) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.entries.retain(|e| e.path != path);
        if path.is_file() {
            self.missing.remove(&path);
        } else {
            self.missing.insert(path.clone());
        }
        self.entries.insert(
            0,
            Entry {
                path,
                opened: now,
                key: key.map(str::to_owned),
            },
        );
        let dropped: Vec<Entry> = self
            .entries
            .drain(MAX_ENTRIES.min(self.entries.len())..)
            .collect();
        for e in dropped {
            self.forget_thumb(&e.path);
        }
        self.save();
        self.look_again();
    }

    /// Marks `path` as moved or deleted (`true`) or back in place.
    pub fn set_missing(&mut self, path: &Path, missing: bool) {
        if missing {
            self.missing.insert(path.to_path_buf());
        } else {
            self.missing.remove(path);
        }
        self.look_again();
    }

    /// What the page knows changed outside a look: the look running now (begun before it)
    /// is not believed, and the next one starts once the thread is free.
    fn look_again(&mut self) {
        self.generation += 1;
        self.stale = true;
    }

    /// Opening `path` failed: a listed file that is not there any more is marked missing.
    pub fn load_failed(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if !path.is_file() && self.entries.iter().any(|e| e.path == path) {
            self.missing.insert(path);
            self.look_again();
        }
    }

    /// Takes `path` off the list, with its picture.
    pub fn remove(&mut self, path: &Path) {
        self.entries.retain(|e| e.path != path);
        self.forget_thumb(path);
        self.save();
    }

    /// Where the picture of `path` is kept.
    pub fn thumb_file(&self, path: &Path) -> Option<PathBuf> {
        use sha2::Digest as _;
        let hash = sha2::Sha256::digest(path.to_string_lossy().as_bytes());
        let name: String = hash[..12].iter().map(|b| format!("{b:02x}")).collect();
        self.thumbs
            .as_ref()
            .map(|dir| dir.join(format!("{name}.png")))
    }

    fn forget_thumb(&mut self, path: &Path) {
        if let Some(file) = self.thumb_file(path) {
            self.textures.remove(&file);
            // Under the lock: a job writing it now finishes first, or finds it unwanted.
            let mut wanted = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
            wanted.remove(&file);
            let _ = std::fs::remove_file(file);
        }
    }

    /// Draws the picture of `path` in the background (one file at a time, on one thread).
    pub fn paint_thumb(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        job: impl FnOnce(PathBuf) -> ThumbJob,
    ) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let Some(out) = self.thumb_file(&path) else {
            return;
        };
        if self.painter.is_none() {
            let (tx, rx) = mpsc::channel::<(u64, ThumbJob)>();
            let fresh = self.fresh.clone();
            let wanted = self.wanted.clone();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("thumbnails".into())
                .spawn(move || {
                    for (number, job) in rx {
                        // As in `Workers::export`: a picture that panics is skipped, not the
                        // thread; it only reads an immutable document.
                        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            write_thumb(&job, number, &wanted)
                        }));
                        match done {
                            Ok(Ok(false)) => {}
                            Ok(Ok(true)) => {
                                if let Ok(mut fresh) = fresh.lock() {
                                    fresh.push(job.out);
                                }
                                ctx.request_repaint();
                            }
                            Ok(Err(e)) => tracing::warn!("no picture for a recent file: {e:#}"),
                            Err(_) => tracing::warn!("no picture for a recent file: it panicked"),
                        }
                    }
                });
            if spawned.is_err() {
                return;
            }
            self.painter = Some(tx);
        }
        let number = self.want(&out);
        if let Some(tx) = &self.painter
            && tx.send((number, job(out))).is_err()
        {
            self.painter = None;
        }
    }

    /// Marks picture file `out` as wanted from a new job, and returns that job's number.
    fn want(&mut self, out: &Path) -> u64 {
        self.next_job += 1;
        self.wanted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(out.to_path_buf(), self.next_job);
        self.next_job
    }

    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        match serde_json::to_vec_pretty(&self.entries) {
            Ok(bytes) => {
                if let Err(e) = spaces::write_private(file, &bytes) {
                    tracing::warn!("recent files not saved: {e}");
                }
            }
            Err(e) => tracing::warn!("recent files not saved: {e}"),
        }
    }

    /// The texture of `path`'s picture, loading at most `budget` more this frame.
    fn texture(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        budget: &mut usize,
    ) -> Option<egui::TextureHandle> {
        let file = self.thumb_file(path)?;
        if let Some(t) = self.textures.get(&file) {
            return t.clone();
        }
        if *budget == 0 {
            ctx.request_repaint();
            return None;
        }
        *budget -= 1;
        let texture = (|| {
            let len = std::fs::metadata(&file).ok()?.len();
            if len > MAX_THUMB_BYTES {
                return None;
            }
            let bytes = std::fs::read(&file).ok()?;
            let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let colour = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            Some(ctx.load_texture(
                format!("recent-{}", file.display()),
                colour,
                egui::TextureOptions::LINEAR,
            ))
        })();
        self.textures.insert(file, texture.clone());
        texture
    }

    /// Called each frame the page shows: on appearing, and [`RECHECK`] seconds after the
    /// last look returned while it stays up (a frame comes with any pointer move), it looks
    /// for moved or deleted files on another thread, so a card, its label and its menu
    /// follow the disk and a slow path never holds up the window; a freshly drawn picture
    /// is loaded again.
    fn begin_frame(&mut self, ctx: &egui::Context, time: f64) {
        if !self.shown {
            self.look_again();
        }
        let mut answers = Vec::new();
        let mut gone = false;
        if let Some(looker) = &self.looker {
            loop {
                match looker.answers.try_recv() {
                    Ok(answer) => answers.push(answer),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        gone = true;
                        break;
                    }
                }
            }
        }
        for (generation, missing) in answers {
            self.asked = None;
            self.checked = time;
            if generation == self.generation {
                self.missing = missing;
            }
        }
        if gone {
            // The thread panicked: a new one with the next look.
            self.looker = None;
            self.asked = None;
            self.checked = time;
        }
        if self.asked.is_none() && (self.stale || time - self.checked >= RECHECK) {
            self.stale = false;
            self.look(ctx, time);
        }
        self.shown = true;
        let fresh: Vec<PathBuf> = self
            .fresh
            .lock()
            .map(|mut f| std::mem::take(&mut *f))
            .unwrap_or_default();
        for out in fresh {
            self.textures.remove(&out);
        }
    }

    /// Asks the looking thread (spawned the first time) to look for moved or deleted files.
    fn look(&mut self, ctx: &egui::Context, time: f64) {
        if self.looker.is_none() {
            let (ask, asks) = mpsc::channel::<(u64, Vec<PathBuf>)>();
            let (tell, answers) = mpsc::channel();
            let probe = self.probe.clone();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("recents-look".into())
                .spawn(move || {
                    for (generation, paths) in asks {
                        let missing: HashSet<PathBuf> =
                            paths.into_iter().filter(|p| !probe(p)).collect();
                        if tell.send((generation, missing)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
            if spawned.is_err() {
                // No thread: try again after the usual wait, not every frame.
                self.checked = time;
                return;
            }
            self.looker = Some(Looker { ask, answers });
        }
        let paths: Vec<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
        let sent = self
            .looker
            .as_ref()
            .is_some_and(|l| l.ask.send((self.generation, paths)).is_ok());
        if sent {
            self.asked = Some(self.generation);
        } else {
            self.looker = None;
            self.checked = time;
        }
    }

    /// The page was not drawn this frame (a file is open).
    pub fn hidden(&mut self) {
        self.shown = false;
        self.hover = None;
    }
}

/// The entries in a saved list, skipping damaged ones; `None` when it is not a JSON list.
fn parse(bytes: &[u8]) -> Option<Vec<Entry>> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Some(Vec::new());
    }
    let items: Vec<serde_json::Value> = serde_json::from_slice(bytes).ok()?;
    let mut entries: Vec<Entry> = items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .filter(|e: &Entry| e.path.is_absolute() && e.key.as_ref().is_none_or(|k| k.len() <= 64))
        .collect();
    entries.truncate(MAX_ENTRIES);
    Some(entries)
}

/// Renders the job's page to fit [`THUMB_SIZE`] over the page's colour and saves it, if job
/// `number` is still the one `wanted` for its file (false: the file was removed, or opened
/// again since; nothing written).
fn write_thumb(
    job: &ThumbJob,
    number: u64,
    wanted: &Mutex<HashMap<PathBuf, u64>>,
) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    let png = thumb_png(job)?;
    let wanted = wanted.lock().unwrap_or_else(|e| e.into_inner());
    if wanted.get(&job.out) != Some(&number) {
        return Ok(false);
    }
    spaces::write_private(&job.out, &png).context("save the picture")?;
    Ok(true)
}

/// The PNG of a job's page, at most [`THUMB_SIZE`].
pub fn thumb_png(job: &ThumbJob) -> anyhow::Result<Vec<u8>> {
    use anyhow::Context as _;
    let bounds =
        figstudio_render::content_bounds(&job.doc, job.page).context("the page is empty")?;
    let (w, h) = THUMB_SIZE;
    let pad = 24.0;
    let scale = ((w - pad * 2.0) / bounds.width().max(1.0))
        .min((h - pad * 2.0) / bounds.height().max(1.0))
        .min(2.0);
    let background = figstudio_render::page_background(&job.doc, job.page);
    let frame = figstudio_render::Renderer::new()
        .render_fit_complete(&job.doc, job.page, scale, pad, Some(background), |want| {
            job.source.as_ref().and_then(|s| s.read(&want.entry).ok())
        })
        .context("nothing to draw")?;
    frame.pixmap.into_png().context("encode the picture")
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// "Opened 3 days ago", from `then` to `now` (seconds).
pub fn ago(now: u64, then: u64) -> String {
    const MIN: u64 = 60;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    let s = now.saturating_sub(then);
    let (n, unit) = match s {
        s if s < MIN => return "Opened just now".into(),
        s if s < HOUR => (s / MIN, "minute"),
        s if s < DAY => (s / HOUR, "hour"),
        s if s < 2 * DAY => return "Opened yesterday".into(),
        s if s < 14 * DAY => (s / DAY, "day"),
        s if s < 60 * DAY => (s / (7 * DAY), "week"),
        s if s < 365 * DAY => (s / (30 * DAY), "month"),
        s => (s / (365 * DAY), "year"),
    };
    let plural = if n == 1 { "" } else { "s" };
    format!("Opened {n} {unit}{plural} ago")
}

/// The folder of `path` for showing, with the home folder as `~`.
fn folder_label(path: &Path) -> String {
    let Some(parent) = path.parent() else {
        return String::new();
    };
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = parent.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", rest.display())
        };
    }
    parent.display().to_string()
}

/// What one card or row shows.
struct Card {
    path: PathBuf,
    colour: theme::Colour,
    name: String,
    when: String,
    style: Style,
    tint: Tint,
    missing: bool,
    texture: Option<egui::TextureHandle>,
}

impl App {
    /// The Recents page in the canvas card. With nothing opened before it is the welcome
    /// block under the title.
    pub(crate) fn recents_page(&mut self, ui: &mut Ui, card: Rect) -> Option<Pick> {
        let ctx = ui.ctx().clone();
        let p = theme::of(ui);
        self.recents.begin_frame(&ctx, ctx.input(|i| i.time));
        self.recents.hover = None;
        let inner = card.shrink2(vec2(32.0, 26.0));
        let header = Rect::from_min_size(inner.min, vec2(inner.width(), 34.0));
        let mut title = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(header.min, vec2(280.0, 58.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        title.spacing_mut().item_spacing.y = 2.0;
        title.add_space(2.0);
        title.add(
            egui::Label::new(
                egui::RichText::new("Recents")
                    .font(theme::semibold(22.0))
                    .color(p.text),
            )
            .selectable(false),
        );
        if self.recents.entries().is_empty() {
            title.add(
                egui::Label::new(
                    egui::RichText::new("Files you open show up here.")
                        .size(13.0)
                        .color(p.text_dim),
                )
                .selectable(false),
            );
            return self.empty_state(ui, card).then_some(Pick::OpenDialog);
        }
        let mut pick = None;
        // From the right: Open file…, the view toggle, the filter.
        let tint = self.tint_now(&ctx);
        let keys = self
            .prefs
            .keymap
            .get(crate::keymap::Binding::Open)
            .map(|s| ctx.format_shortcut(&s))
            .unwrap_or_default();
        // From the right, in the room beside the title: Open file…, the view toggle, the
        // filter. A narrow card drops the filter, then the toggle, then narrows Open.
        let room = (inner.width() - HEADER_TITLE).max(0.0);
        let open_w = room.min(OPEN_W);
        let open = Rect::from_min_size(
            pos2(header.right() - open_w, header.top()),
            vec2(open_w, 34.0),
        );
        if open_w >= OPEN_MIN {
            let label = with_keys("Open file…", &keys);
            let text = if open_w < OPEN_W - 20.0 {
                "Open…"
            } else {
                "Open file…"
            };
            if accent_button(ui, open, "recents-open", text, &label, tint).clicked() {
                pick = Some(Pick::OpenDialog);
            }
        }
        let list = self.prefs.recents_list;
        let toggle = Rect::from_min_size(
            open.left_top() - vec2(14.0 + TOGGLE_W, 0.0),
            vec2(TOGGLE_W, 34.0),
        );
        if room >= OPEN_W + 14.0 + TOGGLE_W {
            ui.painter()
                .rect_filled(toggle, CornerRadius::same(10), p.well);
            let mut toggle_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(toggle.shrink(3.0))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            toggle_ui.spacing_mut().item_spacing.x = 2.0;
            if icons::button(&mut toggle_ui, Icon::Grid, "Grid view", !list).clicked() {
                self.prefs.recents_list = false;
            }
            if icons::button(&mut toggle_ui, Icon::List, "List view", list).clicked() {
                self.prefs.recents_list = true;
            }
        }
        let filter_w = (room - OPEN_W - 14.0 - TOGGLE_W - 12.0).min(240.0);
        if filter_w >= 120.0 {
            let filter = Rect::from_min_size(
                toggle.left_top() - vec2(12.0 + filter_w, 0.0) + vec2(0.0, 3.0),
                vec2(filter_w, 28.0),
            );
            let mut filter_ui = ui.new_child(UiBuilder::new().max_rect(filter));
            crate::chrome::filter_field(
                &mut filter_ui,
                &mut self.recents.filter,
                "Filter recents",
                p.well,
            );
        }

        let needle = self.recents.filter.trim().to_lowercase();
        let now = now();
        let dark = p.dark;
        let entries: Vec<Entry> = self.recents.entries().to_vec();
        let mut budget = LOADS_PER_FRAME;
        let mut cards = Vec::new();
        for e in entries {
            let style = e.style(&self.spaces);
            let file = e.file_name();
            let name = style
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| file.clone());
            if !needle.is_empty()
                && !name.to_lowercase().contains(&needle)
                && !file.to_lowercase().contains(&needle)
            {
                continue;
            }
            let colour = style
                .colour
                .unwrap_or(theme::Colour::Preset(theme::pick_tint(&file, [])));
            let texture = self.recents.texture(&ctx, &e.path, &mut budget);
            cards.push(Card {
                colour,
                missing: self.recents.is_missing(&e.path),
                when: ago(now, e.opened),
                tint: theme::space_tint(colour, dark),
                path: e.path,
                name,
                style,
                texture,
            });
        }

        let body = Rect::from_min_max(header.left_bottom() + vec2(0.0, 22.0), inner.right_bottom());
        let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
        if cards.is_empty() {
            body_ui.put(
                Rect::from_center_size(
                    pos2(body.center().x, body.top() + 80.0),
                    vec2(body.width(), 24.0),
                ),
                egui::Label::new(
                    egui::RichText::new("No recent files match")
                        .size(14.0)
                        .color(p.text_dim),
                )
                .selectable(false),
            );
            return pick;
        }
        egui::ScrollArea::vertical()
            .id_salt("recents-scroll")
            .auto_shrink([false, false])
            .show(&mut body_ui, |ui| {
                let mut hovered = None;
                let picked = if list {
                    list_rows(ui, &cards, &mut hovered)
                } else {
                    grid(ui, &cards, &mut hovered)
                };
                self.recents.hover = hovered;
                pick = picked.or(pick.take());
            });
        pick
    }
}

impl App {
    /// Lists tab `i`'s file and queues the picture of the page it shows. Called once the
    /// file is loaded and any link has moved it to its node's page. Bytes dropped without a
    /// file (only a name) are not listed: there is nothing to open again.
    pub(crate) fn note_recent(&mut self, ctx: &egui::Context, i: usize) {
        let Some(tab) = self.tabs.get(i) else {
            return;
        };
        if !tab.path.is_file() {
            return;
        }
        let (path, doc, page, source) = (
            tab.path.clone(),
            tab.doc.clone(),
            tab.page,
            tab.source.clone(),
        );
        self.recents.note(&path, tab.doc_key.as_deref(), now());
        self.recents.paint_thumb(ctx, &path, |out| ThumbJob {
            doc,
            page,
            source,
            out,
        });
    }

    /// Carries out what was picked on the Recents page (Open file… is the caller's).
    pub(crate) fn recent_picked(&mut self, ctx: &egui::Context, pick: Pick) {
        match pick {
            // What is on disk now decides, not when the page last looked.
            Pick::Open(path) if path.is_file() => {
                self.recents.set_missing(&path, false);
                self.open(path);
            }
            Pick::Open(path) => {
                self.recents.set_missing(&path, true);
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.toast(
                    ctx,
                    format!("{name} was moved or deleted. Remove it from recents, or open it from where it is now."),
                    true,
                );
            }
            Pick::Reveal(path) => {
                if let Err(e) = opener::reveal(&path) {
                    self.toast(ctx, format!("Could not show the file: {e}"), true);
                }
            }
            Pick::Remove(path) => self.recents.remove(&path),
            Pick::OpenDialog => self.open_dialog(ctx),
        }
    }
}

/// The cards in as many columns as fit.
fn grid(ui: &mut Ui, cards: &[Card], hovered: &mut Option<theme::Colour>) -> Option<Pick> {
    let width = ui.available_width();
    let cols = ((width + CARD_GAP) / (CARD_MIN + CARD_GAP))
        .floor()
        .clamp(1.0, 6.0) as usize;
    let w = ((width - CARD_GAP * (cols - 1) as f32) / cols as f32).floor();
    let preview = (w * 0.62).round();
    let cell = preview + 56.0;
    let rows = cards.len().div_ceil(cols);
    // Room for the lift and shadow of the top row.
    let (area, _) = ui.allocate_exact_size(
        vec2(width, rows as f32 * (cell + CARD_GAP) + 4.0),
        Sense::hover(),
    );
    let mut pick = None;
    for (i, card) in cards.iter().enumerate() {
        let (row, col) = (i / cols, i % cols);
        let min = area.min
            + vec2(
                col as f32 * (w + CARD_GAP),
                4.0 + row as f32 * (cell + CARD_GAP),
            );
        let rect = Rect::from_min_size(min, vec2(w, cell));
        if !ui.is_rect_visible(rect) {
            continue;
        }
        if let Some(p) = grid_card(ui, rect, preview, card, hovered) {
            pick = Some(p);
        }
    }
    pick
}

fn grid_card(
    ui: &mut Ui,
    rect: Rect,
    preview: f32,
    card: &Card,
    hovered: &mut Option<theme::Colour>,
) -> Option<Pick> {
    let p = theme::of(ui);
    let response = card_response(ui, rect, card, hovered);
    let hover =
        ui.ctx()
            .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.14);
    let lift = hover * 3.0;
    let top = Rect::from_min_size(rect.min - vec2(0.0, lift), vec2(rect.width(), preview));
    let radius = 12.0;
    let painter = ui.painter();
    painter.add(card_shadow(&p, (8.0 + hover * 14.0) as u8).as_shape(top, CornerRadius::same(12)));
    let t = card.tint;
    painter.add(Shape::mesh(crate::chrome::rounded_gradient(
        top,
        radius,
        [t.glow, t.top, t.bottom, t.top],
    )));
    paint_picture(painter, &p, top.shrink(16.0), card, 6);
    painter.rect_stroke(
        top,
        CornerRadius::same(12),
        Stroke::new(1.0 + hover, t.accent.gamma_multiply(0.25 + 0.75 * hover)),
        StrokeKind::Inside,
    );
    // Under the picture: the badge, the name, when it was opened.
    let text_top = top.bottom() + 12.0;
    let badge = Rect::from_min_size(pos2(rect.left() + 2.0, text_top + 1.0), vec2(16.0, 16.0));
    paint_badge(painter, badge, card.style.symbol.as_ref(), t.accent, p.text);
    let x = badge.right() + 8.0;
    let name_w = rect.right() - x;
    let name = elided(ui, &card.name, theme::semibold(13.5), p.text, name_w);
    ui.painter().galley(pos2(x, text_top), name, p.text);
    let (when, colour) = if card.missing {
        ("Missing: moved or deleted".to_owned(), p.warning)
    } else {
        (card.when.clone(), p.text_dim)
    };
    let when = elided(ui, &when, egui::FontId::proportional(12.0), colour, name_w);
    ui.painter().galley(pos2(x, text_top + 20.0), when, colour);
    menu_and_click(&response, card)
}

/// One row per file: a small picture, the name and folder, when it was opened.
fn list_rows(ui: &mut Ui, cards: &[Card], hovered: &mut Option<theme::Colour>) -> Option<Pick> {
    let width = ui.available_width();
    let (area, _) = ui.allocate_exact_size(vec2(width, cards.len() as f32 * ROW), Sense::hover());
    let mut pick = None;
    for (i, card) in cards.iter().enumerate() {
        let rect = Rect::from_min_size(area.min + vec2(0.0, i as f32 * ROW), vec2(width, ROW));
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let p = theme::of(ui);
        let response = card_response(ui, rect, card, hovered);
        let hover =
            ui.ctx()
                .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.1);
        let painter = ui.painter();
        painter.rect_filled(
            rect.shrink2(vec2(0.0, 2.0)),
            CornerRadius::same(10),
            p.well.gamma_multiply(hover),
        );
        let tile = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(56.0, ROW - 16.0));
        let t = card.tint;
        painter.add(Shape::mesh(crate::chrome::rounded_gradient(
            tile,
            7.0,
            [t.glow, t.top, t.bottom, t.top],
        )));
        paint_picture(painter, &p, tile.shrink(4.0), card, 3);
        let x = tile.right() + 14.0;
        let when_w = 170.0_f32.min(rect.width() * 0.3);
        let name_w = rect.right() - when_w - 16.0 - x;
        let name = elided(ui, &card.name, theme::semibold(13.5), p.text, name_w);
        ui.painter()
            .galley(pos2(x, rect.center().y - 17.0), name, p.text);
        let folder = elided(
            ui,
            &folder_label(&card.path),
            egui::FontId::proportional(12.0),
            p.text_dim,
            name_w,
        );
        ui.painter()
            .galley(pos2(x, rect.center().y + 2.0), folder, p.text_dim);
        let (when, colour) = if card.missing {
            ("Missing".to_owned(), p.warning)
        } else {
            (
                card.when.trim_start_matches("Opened ").to_owned(),
                p.text_dim,
            )
        };
        ui.painter().text(
            pos2(rect.right() - 16.0, rect.center().y),
            Align2::RIGHT_CENTER,
            when,
            egui::FontId::proportional(12.0),
            colour,
        );
        if let Some(p) = menu_and_click(&response, card) {
            pick = Some(p);
        }
    }
    pick
}

/// The clickable area of a card or row, named for screen readers. Notes its colour in
/// `hovered` while the pointer is on it.
fn card_response(
    ui: &mut Ui,
    rect: Rect,
    card: &Card,
    hovered: &mut Option<theme::Colour>,
) -> Response {
    let response = ui.interact(rect, Id::new(("recent", &card.path)), Sense::click());
    let label = if card.missing {
        format!("{} (missing)", card.name)
    } else {
        card.name.clone()
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        *hovered = Some(card.colour);
    }
    response
}

/// The picture inside `area`, or the space's badge while there is none.
fn paint_picture(painter: &egui::Painter, p: &theme::Palette, area: Rect, card: &Card, radius: u8) {
    let fade = if card.missing { 0.45 } else { 1.0 };
    match &card.texture {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = (area.width() / size.x).min(area.height() / size.y);
            let rect = Rect::from_center_size(area.center(), size * scale);
            painter.add(card_shadow(p, 10).as_shape(rect, CornerRadius::same(radius)));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(
                egui::epaint::RectShape::filled(
                    rect,
                    CornerRadius::same(radius),
                    Color32::WHITE.gamma_multiply(fade),
                )
                .with_texture(texture.id(), uv),
            );
        }
        None => {
            let side = (area.height() * 0.42).clamp(10.0, 44.0);
            let badge = Rect::from_center_size(area.center(), vec2(side, side));
            let on = theme::on_accent(card.tint.top);
            paint_badge(
                painter,
                badge,
                card.style.symbol.as_ref(),
                card.tint.accent.gamma_multiply(fade),
                on.gamma_multiply(fade),
            );
        }
    }
}

/// A click opens the file; the right-click menu opens, shows or forgets it.
fn menu_and_click(response: &Response, card: &Card) -> Option<Pick> {
    let mut pick = None;
    if response.clicked() {
        pick = Some(Pick::Open(card.path.clone()));
    }
    context_menu(response, |ui| {
        ui.set_min_width(190.0);
        if ui
            .add_enabled(!card.missing, egui::Button::new("Open"))
            .clicked()
        {
            pick = Some(Pick::Open(card.path.clone()));
        }
        let reveal = if cfg!(target_os = "macos") {
            "Show in Finder"
        } else {
            "Show in folder"
        };
        if ui
            .add_enabled(!card.missing, egui::Button::new(reveal))
            .clicked()
        {
            pick = Some(Pick::Reveal(card.path.clone()));
        }
        ui.separator();
        if ui.button("Remove from recents").clicked() {
            pick = Some(Pick::Remove(card.path.clone()));
        }
    });
    pick
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("figstudio-recents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Steps `r`'s page (frame time `time`) until the look it is waiting on has answered.
    fn settle(r: &mut Recents, ctx: &egui::Context, time: f64) {
        let waiting = r.asked;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while waiting.is_some() && r.asked == waiting {
            assert!(
                std::time::Instant::now() < deadline,
                "the look never returned"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
            r.begin_frame(ctx, time);
        }
    }

    /// A probe that waits on `gate` after it looked, counting its calls in `looks`.
    fn held_probe(
        looks: &Arc<std::sync::atomic::AtomicUsize>,
        gate: &Arc<Mutex<()>>,
    ) -> Arc<dyn Fn(&Path) -> bool + Send + Sync> {
        let (l, g) = (looks.clone(), gate.clone());
        Arc::new(move |p: &Path| {
            let there = p.is_file();
            l.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(g.lock());
            there
        })
    }

    #[test]
    fn a_slow_disk_neither_holds_up_the_page_nor_piles_up_looks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ctx = egui::Context::default();
        let mut r = Recents::memory();
        let slow = Path::new("/slow/a.fig");
        r.note(slow, None, 1);
        let looks = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        r.probe = held_probe(&looks, &gate);
        let started = std::time::Instant::now();
        r.begin_frame(&ctx, 0.0);
        // Frames go on while the disk hangs, long past the wait, and clicks and loads that
        // drop the running look ask for no second one while it is stuck.
        for t in [0.5, 2.0, 5.0, 9.0] {
            r.begin_frame(&ctx, t);
            r.set_missing(slow, true);
            r.note(Path::new("/slow/b.fig"), None, 2);
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "the page waited"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            looks.load(Ordering::SeqCst),
            1,
            "one look at a time, one thread"
        );
        drop(held);
        settle(&mut r, &ctx, 9.5);
        // That answer was from before the clicks: dropped, and a new look asked right away.
        assert!(r.asked.is_some(), "the look after the clicks");
        settle(&mut r, &ctx, 9.5);
        assert!(r.is_missing(slow));
        // The wait counts from when the look returned, not when it began.
        r.begin_frame(&ctx, 10.0);
        assert!(r.asked.is_none(), "no look right after one returned");
        r.begin_frame(&ctx, 10.6);
        assert!(r.asked.is_some(), "the next look, a second later");
        settle(&mut r, &ctx, 10.7);
    }

    #[test]
    fn a_look_begun_before_a_file_was_opened_does_not_undo_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ctx = egui::Context::default();
        let dir = temp_dir("older");
        let path = dir.join("back.fig");
        let mut r = Recents::memory();
        r.note(&path, None, 1);
        assert!(r.is_missing(&path));
        let looks = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        r.probe = held_probe(&looks, &gate);
        r.begin_frame(&ctx, 0.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while looks.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "the look never began");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // The look saw it gone; then it is put back and opened from the dialog.
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/structure.fig"
            ),
            &path,
        )
        .unwrap();
        r.note(&path, None, 2);
        assert!(!r.is_missing(&path));
        drop(held);
        settle(&mut r, &ctx, 0.1);
        assert!(
            !r.is_missing(&path),
            "the older look's answer is dropped, not stored"
        );
        settle(&mut r, &ctx, 0.2);
        assert!(!r.is_missing(&path), "the new look agrees");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_where_the_file_was_counts_as_missing() {
        let ctx = egui::Context::default();
        let dir = temp_dir("folder");
        let path = dir.join("now-a-folder.fig");
        std::fs::create_dir_all(&path).unwrap();
        let mut r = Recents::memory();
        r.note(&path, None, 1);
        assert!(r.is_missing(&path), "noted as missing");
        r.begin_frame(&ctx, 0.0);
        settle(&mut r, &ctx, 0.1);
        assert!(
            r.is_missing(&path),
            "a look agrees with a click: not a file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_file_again_moves_it_to_the_front_once() {
        let mut r = Recents::memory();
        r.note(Path::new("/a/one.fig"), Some("k1"), 10);
        r.note(Path::new("/a/two.fig"), None, 20);
        r.note(Path::new("/a/one.fig"), Some("k1"), 30);
        let paths: Vec<_> = r
            .entries()
            .iter()
            .map(|e| (e.path.clone(), e.opened))
            .collect();
        assert_eq!(
            paths,
            [
                (PathBuf::from("/a/one.fig"), 30),
                (PathBuf::from("/a/two.fig"), 20)
            ]
        );
    }

    #[test]
    fn the_list_is_capped_and_the_oldest_pictures_go_with_it() {
        let dir = temp_dir("cap");
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let first = PathBuf::from("/files/0.fig");
        let thumb = r.thumb_file(&first).unwrap();
        std::fs::write(&thumb, b"png").unwrap();
        for i in 0..=MAX_ENTRIES {
            r.note(&PathBuf::from(format!("/files/{i}.fig")), None, i as u64);
        }
        assert_eq!(r.entries().len(), MAX_ENTRIES);
        assert!(r.entries().iter().all(|e| e.path != first));
        assert!(!thumb.exists(), "the dropped file's picture is deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_survives_a_restart_and_a_damaged_one_is_left_alone() {
        let dir = temp_dir("disk");
        let file = dir.join("recents.json");
        let mut r = Recents::at(file.clone(), None);
        r.note(Path::new("/x/a.fig"), Some("key"), 5);
        r.note(Path::new("/x/b.fig"), None, 6);
        let again = Recents::at(file.clone(), None);
        assert_eq!(again.entries(), r.entries());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only its owner reads the list");
        }
        // One bad entry is skipped, not the list; relative paths are not kept.
        std::fs::write(
            &file,
            r#"[{"path":"/x/a.fig","opened":1},{"path":7},{"path":"rel.fig","opened":2}]"#,
        )
        .unwrap();
        assert_eq!(Recents::at(file.clone(), None).entries().len(), 1);
        // Not a list at all: kept in memory, the file is not written over.
        std::fs::write(&file, "not json").unwrap();
        let mut damaged = Recents::at(file.clone(), None);
        damaged.note(Path::new("/x/c.fig"), None, 9);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_a_file_deletes_its_picture() {
        let dir = temp_dir("remove");
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        let path = Path::new("/y/a.fig");
        r.note(path, None, 1);
        let thumb = r.thumb_file(path).unwrap();
        std::fs::write(&thumb, b"png").unwrap();
        r.remove(path);
        assert!(r.entries().is_empty());
        assert!(!thumb.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opened_times_read_as_people_say_them() {
        let day = 86_400;
        assert_eq!(ago(100, 90), "Opened just now");
        assert_eq!(ago(10_000, 10_000 - 60), "Opened 1 minute ago");
        assert_eq!(ago(10_000, 10_000 - 7_200), "Opened 2 hours ago");
        assert_eq!(ago(10 * day, 9 * day), "Opened yesterday");
        assert_eq!(ago(10 * day, 7 * day), "Opened 3 days ago");
        assert_eq!(ago(100 * day, 79 * day), "Opened 3 weeks ago");
        assert_eq!(ago(400 * day, 310 * day), "Opened 3 months ago");
        assert_eq!(ago(1000 * day, 200 * day), "Opened 2 years ago");
        assert_eq!(
            ago(5, 10),
            "Opened just now",
            "a clock set back is not the future"
        );
    }

    fn structure_job(out: PathBuf) -> ThumbJob {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/structure.fig"
        );
        let doc = Arc::new(figstudio_fig::Document::open(path).unwrap());
        ThumbJob {
            page: doc.pages[0],
            doc,
            source: None,
            out,
        }
    }

    #[test]
    fn a_picture_finished_after_its_file_was_removed_is_not_written() {
        let dir = temp_dir("late");
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        let path = Path::new("/z/a.fig");
        r.note(path, None, 1);
        let out = r.thumb_file(path).unwrap();
        // Queued (wanted), then removed before the job got to it.
        let first = r.want(&out);
        r.remove(path);
        assert!(!write_thumb(&structure_job(out.clone()), first, &r.wanted).unwrap());
        assert!(!out.exists(), "Remove from recents sticks on disk");
        // Still wanted: written.
        r.note(path, None, 2);
        let second = r.want(&out);
        assert!(write_thumb(&structure_job(out.clone()), second, &r.wanted).unwrap());
        assert!(out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn of_two_jobs_for_one_file_only_the_newest_writes() {
        let dir = temp_dir("twice");
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let mut r = Recents::at(dir.join("recents.json"), Some(dir.join("thumbs")));
        let path = Path::new("/z/a.fig");
        r.note(path, None, 1);
        let out = r.thumb_file(path).unwrap();
        let older = r.want(&out);
        let newer = r.want(&out);
        assert!(write_thumb(&structure_job(out.clone()), newer, &r.wanted).unwrap());
        assert!(
            !write_thumb(&structure_job(out.clone()), older, &r.wanted).unwrap(),
            "the older job, finishing last, does not replace the newer picture"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_fits_the_card_size() {
        let png = thumb_png(&structure_job(PathBuf::new())).unwrap();
        let image = image::load_from_memory(&png).unwrap();
        let (w, h) = (f64::from(image.width()), f64::from(image.height()));
        assert!(
            w <= THUMB_SIZE.0 + 1.0 && h <= THUMB_SIZE.1 + 1.0,
            "{w}x{h}"
        );
        assert!(
            w >= THUMB_SIZE.0 - 2.0 || h >= THUMB_SIZE.1 - 2.0,
            "fills one side: {w}x{h}"
        );
    }
}
