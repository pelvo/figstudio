//! One open document: view state, selection and its pending tile requests.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use figstudio_fig::{Document, Guid};
use figstudio_render::Stats;
use figstudio_render::color::{AlphaColor, Srgb};
use figstudio_render::prototype::{self, Player, Screen, Step};

/// Page-to-screen mapping: `screen = canvas.min + pan + page * zoom` (in points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f32,
    pub pan: egui::Vec2,
}

impl View {
    pub fn to_screen(self, origin: egui::Pos2, p: egui::Pos2) -> egui::Pos2 {
        origin + self.pan + p.to_vec2() * self.zoom
    }

    pub fn to_page(self, origin: egui::Pos2, s: egui::Pos2) -> egui::Pos2 {
        ((s - origin - self.pan) / self.zoom).to_pos2()
    }

    /// Zooms by `factor` keeping the page point under `anchor` (screen) fixed.
    pub fn zoom_at(&mut self, origin: egui::Pos2, anchor: egui::Pos2, factor: f32) {
        let before = self.to_page(origin, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_screen(origin, before);
        self.pan += anchor - after;
    }
}

pub const MIN_ZOOM: f32 = 0.01;
pub const MAX_ZOOM: f32 = 256.0;

pub struct Tab {
    pub id: u64,
    pub path: PathBuf,
    /// `path` resolved (symlinks, `..`) by the loader, else `path`: how a link's local copy is
    /// recognised as this tab without disk access on the UI thread.
    pub real_path: PathBuf,
    pub doc: Arc<Document>,
    pub pick_cache: crate::pick_cache::PickCache,
    pub a11y_reach: crate::canvas_a11y::ReachCache,
    /// Motion and shader details of the selected layer (`inspect.rs`).
    pub inspect: crate::inspect::Cache,
    /// Code Connect mappings and page suggestions (`code_connect.rs`).
    pub code_connect: crate::code_connect::Cache,
    /// Shader code: sources, missing shaders and the notice (`shader_code.rs`).
    pub shaders: crate::shader_code::TabShaders,
    /// First 16 hex of SHA-256 over the file size and first 4 MiB (`docs[].key`).
    #[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
    pub doc_key: Option<String>,
    /// Where the document's unread images are (big files open lazily).
    pub source: Option<crate::source::Source>,
    pub page: Guid,
    pub view: View,
    pub fit_pending: bool,
    /// Fit this node into view on the next canvas frame (`reveal_node` from an agent).
    pub fit_node: Option<Guid>,
    pub selection: Option<Guid>,
    /// Inner canvas target; panels and MCP retain the outermost instance selection.
    pub inner: Option<figstudio_fig::resolve::InstancePath>,
    pub text_select: Option<crate::text_select::Selection>,
    pub hover: Option<Guid>,
    pub expanded: HashSet<Guid>,
    pub reveal: Option<Guid>,
    /// Tiles requested from the render worker and not back yet (`tiles::next_batch`).
    pub tile_requests: Vec<crate::tiles::Key>,
    /// Tiles received since the last batch, told to the worker with the next one.
    pub tiles_received: Vec<(crate::tiles::Key, u64)>,
    pub stats: Stats,
    pub render_time: Duration,
    pub load_time: Duration,
    /// Canvas rect from the last frame, for keyboard zoom around the centre.
    pub canvas_rect: Option<egui::Rect>,
    /// The view drawn last frame: `view`, or the ease toward it after a jump.
    pub shown: View,
    /// A Copy menu item picked this frame; `main.rs` puts it on the clipboard.
    pub pending_copy: Option<crate::copy::Item>,
    /// A timeline whose Play button was pressed this frame; `main.rs` opens its preview.
    pub pending_motion: Option<Guid>,
    /// Properties → Motion → Export asked for this timeline root in this format.
    pub pending_motion_export: Option<(Guid, figstudio_video::Format)>,
    /// The open motion preview (`motion_preview.rs`).
    pub motion_preview: Option<crate::motion_preview::Preview>,
    /// The drag-drawn region (`marquee.rs`).
    pub marquee: crate::marquee::Marquee,
    /// The preset tint picked for this file (`theme::pick_tint`), unless `style` sets one.
    pub tint: usize,
    /// The space's colour, symbol and name chosen by the user (`spaces.rs`).
    pub style: crate::spaces::Style,
    /// A hash of the file's folder (`spaces::folder_of`), to tell same-named files apart.
    pub folder: Option<String>,
    /// Text layers matching the find bar, highlighted on the canvas (`find.rs`).
    pub find_hits: Vec<crate::find::Hit>,
    pub find_current: Option<usize>,
    /// The find bar's presence (the highlights fade with it) and its search's number (new
    /// matches fade in).
    pub find_fade: f32,
    pub find_search: u64,
    /// Prototype Preview, while it runs: the canvas shows its screen alone.
    pub preview: Option<Preview>,
    /// Per-tab Undo/Redo stack for non-destructive editing.
    pub history: figstudio_fig::edit::History,
    /// [`figstudio_fig::edit::History::state_id`] at open time or the last Save Copy.
    pub saved_revision: u64,
    /// Whether `history.state_id() != saved_revision`.
    pub dirty_unsaved: bool,
    /// Active interactive Move / Resize / Create drag on the canvas.
    pub edit_drag: Option<crate::editor::CanvasDrag>,
    /// Set by creation tools once a new layer is placed so the toolbar returns to Select.
    pub reset_tool_to_select: bool,
}

/// One picture the canvas draws (see [`Tab::draw_layers`]).
#[derive(Clone, Debug)]
pub struct DrawLayer {
    pub id: LayerId,
    /// The document number its tiles are stored under.
    pub doc: u64,
    pub state: Arc<prototype::State>,
    /// Page units it is drawn moved by.
    pub shift: egui::Vec2,
    /// Page-space rectangle it is drawn inside (before the shift), if any.
    pub clip: Option<egui::Rect>,
}

/// Which picture a [`DrawLayer`] is, kept across states (for its fallback).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LayerId {
    Screen,
    Content(Vec<Guid>),
    Pinned(Vec<Guid>, Guid),
}

/// The page-space bounding box of an area.
fn area_rect(area: &prototype::Area) -> egui::Rect {
    let c = area.corners();
    let (lo, hi) = c.iter().fold(
        (
            egui::pos2(f32::MAX, f32::MAX),
            egui::pos2(f32::MIN, f32::MIN),
        ),
        |(lo, hi), p| {
            let p = egui::pos2(p.x as f32, p.y as f32);
            (lo.min(p), hi.max(p))
        },
    );
    egui::Rect::from_min_max(lo, hi)
}

/// Seconds without scrolling before Preview reads the moved links again.
const SCROLL_SETTLE: f64 = 0.15;

/// The document number for tiles of `doc` drawn with prototype `state`: the document's own
/// for no state, else one per state, so going back to a state reuses its tiles.
fn state_doc(doc: u64, state: &prototype::State) -> u64 {
    use std::hash::{Hash, Hasher};
    if state.swaps.is_empty() && state.scrolls.is_empty() && state.pass == prototype::Pass::Full {
        return doc;
    }
    let mut swaps: Vec<_> = state.swaps.iter().collect();
    swaps.sort();
    let mut scrolls: Vec<_> = state.scrolls.iter().collect();
    scrolls.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (doc, swaps, scrolls, &state.pass).hash(&mut hasher);
    hasher.finish() | 1 << 63
}

/// Prototype Preview: the screens visited and the links of the one shown.
pub struct Preview {
    pub player: Player,
    pub screen: Screen,
    /// The page and view before Preview started; both come back when it stops.
    pub canvas: (Guid, View),
    /// When a click hit no link (`egui` input time): the links flash, as in Figma.
    pub missed: Option<f64>,
    /// Present: the screen fills the window, with no sidebar, panels or pills.
    pub present: bool,
    /// When the pointer last moved over Preview (`egui` input time): Present's controls fade
    /// out after it rests.
    pub active_at: f64,
    /// The canvas size the screen was last fitted to.
    pub fitted_size: Option<egui::Vec2>,
    /// The player's state as the tiles draw it, and their document number for it.
    pub drawn: (u64, Arc<prototype::State>),
    /// Per layer drawn, the tile document number of its last picture whose tiles were all
    /// on screen: they stay under a newer picture's until those arrive (a hover changes the
    /// button in place, without a blank).
    pub complete: std::collections::HashMap<LayerId, u64>,
    /// When a scroll moved the links without reading them again (`Tab::preview_tick` reads
    /// them once the scroll has paused for [`SCROLL_SETTLE`]).
    pub stale_since: Option<f64>,
}

impl Preview {
    /// The shown screen's place among its page's frames (`prototype::frames`), from 1, and
    /// how many there are; `None` for a screen that is not one of them.
    pub fn position(&self, doc: &figstudio_fig::Document) -> Option<(usize, usize)> {
        let screen = self.player.screen();
        let frames = prototype::frames(doc, doc.page_of(screen)?);
        let at = frames.iter().position(|f| *f == screen)?;
        Some((at + 1, frames.len()))
    }
}

impl Tab {
    pub fn new(
        id: u64,
        path: PathBuf,
        doc: Arc<Document>,
        source: Option<crate::source::Source>,
        load_time: Duration,
    ) -> Self {
        #[cfg(test)]
        let doc_key = crate::source::doc_key_from_path(&path);
        #[cfg(not(test))]
        let doc_key = None;
        let page = doc
            .pages
            .first()
            .copied()
            .or(doc.root)
            .unwrap_or(figstudio_fig::Guid(0, 0));
        let folder = crate::spaces::folder_of(&path);
        let saved_revision = 0;
        Tab {
            id,
            pick_cache: crate::pick_cache::PickCache::new(&doc),
            a11y_reach: Default::default(),
            inspect: Default::default(),
            code_connect: Default::default(),
            shaders: Default::default(),
            real_path: path.clone(),
            path,
            doc,
            doc_key,
            source,
            page,
            view: View {
                zoom: 1.0,
                pan: egui::Vec2::ZERO,
            },
            fit_pending: true,
            fit_node: None,
            selection: None,
            inner: None,
            text_select: None,
            hover: None,
            expanded: HashSet::new(),
            reveal: None,
            tile_requests: Vec::new(),
            tiles_received: Vec::new(),
            stats: Stats::default(),
            render_time: Duration::ZERO,
            load_time,
            canvas_rect: None,
            shown: View {
                zoom: 1.0,
                pan: egui::Vec2::ZERO,
            },
            pending_copy: None,
            pending_motion: None,
            pending_motion_export: None,
            motion_preview: None,
            marquee: Default::default(),
            tint: crate::theme::HOME_TINT,
            style: Default::default(),
            folder,
            find_hits: Vec::new(),
            find_current: None,
            find_fade: 0.0,
            find_search: 0,
            preview: None,
            history: figstudio_fig::edit::History::new(200),
            saved_revision,
            dirty_unsaved: false,
            edit_drag: None,
            reset_tool_to_select: false,
        }
    }

    /// Recomputes `dirty_unsaved` against `saved_revision` (so Edit -> Save -> Undo is dirty,
    /// and Redo back to the saved state clears `dirty_unsaved`).
    pub fn sync_unsaved(&mut self) {
        self.dirty_unsaved = self.history.state_id() != self.saved_revision;
    }

    /// Marks the current `history.state_id()` as saved on disk.
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.history.state_id();
        self.dirty_unsaved = false;
    }

    /// The space's colour: the user's, else the tint picked for the file.
    pub fn colour(&self) -> crate::theme::Colour {
        self.style
            .colour
            .unwrap_or(crate::theme::Colour::Preset(self.tint))
    }

    /// The space's name: the user's, else the file name.
    pub fn space_name(&self) -> String {
        self.style
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| self.title())
    }

    pub fn with_real_path(mut self, real_path: Option<PathBuf>) -> Self {
        if let Some(real_path) = real_path {
            self.real_path = real_path;
        }
        self
    }

    pub fn with_doc_key(mut self, doc_key: Option<String>) -> Self {
        if doc_key.is_some() {
            self.doc_key = doc_key;
        }
        self
    }

    /// Zooms around the canvas centre (keyboard / menu zoom).
    pub fn zoom_centre(&mut self, zoom: f32) {
        let rect = self.canvas_rect.unwrap_or(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        ));
        let factor = zoom.clamp(MIN_ZOOM, MAX_ZOOM) / self.view.zoom;
        self.view.zoom_at(rect.min, rect.center(), factor);
    }

    pub fn title(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .or_else(|| self.doc.file_name.clone())
            .unwrap_or_else(|| "Untitled".into())
    }

    /// Zero-based page index, with internal pages after normal pages.
    #[cfg(any(target_arch = "wasm32", test))]
    pub fn page_index(&self) -> Option<usize> {
        self.doc
            .pages
            .iter()
            .chain(&self.doc.internal_pages)
            .position(|&page| page == self.page)
    }

    /// Page addressed by the browser's zero-based page probe.
    #[cfg(any(target_arch = "wasm32", test))]
    pub fn page_at(&self, index: usize) -> Option<Guid> {
        self.doc
            .pages
            .iter()
            .chain(&self.doc.internal_pages)
            .nth(index)
            .copied()
    }

    pub fn set_page(&mut self, page: Guid) {
        if self.page != page {
            self.page = page;
            self.selection = None;
            self.inner = None;
            self.text_select = None;
            self.hover = None;
            self.fit_pending = true;
        }
    }

    /// What the canvas draws: the Preview screen, else the page.
    pub fn root(&self) -> Guid {
        self.preview
            .as_ref()
            .map_or(self.page, |p| p.player.screen())
    }

    /// The colour behind [`Tab::root`].
    pub fn background(&self) -> AlphaColor<Srgb> {
        if self.preview.is_some() {
            prototype::background(&self.doc, self.page)
        } else {
            figstudio_render::page_background(&self.doc, self.page)
        }
    }

    /// Starts Preview at the selection's screen, else the page's first flow (see
    /// [`prototype::start`]); `false` when there is no frame to show.
    pub fn start_preview(&mut self, present: bool) -> bool {
        let Some(start) = prototype::start(&self.doc, self.page, self.selection) else {
            return false;
        };
        let canvas = (self.page, self.view);
        self.show_screen(start);
        self.select(None);
        self.preview = Some(Preview {
            player: Player::new(start),
            screen: self.read_screen(start),
            canvas,
            missed: None,
            present,
            fitted_size: None,
            active_at: 0.0,
            drawn: (self.doc.instance, Arc::default()),
            complete: std::collections::HashMap::new(),
            stale_since: None,
        });
        true
    }

    pub fn stop_preview(&mut self) {
        if let Some(preview) = self.preview.take() {
            let (page, view) = preview.canvas;
            self.set_page(page);
            self.fit_node = None;
            self.fit_pending = false;
            self.view = view;
        }
    }

    /// Follows a link (or Back) in Preview, or restarts with `None`; returns whether the
    /// screen changed.
    pub fn preview_go(&mut self, step: Option<Step>) -> bool {
        let Some(preview) = self.preview.as_mut() else {
            return false;
        };
        let before = preview.player.screen();
        let changed = match step {
            Some(step) => preview.player.go(step),
            None => preview.player.restart(),
        };
        self.after_player(before, changed);
        changed
    }

    /// Passes a pointer event (page space) to the player; returns whether anything changed.
    pub fn preview_pointer(&mut self, event: prototype::Pointer, now: f64) -> bool {
        let Some(preview) = self.preview.as_mut() else {
            return false;
        };
        // A press needs the links where they are now: read them if a scroll left them stale.
        if matches!(
            event,
            prototype::Pointer::Down(_) | prototype::Pointer::Up(_)
        ) && preview.stale_since.is_some()
        {
            self.reread();
        }
        let Some(preview) = self.preview.as_mut() else {
            return false;
        };
        let before = preview.player.screen();
        let changed = preview.player.pointer(&preview.screen, event, now);
        if changed && matches!(event, prototype::Pointer::Scroll(..)) {
            // Scrolling only moves what is inside the frame: draw it now, read the links once
            // the scroll pauses (reading a heavy screen on every wheel event would stutter).
            let state = preview.player.state().clone();
            preview.drawn = (state_doc(self.doc.instance, &state), Arc::new(state));
            preview.stale_since = Some(now);
            return true;
        }
        self.after_player(before, changed);
        changed
    }

    /// Passes a wheel scroll to the player; returns whether a frame of the screen took it
    /// (else the view pans), moved or not yet (fractions of a unit add up).
    pub fn preview_scroll(&mut self, event: prototype::Pointer, now: f64) -> bool {
        let prototype::Pointer::Scroll(p, delta) = event else {
            return false;
        };
        let takes = self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.player.takes_scroll(&preview.screen, p, delta));
        if takes {
            self.preview_pointer(event, now);
        }
        takes
    }

    /// Reads the shown screen's links again (after scrolling moved them).
    fn reread(&mut self) {
        let Some(screen) = self.preview.as_ref().map(|p| p.player.screen()) else {
            return;
        };
        let links = self.read_screen(screen);
        if let Some(preview) = self.preview.as_mut() {
            preview.screen = links;
            preview.stale_since = None;
        }
    }

    /// Fires a due "after delay" link; returns when the next one is due, if any.
    pub fn preview_tick(&mut self, now: f64) -> Option<f64> {
        let stale = self.preview.as_ref()?.stale_since;
        if stale.is_some_and(|at| now - at >= SCROLL_SETTLE) {
            self.reread();
        }
        let preview = self.preview.as_mut()?;
        let before = preview.player.screen();
        let changed = preview.player.tick(&preview.screen, now);
        self.after_player(before, changed);
        let preview = self.preview.as_ref()?;
        let reread = preview.stale_since.map(|at| at + SCROLL_SETTLE);
        match (preview.player.next_due(&preview.screen), reread) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// After the player changed the screen or the state: show the screen, draw the state and
    /// read the links its variants have.
    fn after_player(&mut self, before: Guid, changed: bool) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        if !changed {
            return;
        }
        let screen = preview.player.screen();
        let state = preview.player.state().clone();
        if *preview.drawn.1 != state {
            preview.drawn = (state_doc(self.doc.instance, &state), Arc::new(state));
        }
        if screen != before {
            self.show_screen(screen);
        }
        let links = self.read_screen(screen);
        if let Some(preview) = self.preview.as_mut() {
            preview.screen = links;
            preview.missed = None;
            preview.stale_since = None;
        }
    }

    /// The document number the tiles of the canvas's main picture are stored under: the
    /// document's, or one for the prototype state Preview draws.
    pub fn tile_doc(&self) -> u64 {
        self.draw_layers()[0].doc
    }

    /// What the canvas draws, in order: the main picture, then in Preview each scrolling
    /// frame's content (shifted by its scroll offset, clipped to the frame) and its pinned
    /// children. Scrolling changes only the shifts, so it re-renders nothing.
    pub fn draw_layers(&self) -> Vec<DrawLayer> {
        let whole = |doc, state| DrawLayer {
            id: LayerId::Screen,
            doc,
            state,
            shift: egui::Vec2::ZERO,
            clip: None,
        };
        let Some(preview) = &self.preview else {
            return vec![whole(self.doc.instance, Arc::default())];
        };
        let layered: Vec<&prototype::Scroller> = preview
            .screen
            .scrollers
            .iter()
            .filter(|s| !s.nested)
            .collect();
        if layered.is_empty() {
            return vec![whole(preview.drawn.0, preview.drawn.1.clone())];
        }
        let keys: Vec<Vec<Guid>> = layered.iter().map(|s| s.layer.key()).collect();
        let mut unscrolled = (*preview.drawn.1).clone();
        unscrolled.scrolls.retain(|k, _| !keys.contains(k));
        let with = |pass: prototype::Pass| {
            let state = prototype::State {
                pass,
                ..unscrolled.clone()
            };
            (state_doc(self.doc.instance, &state), Arc::new(state))
        };
        let (doc, state) = with(prototype::Pass::Base(keys.clone()));
        let mut layers = vec![whole(doc, state)];
        for scroller in layered {
            let key = scroller.layer.key();
            let (x, y) = preview.drawn.1.scrolls.get(&key).copied().unwrap_or((0, 0));
            let offset = (f64::from(x), f64::from(y));
            let clip = scroller
                .clips
                .iter()
                .map(area_rect)
                .fold(area_rect(&scroller.area), |a, b| a.intersect(b));
            let (doc, state) = with(prototype::Pass::Content(key.clone()));
            layers.push(DrawLayer {
                id: LayerId::Content(key.clone()),
                doc,
                state,
                shift: egui::vec2(-offset.0 as f32, -offset.1 as f32),
                clip: Some(clip),
            });
            for (child, pin) in &scroller.pinned {
                let (dx, dy) = pin.shift(offset);
                let (doc, state) = with(prototype::Pass::Pinned(key.clone(), *child));
                layers.push(DrawLayer {
                    id: LayerId::Pinned(key.clone(), *child),
                    doc,
                    state,
                    shift: egui::vec2(dx as f32, dy as f32),
                    clip: Some(clip),
                });
            }
        }
        layers
    }

    /// The prototype state the tiles draw (empty outside Preview).
    pub fn prototype_state(&self) -> Arc<prototype::State> {
        self.preview
            .as_ref()
            .map_or_else(Arc::default, |p| p.drawn.1.clone())
    }

    /// Steps to the next (`forward`) or previous frame of the screen's page, as Figma's arrow
    /// keys do; a screen that is not one of them steps to the page's first frame. Back
    /// returns from a step like from a link. Returns whether the screen changed.
    pub fn preview_step(&mut self, forward: bool) -> bool {
        let Some(preview) = self.preview.as_ref() else {
            return false;
        };
        let screen = preview.player.screen();
        let Some(page) = self.doc.page_of(screen) else {
            return false;
        };
        let frames = prototype::frames(&self.doc, page);
        let next = match frames.iter().position(|f| *f == screen) {
            Some(at) if forward => frames.get(at + 1),
            Some(at) => at.checked_sub(1).and_then(|i| frames.get(i)),
            None => frames.first(),
        };
        match next.copied() {
            Some(to) => self.preview_go(Some(Step::Navigate(to))),
            None => false,
        }
    }

    /// Goes to the screen's page and fits it into view on the next draw.
    fn show_screen(&mut self, screen: Guid) {
        if let Some(page) = self.doc.page_of(screen) {
            self.set_page(page);
        }
        self.fit_node = Some(screen);
    }

    // shortcut: read on the UI thread with the pick renderer (about 0.1 s for a screen with
    // hundreds of instance links in a 100k-node file); move to the worker if that shows.
    fn read_screen(&self, frame: Guid) -> Screen {
        let state = self.prototype_state();
        self.pick_cache.with(&self.doc, |renderer, _| {
            let canvas = std::mem::replace(&mut renderer.prototype, state);
            let screen = Screen::read(renderer, &self.doc, frame);
            renderer.prototype = canvas;
            screen
        })
    }

    /// Reveal `node` (MCP `reveal_node`, Open Figma link): switch to it when it is a page, else
    /// select it and fit it into view on the next draw.
    pub fn show_node(&mut self, node: Guid) {
        if self.doc.pages.contains(&node) || self.doc.internal_pages.contains(&node) {
            self.set_page(node);
        } else {
            self.select(Some(node));
            self.fit_node = Some(node);
        }
    }
}
