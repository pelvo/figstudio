//! One open document: view state, selection and its pending tile requests.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use figstudio_fig::{Document, Guid};
use figstudio_render::Stats;

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
        Tab {
            id,
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
            marquee: Default::default(),
            tint: crate::theme::HOME_TINT,
            style: Default::default(),
            folder,
            find_hits: Vec::new(),
            find_current: None,
            find_fade: 0.0,
            find_search: 0,
        }
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

    pub fn set_page(&mut self, page: Guid) {
        if self.page != page {
            self.page = page;
            self.selection = None;
            self.hover = None;
            self.fit_pending = true;
        }
    }

    /// Reveal `node` (MCP `reveal_node`, Open Figma link): switch to it when it is a page, else
    /// select it and fit it into view on the next draw.
    pub fn show_node(&mut self, node: Guid) {
        if self.doc.pages.contains(&node) {
            self.set_page(node);
        } else {
            self.select(Some(node));
            self.fit_node = Some(node);
        }
    }

    /// Selects a node, switching page if needed and expanding its ancestors in the Layers tree.
    pub fn select(&mut self, id: Option<Guid>) {
        self.selection = id;
        let Some(id) = id else { return };
        if let Some(page) = self.doc.page_of(id)
            && page != self.page
        {
            self.page = page;
            self.fit_pending = true;
        }
        let mut cur = self.doc.node(id).and_then(|n| n.parent);
        while let Some(p) = cur {
            self.expanded.insert(p);
            cur = self.doc.node(p).and_then(|n| n.parent);
        }
        self.reveal = Some(id);
    }
}
