//! ⌘F: find text on the current page, the way a browser finds in a page: text layers, text
//! inside instances, and the frame and section names drawn above frames. Matches are
//! highlighted on the canvas; Enter / ⌘G go to the next one, Shift for the
//! previous; Escape closes the bar.

use egui::{
    Align, Color32, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    WidgetInfo, WidgetType, pos2, vec2,
};
use figstudio_fig::{Document, Guid};

use crate::chrome::card_shadow;
use crate::icons::{self, Icon};
use crate::tab::Tab;
use crate::theme;

/// Highlight of every match on the canvas, and of the current one.
pub const HIT: Color32 = Color32::from_rgb(255, 214, 10);
pub const CURRENT: Color32 = Color32::from_rgb(255, 149, 0);

/// Most matches kept for one search.
const MAX_HITS: usize = 5000;

#[derive(Debug, Default)]
pub struct FindBar {
    open: bool,
    pub query: String,
    focus: bool,
    /// The matches for `hits_for` (query, document instance, page), in reading order.
    hits: Vec<Hit>,
    hits_for: Option<(String, u64, Guid)>,
    current: usize,
    /// Show the current match on the next canvas frame.
    reveal: bool,
    /// How present the bar is: it slides in, and fades out with its highlights after closing.
    fade: f32,
    /// Counts searches, so new matches fade in.
    search: u64,
    /// The tab (`Tab::id`) and page the last matches were given to: after closing, only they
    /// keep theirs while fading (a match box belongs to its page).
    shown_on: Option<(u64, Guid)>,
}

impl FindBar {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the bar (or focuses it again), keeping the last search like a browser.
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
    }

    /// Closes the bar. Its highlights fade out with it (`update`).
    pub fn hide(&mut self) {
        self.open = false;
        self.hits_for = None;
    }

    /// Moves to the next match (`forward`) or the previous one, wrapping around.
    pub fn step(&mut self, forward: bool) {
        let n = self.hits.len();
        if n == 0 {
            return;
        }
        self.current = if forward {
            (self.current + 1) % n
        } else {
            (self.current + n - 1) % n
        };
        self.reveal = true;
    }

    /// No file is open: the bar (drawn only over a file) eases out, so it is not left
    /// half-faded for the next file, and its matches go.
    pub fn update_without_tab(&mut self, ctx: &egui::Context) {
        self.fade = crate::anim::presence(ctx, egui::Id::new("find-bar"), false);
        self.hits.clear();
        self.hits_for = None;
        self.shown_on = None;
    }

    /// Searches `tab`'s page again when the query, file or page changed, then gives the tab
    /// the matches to highlight. Call before the canvas draws.
    pub fn update(&mut self, tab: &mut Tab, ctx: &egui::Context) {
        self.fade = crate::anim::presence(ctx, egui::Id::new("find-bar"), self.open);
        tab.find_fade = self.fade;
        if !self.open && self.fade > 0.001 && self.shown_on == Some((tab.id, tab.page)) {
            // Closing: the last highlights stay while they fade, on their own page.
            return;
        }
        if !self.open || self.query.trim().is_empty() {
            self.hits.clear();
            self.hits_for = None;
            self.shown_on = None;
            tab.find_hits.clear();
            tab.find_current = None;
            return;
        }
        let key = (self.query.clone(), tab.doc.instance, tab.page);
        if self.hits_for.as_ref() != Some(&key) {
            self.hits = find_text(&tab.doc, tab.page, &self.query);
            self.hits_for = Some(key);
            self.search += 1;
            self.current = 0;
            self.reveal = true;
        }
        tab.find_hits.clone_from(&self.hits);
        self.shown_on = Some((tab.id, tab.page));
        tab.find_search = self.search;
        tab.find_current = (!self.hits.is_empty()).then_some(self.current);
        if std::mem::take(&mut self.reveal)
            && let Some(hit) = self.hits.get(self.current)
        {
            reveal(tab, hit);
        }
    }
}

/// One match: a text layer of the page, or text drawn inside an instance (its component's
/// layer with the instance's overrides), with its page-space box.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Node(Guid),
    Inside(Rect),
    /// A frame or section whose name (drawn above it) matches.
    Name(Guid),
}

impl Hit {
    /// The match's box on the page.
    pub fn bounds(&self, doc: &Document) -> Option<Rect> {
        match self {
            Hit::Node(id) | Hit::Name(id) => figstudio_render::node_bounds(doc, *id).map(|b| {
                Rect::from_min_max(
                    Pos2::new(b.x0 as f32, b.y0 as f32),
                    Pos2::new(b.x1 as f32, b.y1 as f32),
                )
            }),
            Hit::Inside(rect) => Some(*rect),
        }
    }
}

/// Most nodes looked at inside instances for one search (huge design-system pages).
const MAX_INSIDE: usize = 200_000;

/// Text on `page` containing `query` (ignoring case): text layers, text inside instances as
/// they show it, and the names drawn above frames and sections. Top to bottom, then left to
/// right.
pub fn find_text(doc: &Document, page: Guid, query: &str) -> Vec<Hit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let matches = |text: &str| text.to_lowercase().contains(&query);
    let mut hits: Vec<Hit> = crate::labels::named(doc, page)
        .into_iter()
        .filter(|id| doc.node(*id).is_some_and(|n| matches(&n.name)))
        .map(Hit::Name)
        .collect();
    let mut instances = Vec::new();
    let mut stack: Vec<Guid> = doc.children(page).to_vec();
    while let Some(id) = stack.pop() {
        stack.extend(doc.children(id).iter().copied());
        let Some(node) = doc.node(id) else { continue };
        if node.kind == "INSTANCE" {
            instances.push(id);
        } else if crate::copy::text(node).is_some_and(matches) {
            hits.push(Hit::Node(id));
        }
    }
    let definitions = std::sync::Arc::new(figstudio_fig::resolve::definitions(doc));
    let mut renderer = figstudio_render::Renderer::new();
    let mut budget = MAX_INSIDE;
    for instance in instances {
        if budget == 0 || hits.len() >= MAX_HITS {
            break;
        }
        let Ok(walk) =
            figstudio_fig::resolve::walk_instance(doc, &definitions, instance, None, budget)
        else {
            continue;
        };
        budget = budget.saturating_sub(walk.nodes.len());
        for inside in walk.nodes {
            let is_text = doc.node(inside.node).is_some_and(|n| n.kind == "TEXT");
            let text = inside
                .data
                .path(&["textData", "characters"])
                .and_then(figstudio_fig::Val::as_str);
            if !is_text || !text.is_some_and(matches) {
                continue;
            }
            if let Some(b) = renderer.path_bounds(doc, &inside.path) {
                hits.push(Hit::Inside(Rect::from_min_max(
                    Pos2::new(b.x0 as f32, b.y0 as f32),
                    Pos2::new(b.x1 as f32, b.y1 as f32),
                )));
            }
        }
    }
    hits.truncate(MAX_HITS);
    // Reading order: rows of the page from the top, left to right within a row.
    let place = |hit: &Hit| {
        hit.bounds(doc)
            .map_or((f32::MAX, f32::MAX), |b| (b.top(), b.left()))
    };
    hits.sort_by(|a, b| {
        let (pa, pb) = (place(a), place(b));
        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
    });
    hits
}

/// Brings a match into view the way a browser scrolls to it: nothing when it is already
/// readable on screen, else centred, zoomed in when it would be too small to read and out
/// when it would not fit.
fn reveal(tab: &mut Tab, hit: &Hit) {
    let Some(page) = hit.bounds(&tab.doc) else {
        return;
    };
    let Some(canvas) = tab.canvas_rect else {
        return;
    };
    let screen = Rect::from_min_max(
        tab.view.to_screen(canvas.min, page.min),
        tab.view.to_screen(canvas.min, page.max),
    );
    let readable = screen.height() >= 9.0;
    if readable && canvas.shrink(48.0).contains_rect(screen) {
        return;
    }
    if page.width() * tab.view.zoom > canvas.width() - 96.0
        || page.height() * tab.view.zoom > canvas.height() - 96.0
    {
        crate::canvas::fit_rect(tab, Rect::from_min_size(Pos2::ZERO, canvas.size()), page);
        return;
    }
    if !readable {
        tab.view.zoom =
            (24.0 / page.height().max(0.01)).clamp(crate::tab::MIN_ZOOM, crate::tab::MAX_ZOOM);
    }
    tab.view.pan = canvas.size() / 2.0 - page.center().to_vec2() * tab.view.zoom;
}

impl crate::App {
    /// The find bar at the top right of the canvas card. Returns its bottom, for whatever
    /// sits below it.
    pub(crate) fn find_bar(&mut self, ui: &mut Ui, card: Rect) -> f32 {
        let (open, t) = (self.find.open, self.find.fade);
        if !open && t <= 0.001 {
            return card.top();
        }
        // Slides down into place and fades in; after closing fades back up, taking no input.
        let mut ui = ui.new_child(UiBuilder::new().max_rect(ui.max_rect()));
        let ui = &mut ui;
        ui.multiply_opacity(t.clamp(0.0, 1.0));
        if !open {
            crate::anim::inert(ui);
        }
        let p = theme::of(ui);
        let bar = &mut self.find;
        let rect = Rect::from_min_size(
            pos2(card.right() - 12.0 - 340.0, card.top() + 12.0),
            vec2(340.0, 36.0),
        )
        .intersect(card.shrink(8.0))
        .translate(vec2(0.0, -(1.0 - t) * 10.0));
        if open {
            ui.interact(rect, egui::Id::new("find-bar"), Sense::click());
        }
        let radius = CornerRadius::same(18);
        let painter = ui.painter();
        painter.add(card_shadow(&p, 14).as_shape(rect, radius));
        painter.rect_filled(rect, radius, p.glass);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, p.card_stroke),
            egui::StrokeKind::Inside,
        );
        let (mut step, mut close) = (None, false);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(12.0, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), Icon::Search, r, p.text_dim);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut bar.query)
                        .hint_text("Find on page")
                        .frame(egui::Frame::NONE)
                        .vertical_align(Align::Center)
                        .desired_width(150.0),
                );
                if std::mem::take(&mut bar.focus) {
                    field.request_focus();
                    // Select what is there, so typing replaces the last search.
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field.id) {
                        let all = egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(bar.query.chars().count()),
                        );
                        state.cursor.set_char_range(Some(all));
                        state.store(ui.ctx(), field.id);
                    }
                }
                if field.lost_focus() {
                    let (enter, shift, escape) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.modifiers.shift,
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if enter {
                        step = Some(!shift);
                        field.request_focus();
                    }
                    close |= escape;
                }
                let n = bar.hits.len();
                let count = match (n, bar.query.trim().is_empty()) {
                    (_, true) => String::new(),
                    (0, false) => "No matches".to_owned(),
                    (n, false) => format!("{} of {n}", bar.current.min(n - 1) + 1),
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icons::sized_button(ui, Icon::Close, "Close find", false, 24.0).clicked() {
                        close = true;
                    }
                    for (forward, label, turn) in
                        [(true, "Next match", 1.0), (false, "Previous match", -1.0)]
                    {
                        let (r, response) =
                            ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                        response
                            .widget_info(|| WidgetInfo::labeled(WidgetType::Button, n > 0, label));
                        let (hover, _) = icons::feedback(ui, &response);
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(7),
                            p.hover.gamma_multiply(hover * 1.5),
                        );
                        let color = if n > 0 { p.text_dim } else { p.text_faint };
                        icons::chevron(ui.painter(), r.shrink(7.0), color, turn);
                        if response.on_hover_text(label).clicked() {
                            step = Some(forward);
                        }
                    }
                    ui.label(RichText::new(count).size(11.5).color(p.text_faint));
                });
            },
        );
        if let Some(forward) = step {
            bar.step(forward);
        }
        if close && open {
            bar.hide();
        }
        rect.bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_text_in_reading_order_ignoring_case_inside_instances_too() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig");
        let doc = Document::open(path).expect("fixture");
        let page = doc.pages[0];
        // "ABC" is the component's text layer and an instance of it; the other instance
        // overrides its text with "XYZ".
        let abc = find_text(&doc, page, "abc");
        assert_eq!(abc.len(), 2, "{abc:?}");
        assert!(abc.iter().any(|h| matches!(h, Hit::Node(_))), "{abc:?}");
        assert!(abc.iter().any(|h| matches!(h, Hit::Inside(_))), "{abc:?}");
        let top = |h: &Hit| h.bounds(&doc).map(|b| b.top());
        assert!(top(&abc[0]) <= top(&abc[1]), "top first");
        let xyz = find_text(&doc, page, " XyZ ");
        assert_eq!(xyz.len(), 1, "case and spaces");
        assert!(matches!(xyz[0], Hit::Inside(_)), "an override is found");
        assert!(find_text(&doc, page, "").is_empty());
        assert!(find_text(&doc, page, "nowhere on this page").is_empty());
        // The names drawn above frames are found too, like any text on the page.
        let named = find_text(&doc, page, "symbols AND images");
        assert_eq!(named.len(), 1, "{named:?}");
        let Hit::Name(frame) = named[0] else {
            panic!("a frame name: {named:?}")
        };
        assert_eq!(
            doc.node(frame).map(|n| n.name.as_str()),
            Some("Symbols and images")
        );
        assert!(
            find_text(&doc, page, "Symbol and images").is_empty(),
            "as in a browser, the words must appear as typed"
        );
    }

    #[test]
    fn stepping_wraps_around() {
        let mut bar = FindBar {
            hits: vec![
                Hit::Node(Guid(1, 1)),
                Hit::Node(Guid(1, 2)),
                Hit::Node(Guid(1, 3)),
            ],
            ..FindBar::default()
        };
        bar.step(false);
        assert_eq!(bar.current, 2);
        bar.step(true);
        assert_eq!(bar.current, 0);
    }
}
