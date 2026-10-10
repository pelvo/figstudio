//! One open document: view state, selection and the last rendered frame.

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

/// A rendered frame and the view it was rendered for, so it can be re-projected while a
/// newer frame is on its way.
pub struct Shown {
    pub texture: egui::TextureHandle,
    pub view: View,
    pub size_points: egui::Vec2,
    pub page: Guid,
}

pub struct Tab {
    pub id: u64,
    pub path: PathBuf,
    pub doc: Arc<Document>,
    /// Where the document's unread images are (big files open lazily).
    pub source: Option<crate::source::Source>,
    pub page: Guid,
    pub view: View,
    pub fit_pending: bool,
    pub selection: Option<Guid>,
    pub hover: Option<Guid>,
    pub expanded: HashSet<Guid>,
    pub reveal: Option<Guid>,
    pub shown: Option<Shown>,
    pub generation: u64,
    pub requested_key: Option<String>,
    pub shown_generation: u64,
    pub stats: Stats,
    pub render_time: Duration,
    pub load_time: Duration,
    /// Canvas rect from the last frame, for keyboard zoom around the centre.
    pub canvas_rect: Option<egui::Rect>,
}

impl Tab {
    pub fn new(
        id: u64,
        path: PathBuf,
        doc: Arc<Document>,
        source: Option<crate::source::Source>,
        load_time: Duration,
    ) -> Self {
        let page = doc
            .pages
            .first()
            .copied()
            .or(doc.root)
            .unwrap_or(figstudio_fig::Guid(0, 0));
        Tab {
            id,
            path,
            doc,
            source,
            page,
            view: View {
                zoom: 1.0,
                pan: egui::Vec2::ZERO,
            },
            fit_pending: true,
            selection: None,
            hover: None,
            expanded: HashSet::new(),
            reveal: None,
            shown: None,
            generation: 0,
            requested_key: None,
            shown_generation: 0,
            stats: Stats::default(),
            render_time: Duration::ZERO,
            load_time,
            canvas_rect: None,
        }
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
