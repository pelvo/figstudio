//! Recently opened files: the Recents page the canvas card shows when no file is open, as
//! Figma's file browser. Each file is a card in its space's gradient with a picture of the
//! page it opened on.
//!
//! Where the list and its pictures are kept is the platform's (`Io`): on the desktop a JSON
//! file and PNGs in the config folder (`recents_disk.rs`); in the browser IndexedDB, with
//! the file handle where the browser gives one (`recents_web.rs`). Neither leaves the
//! device.

use std::collections::HashSet;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{
    Align2, Color32, CornerRadius, Id as EguiId, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};
use serde::{Deserialize, Serialize};

use crate::App;
use crate::chrome::{accent_button, card_shadow, context_menu, elided, paint_badge, with_keys};
use crate::icons::{self, Icon};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use crate::recents_disk::Io;
#[cfg(target_arch = "wasm32")]
pub(crate) use crate::recents_web::Io;
use crate::spaces::{self, Style};
use crate::theme::{self, Tint};

/// Most files listed; the oldest go first (with their pictures).
/// Files listed beside the app (Dock menu, jump list).
#[cfg(any(target_os = "macos", windows, test))]
pub(crate) const SHORTLIST: usize = 5;
pub(crate) const MAX_ENTRIES: usize = 60;
/// A picture fits in this many pixels (two points per pixel on a card about 300 wide).
pub const THUMB_SIZE: (f64, f64) = (640.0, 400.0);
/// Pictures decoded per frame: the first frame of a long list does not stall.
const LOADS_PER_FRAME: usize = 3;
/// Narrowest card in the grid, and the gap between cards.
const CARD_MIN: f32 = 210.0;
const CARD_GAP: f32 = 20.0;
/// A list row's height.
const ROW: f32 = 54.0;
/// The header: room kept for the title, Open file…'s width (and the narrowest it gets
/// before it goes; ⌘O and the sidebar's + still open files), the view toggle's width.
const HEADER_TITLE: f32 = 150.0;
const OPEN_W: f32 = 124.0;
const OPEN_MIN: f32 = 72.0;
const TOGGLE_W: f32 = 66.0;

/// One file opened before.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// As its tab shows it: the full path on the desktop, the file's name in the browser.
    pub path: PathBuf,
    /// Browser only: its file handle's number in IndexedDB, where the browser gives handles
    /// (Chrome, Edge). The same file always has the same number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<u32>,
    /// When it was last opened, in seconds since the Unix epoch.
    pub opened: u64,
    /// Its content key (`Tab::doc_key`): finds its space's look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// Which file an entry is: its path, and in the browser its handle (two files with one
/// name are two entries when the browser can tell them apart).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Id {
    pub path: PathBuf,
    pub handle: Option<u32>,
}

impl Id {
    /// A file known by its path alone (the desktop).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            handle: None,
        }
    }
}

impl Entry {
    pub fn id(&self) -> Id {
        Id {
            path: self.path.clone(),
            handle: self.handle,
        }
    }

    /// The file name, as a tab shows it.
    pub fn file_name(&self) -> String {
        file_label(&self.path)
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
}

/// What was clicked on the Recents page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Open(Id),
    OpenDialog,
    /// Show in Finder (the desktop; the browser has no folder to show).
    #[cfg(not(target_arch = "wasm32"))]
    Reveal(Id),
    Remove(Id),
}

/// What the platform learnt since the last frame (`Io::begin_frame`).
#[derive(Debug, Default)]
pub(crate) struct News {
    /// The files not found by the look that just returned.
    pub missing: Option<HashSet<Id>>,
    /// The saved list, read in the background (the browser).
    pub entries: Option<Vec<Entry>>,
}

/// The list, its pictures and the page's state.
pub struct Recents {
    /// Newest first.
    entries: Vec<Entry>,
    /// Files not found when the page last looked or a click tried them.
    missing: HashSet<Id>,
    /// The page was drawn in the last frame.
    shown: bool,
    /// Where the list and pictures are kept.
    pub(crate) io: Io,
    pub filter: String,
    /// The space colour of the card under the pointer: the window takes it (`App::tint_now`).
    pub hover: Option<theme::Colour>,
    /// The saved list is still being read (the browser): no welcome yet.
    pub(crate) waiting: bool,
}

impl std::fmt::Debug for Recents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recents")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl Recents {
    /// A list that only lives in memory, without pictures.
    pub fn memory() -> Self {
        Self::with(Io::memory(), Vec::new())
    }

    /// The list of this device.
    pub fn load() -> Self {
        let (io, entries) = Io::load();
        let mut recents = Self::with(io, entries);
        // The browser's list arrives a few frames later (`take`).
        recents.waiting = cfg!(target_arch = "wasm32");
        recents
    }

    /// The list saved at `file`, pictures in `thumbs`. A list that cannot be read or does
    /// not parse is left as it is, and this session's list stays in memory.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn at(file: PathBuf, thumbs: Option<PathBuf>) -> Self {
        let (io, entries) = Io::at(file, thumbs);
        Self::with(io, entries)
    }

    /// Where the picture of the file at `path` is written.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn thumb_file(&self, path: &Path) -> Option<PathBuf> {
        self.io.thumb_file(&Id::file(path))
    }

    fn with(io: Io, entries: Vec<Entry>) -> Self {
        Self {
            entries,
            missing: HashSet::new(),
            shown: false,
            io,
            filter: String::new(),
            hover: None,
            waiting: false,
        }
    }

    /// `new` is `old`'s file, found moved and picked where it is now (the browser gives it a
    /// new handle): `old`'s card, if still missing, gives way. Only that card: two files may
    /// share a name. A card with a handle stays for a file without one (the plain picker),
    /// which could not open it again.
    #[cfg_attr(
        not(any(target_arch = "wasm32", test)),
        expect(dead_code, reason = "the browser picks a moved file again")
    )]
    pub fn replace_moved(&mut self, old: &Id, new: &Id) {
        let loses_handle = old.handle.is_some() && new.handle.is_none();
        if old.path != new.path || old == new || loses_handle || !self.missing.contains(old) {
            return;
        }
        self.entries.retain(|e| e.id() != *old);
        self.missing.remove(old);
        self.io.forget(old);
        self.io.save(&self.entries);
        self.io.changed();
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_missing(&self, id: &Id) -> bool {
        self.missing.contains(id)
    }

    /// The files listed beside the app (the macOS Dock menu, the Windows jump list): the
    /// newest [`SHORTLIST`] not known to be missing.
    #[cfg(any(target_os = "macos", windows, test))]
    pub fn shortlist(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|e| !self.missing.contains(&e.id()))
            .take(SHORTLIST)
            .map(|e| e.path.clone())
            .collect()
    }

    /// The file at `path` was opened at `now`: it moves to the front.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn note(&mut self, path: &Path, key: Option<&str>, now: u64) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let there = path.is_file();
        self.add(Id::file(path), key, now, there);
    }

    /// `id` was opened at `now` (`there`: it is where it was opened from): it moves to the
    /// front, and the oldest beyond [`MAX_ENTRIES`] go with their pictures.
    pub(crate) fn add(&mut self, id: Id, key: Option<&str>, now: u64, there: bool) {
        self.entries.retain(|e| e.id() != id);
        if there {
            self.missing.remove(&id);
        } else {
            self.missing.insert(id.clone());
        }
        self.entries.insert(
            0,
            Entry {
                path: id.path,
                handle: id.handle,
                opened: now,
                key: key.map(str::to_owned),
            },
        );
        let dropped: Vec<Entry> = self
            .entries
            .drain(MAX_ENTRIES.min(self.entries.len())..)
            .collect();
        for e in dropped {
            self.io.forget(&e.id());
        }
        self.io.save(&self.entries);
        self.io.changed();
    }

    /// Marks `id` as moved or deleted (`true`) or back in place.
    pub fn set_missing(&mut self, id: &Id, missing: bool) {
        if missing {
            self.missing.insert(id.clone());
        } else {
            self.missing.remove(id);
        }
        self.io.changed();
    }

    /// Opening `path` failed: a listed file that is not there any more is marked missing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_failed(&mut self, path: &Path) {
        let id = Id::file(std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
        if !id.path.is_file() && self.entries.iter().any(|e| e.id() == id) {
            self.missing.insert(id);
            self.io.changed();
        }
    }

    /// Takes `id` off the list, with its picture.
    pub fn remove(&mut self, id: &Id) {
        self.entries.retain(|e| e.id() != *id);
        self.missing.remove(id);
        self.io.forget(id);
        self.io.save(&self.entries);
    }

    /// Draws the picture of `id` in the background.
    pub fn paint_thumb(&mut self, ctx: &egui::Context, id: &Id, job: ThumbJob) {
        self.io.paint(ctx, id, job);
    }

    /// Called each frame the page shows: takes what the platform learnt (the files not
    /// found, a list read in the background).
    pub(crate) fn begin_frame(&mut self, ctx: &egui::Context, time: f64) {
        let appeared = !self.shown;
        self.shown = true;
        let news = self.io.begin_frame(ctx, time, appeared, &self.entries);
        self.take(news);
    }

    /// Takes what the platform learnt while the page is not drawn (every frame). The
    /// browser's pool cannot wake the window, so it is drawn again shortly while a picture
    /// is being drawn.
    pub fn sync(&mut self, ctx: &egui::Context) {
        let news = self.io.poll();
        self.take(news);
        if self.io.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub(crate) fn take(&mut self, news: News) {
        if let Some(missing) = news.missing {
            self.missing = missing;
        }
        if let Some(saved) = news.entries {
            self.waiting = false;
            // Files opened before the saved list was read stay in front of it.
            let noted = !self.entries.is_empty();
            for e in saved {
                if !self.entries.iter().any(|n| n.id() == e.id()) {
                    self.entries.push(e);
                }
            }
            let over: Vec<Entry> = self
                .entries
                .drain(MAX_ENTRIES.min(self.entries.len())..)
                .collect();
            for e in over {
                self.io.forget(&e.id());
            }
            if noted {
                self.io.save(&self.entries);
            }
        }
    }

    /// The page was not drawn this frame (a file is open).
    pub fn hidden(&mut self) {
        self.shown = false;
        self.hover = None;
    }
}

/// The names of the files listed beside the app (`Recents::shortlist`); files of one name
/// get their folder after it.
#[cfg(any(target_os = "macos", windows, test))]
pub(crate) fn shortlist_labels(paths: &[PathBuf]) -> Vec<String> {
    let names: Vec<String> = paths.iter().map(|p| file_label(p)).collect();
    let folder = |path: &PathBuf| {
        path.parent()
            .and_then(Path::file_name)
            .map(|f| f.to_string_lossy().into_owned())
    };
    names
        .iter()
        .zip(paths)
        .map(|(name, path)| {
            let alike: Vec<&PathBuf> = names
                .iter()
                .zip(paths)
                .filter(|(n, _)| *n == name)
                .map(|(_, p)| p)
                .collect();
            if alike.len() < 2 {
                return name.clone();
            }
            // The folder's name when that tells them apart, else the folder's whole path.
            let short = folder(path).filter(|f| {
                alike
                    .iter()
                    .filter(|p| folder(p).as_ref() == Some(f))
                    .count()
                    == 1
            });
            let place = short.unwrap_or_else(|| {
                path.parent()
                    .map_or_else(String::new, |p| p.display().to_string())
            });
            format!("{name} — {place}")
        })
        .collect()
}

/// `label` as a native menu title: muda takes `&` as a mnemonic mark.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn menu_title(label: &str) -> String {
    label.replace('&', "&&")
}

/// The name of the file at `path`, as a tab and a card show it.
pub(crate) fn file_label(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The texture of a PNG picture, `None` when it does not decode.
pub(crate) fn png_texture(
    ctx: &egui::Context,
    name: &str,
    bytes: &[u8],
) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let colour = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(name, colour, egui::TextureOptions::LINEAR))
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
    let frame = crate::renderer(Default::default())
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

/// The second line of a list row: the file's folder on the desktop.
#[cfg(not(target_arch = "wasm32"))]
fn place_label(id: &Id) -> String {
    folder_label(&id.path)
}

/// The second line of a list row: in the browser, whether a click opens the file or the
/// picker (no handle: Firefox, Safari, or a file dropped without one).
#[cfg(target_arch = "wasm32")]
fn place_label(id: &Id) -> String {
    if id.handle.is_some() {
        String::new()
    } else {
        "Opens the file picker".into()
    }
}

/// The folder of `path` for showing, with the home folder as `~`.
#[cfg(not(target_arch = "wasm32"))]
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
    id: Id,
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
        // The saved list is not read yet: no welcome that a list would replace.
        if self.recents.entries().is_empty() && self.recents.waiting {
            return None;
        }
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
            let id = e.id();
            let texture = self.recents.io.texture(&ctx, &id, &mut budget);
            cards.push(Card {
                colour,
                missing: self.recents.is_missing(&id),
                when: ago(now, e.opened),
                tint: theme::space_tint(colour, dark),
                id,
                name,
                style,
                texture,
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            let labels: Vec<_> = cards
                .iter()
                .flat_map(|card| {
                    std::iter::once(card.name.clone()).chain(list.then(|| place_label(&card.id)))
                })
                .collect();
            // The names choose the CJK face only when no open file or menu pick has; this
            // page also shows with files open (the + control), so it says which are.
            self.web
                .load_cjk_for_labels(&ctx, &labels, self.tabs.iter().map(|tab| tab.id));
            self.web.symbols.load_labels_if_needed(&ctx, &labels);
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
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn note_recent(&mut self, ctx: &egui::Context, i: usize) {
        let Some(tab) = self.tabs.get(i) else {
            return;
        };
        if !tab.path.is_file() {
            return;
        }
        let path = std::path::absolute(&tab.path).unwrap_or_else(|_| tab.path.clone());
        let job = ThumbJob {
            doc: tab.doc.clone(),
            page: tab.page,
            source: tab.source.clone(),
        };
        self.recents.note(&path, tab.doc_key.as_deref(), now());
        self.recents.paint_thumb(ctx, &Id::file(path), job);
    }

    /// Lists tab `i`'s file if it was picked or dropped here (a link or an agent's file is
    /// not), with its handle where the browser gave one, and queues its picture.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn note_recent(&mut self, ctx: &egui::Context, i: usize) {
        let Some(tab) = self.tabs.get(i) else {
            return;
        };
        let Some(origin) = crate::web::upload_origin(tab.id) else {
            return;
        };
        let id = Id {
            path: tab.path.clone(),
            handle: origin.handle,
        };
        let job = ThumbJob {
            doc: tab.doc.clone(),
            page: tab.page,
            source: tab.source.clone(),
        };
        self.recents
            .add(id.clone(), tab.doc_key.as_deref(), now(), true);
        // Picked in the picker a moved card opened: that card gives way.
        if let Some(old) = origin.replaces {
            let old = Id {
                path: tab.path.clone(),
                handle: Some(old),
            };
            self.recents.replace_moved(&old, &id);
        }
        self.recents.paint_thumb(ctx, &id, job);
    }

    /// Carries out what was picked on the Recents page. A card with a handle opens its
    /// file (the browser may ask to allow it); one without (Firefox, Safari) opens the
    /// picker and says which file to pick.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn recent_picked(&mut self, ctx: &egui::Context, pick: Pick) {
        match pick {
            Pick::Open(id) => match id.handle {
                Some(handle) => self.web.open_handle(ctx, handle, id.path),
                None => {
                    let name = id.path.display().to_string();
                    self.toast(ctx, format!("Pick {name} to open it again."), false);
                    self.open_dialog(ctx);
                }
            },
            Pick::Remove(id) => self.recents.remove(&id),
            Pick::OpenDialog => self.open_dialog(ctx),
        }
    }

    /// Files the person took off the jump list (Windows) leave Recents too, so they stay off.
    #[cfg(any(target_os = "macos", windows, test))]
    pub(crate) fn taken_off_the_list(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            self.recents.remove(&Id::file(path));
        }
    }

    /// Opens `path`, picked in the Dock menu: as its card would, so a file gone since the
    /// menu was made is marked missing and says so.
    #[cfg(any(target_os = "macos", windows, test))]
    pub(crate) fn open_recent_file(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.recent_picked(ctx, Pick::Open(Id::file(path)));
    }

    /// Carries out what was picked on the Recents page (Open file… is the caller's).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn recent_picked(&mut self, ctx: &egui::Context, pick: Pick) {
        match pick {
            // What is on disk now decides, not when the page last looked.
            Pick::Open(id) if id.path.is_file() => {
                self.recents.set_missing(&id, false);
                self.open(id.path);
            }
            Pick::Open(id) => {
                self.recents.set_missing(&id, true);
                let name = id
                    .path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.toast(
                    ctx,
                    format!("{name} was moved or deleted. Remove it from recents, or open it from where it is now."),
                    true,
                );
            }
            Pick::Reveal(id) => {
                if let Err(e) = opener::reveal(&id.path) {
                    self.toast(ctx, format!("Could not show the file: {e}"), true);
                }
            }
            Pick::Remove(id) => self.recents.remove(&id),
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
            &place_label(&card.id),
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
    let response = ui.interact(rect, EguiId::new(("recent", &card.id)), Sense::click());
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
        pick = Some(Pick::Open(card.id.clone()));
    }
    context_menu(response, |ui| {
        ui.set_min_width(190.0);
        if ui
            .add_enabled(!card.missing, egui::Button::new("Open"))
            .clicked()
        {
            pick = Some(Pick::Open(card.id.clone()));
        }
        // The browser has no folder to show.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let reveal = if cfg!(target_os = "macos") {
                "Show in Finder"
            } else {
                "Show in folder"
            };
            if ui
                .add_enabled(!card.missing, egui::Button::new(reveal))
                .clicked()
            {
                pick = Some(Pick::Reveal(card.id.clone()));
            }
        }
        ui.separator();
        if ui.button("Remove from recents").clicked() {
            pick = Some(Pick::Remove(card.id.clone()));
        }
    });
    pick
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps `r`'s page (frame time `time`) until the look it is waiting on has answered.
    /// A probe that waits on `gate` after it looked, counting its calls in `looks`.
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

    fn entry(name: &str, handle: Option<u32>, opened: u64) -> Entry {
        Entry {
            path: PathBuf::from(name),
            handle,
            opened,
            key: None,
        }
    }

    #[test]
    fn files_opened_before_the_saved_list_arrives_stay_in_front_of_it() {
        let mut r = Recents::memory();
        // Opened while the browser was still reading the list.
        r.add(
            Id {
                path: "b.fig".into(),
                handle: Some(2),
            },
            None,
            50,
            true,
        );
        let mut saved = vec![
            entry("a.fig", Some(1), 40),
            entry("b.fig", Some(2), 30),
            // Same name, another file (another handle): its own entry.
            entry("b.fig", Some(3), 20),
        ];
        saved.extend((0..MAX_ENTRIES).map(|i| entry(&format!("{i}.fig"), None, 1)));
        r.take(News {
            missing: None,
            entries: Some(saved),
        });
        let first: Vec<_> = r.entries()[..4]
            .iter()
            .map(|e| (e.path.display().to_string(), e.handle, e.opened))
            .collect();
        assert_eq!(
            first,
            [
                ("b.fig".into(), Some(2), 50),
                ("a.fig".into(), Some(1), 40),
                ("b.fig".into(), Some(3), 20),
                ("0.fig".into(), None, 1),
            ]
        );
        assert_eq!(r.entries().len(), MAX_ENTRIES, "capped after the merge");
    }

    #[test]
    fn a_moved_file_picked_again_takes_the_place_of_its_missing_card() {
        let at = |handle| Id {
            path: "a.fig".into(),
            handle: Some(handle),
        };
        let mut r = Recents::memory();
        r.add(at(1), None, 10, true);
        // Another file of the same name, moved too.
        r.add(at(2), None, 20, true);
        r.set_missing(&at(2), true);
        // A third, opened anew: it takes no card's place.
        r.add(at(3), None, 25, true);
        // The first is clicked, found moved, and picked where it is now: a new handle.
        r.set_missing(&at(1), true);
        r.add(at(4), None, 30, true);
        r.replace_moved(&at(1), &at(4));
        let handles: Vec<_> = r.entries().iter().map(|e| e.handle).collect();
        assert_eq!(handles, [Some(4), Some(3), Some(2)]);
        assert!(!r.missing.contains(&at(1)));
        assert!(r.missing.contains(&at(2)), "the other moved card stays");
        // A card back in place is not replaced.
        r.replace_moved(&at(3), &at(4));
        assert_eq!(r.entries().len(), 3);
        // Nor one with a handle by a file without (the plain picker, when the browser's
        // own one is refused): its handle would be lost.
        let plain = Id {
            path: "a.fig".into(),
            handle: None,
        };
        r.add(plain.clone(), None, 35, true);
        r.replace_moved(&at(2), &plain);
        assert!(r.is_missing(&at(2)), "the card and its handle stay");
        r.remove(&plain);
        // Only the next file picked replaces it.
        r.set_missing(&at(3), true);
        r.add(at(5), None, 40, true);
        assert_eq!(r.entries().len(), 4);
    }

    #[test]
    fn a_relocate_given_up_leaves_the_missing_card_to_other_opens() {
        let at = |handle| Id {
            path: "a.fig".into(),
            handle: Some(handle),
        };
        let mut r = Recents::memory();
        r.add(at(1), None, 10, true);
        r.set_missing(&at(1), true);
        // The picker it opened is cancelled; another file of that name opens some other way
        // (only a file that picker returned replaces the card: `replace_moved`).
        r.add(at(2), None, 20, true);
        let handles: Vec<_> = r.entries().iter().map(|e| e.handle).collect();
        assert_eq!(handles, [Some(2), Some(1)]);
    }

    #[test]
    fn the_shortlist_is_the_five_newest_files_not_missing() {
        let mut r = Recents::memory();
        for i in 0..7 {
            r.add(Id::file(format!("/f{i}.fig")), None, i, true);
        }
        r.set_missing(&Id::file("/f5.fig"), true);
        let expected: Vec<PathBuf> = ["/f6.fig", "/f4.fig", "/f3.fig", "/f2.fig", "/f1.fig"]
            .map(PathBuf::from)
            .into();
        assert_eq!(r.shortlist(), expected);
    }

    #[test]
    fn files_of_one_name_beside_the_app_are_told_apart_by_their_folder() {
        let paths = ["/a/x.fig", "/b/x.fig", "/c/y&z.fig"].map(PathBuf::from);
        assert_eq!(
            shortlist_labels(&paths),
            ["x.fig — a", "x.fig — b", "y&z.fig"]
        );
        // Folders of one name, or none (a root), give their whole path.
        let paths = ["/work/final/x.fig", "/archive/final/x.fig", "/x.fig"].map(PathBuf::from);
        assert_eq!(
            shortlist_labels(&paths),
            ["x.fig — /work/final", "x.fig — /archive/final", "x.fig — /"]
        );
        // A menu title shows a single `&` only when doubled (muda takes `&x` as a mnemonic).
        assert_eq!(menu_title("y&z.fig"), "y&&z.fig");
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

    #[test]
    fn a_picture_fits_the_card_size() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/structure.fig"
        );
        let doc = Arc::new(figstudio_fig::Document::open(path).unwrap());
        let png = thumb_png(&ThumbJob {
            page: doc.pages[0],
            doc,
            source: None,
        })
        .unwrap();
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
