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
    find_at_most(doc, page, query, MAX_HITS)
}

/// Where a match sits in reading order: its box's top, then its left.
type Place = (f32, f32);

fn place_of(b: Option<Rect>) -> Place {
    b.map_or((f32::MAX, f32::MAX), |b| (b.top(), b.left()))
}

fn by_place(a: &Place, b: &Place) -> std::cmp::Ordering {
    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Keeps the first `cap` of `hits` in reading order. Returns where the last kept one sits:
/// nothing lower can be among them any more.
fn keep_top(hits: &mut Vec<(Place, Hit)>, cap: usize) -> Option<Place> {
    if hits.len() > cap {
        hits.select_nth_unstable_by(cap - 1, |a, b| by_place(&a.0, &b.0));
        hits.truncate(cap);
    }
    (hits.len() >= cap)
        .then(|| hits.iter().map(|h| h.0).max_by(by_place))
        .flatten()
}

/// [`find_text`] keeping the top `cap` matches of the page (`cap` ≥ 1). Instances are
/// searched from the top of the page down, and the search stops once `cap` matches sit above
/// the next instance, so a big page is not walked to its end for matches that would be cut.
fn find_at_most(doc: &Document, page: Guid, query: &str, cap: usize) -> Vec<Hit> {
    let cap = cap.max(1);
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let matches = |text: &str| text.to_lowercase().contains(&query);
    let placed = |hit: Hit| (place_of(hit.bounds(doc)), hit);
    let mut hits: Vec<(Place, Hit)> = crate::labels::named(doc, page)
        .into_iter()
        .filter(|id| doc.node(*id).is_some_and(|n| matches(&n.name)))
        .map(|id| placed(Hit::Name(id)))
        .collect();
    let mut instances = Vec::new();
    let mut stack: Vec<Guid> = doc.children(page).to_vec();
    while let Some(id) = stack.pop() {
        let node = doc.node(id);
        // A hidden layer is not drawn, and neither is anything inside it.
        if node.is_some_and(|n| n.data.flag("visible") == Some(false)) {
            continue;
        }
        stack.extend(doc.children(id).iter().copied());
        let Some(node) = node else { continue };
        if node.kind == "INSTANCE" {
            instances.push(placed(Hit::Node(id)));
        } else if crate::copy::text(node).is_some_and(matches) {
            hits.push(placed(Hit::Node(id)));
        }
    }
    // Top of the page first, so the walk can stop where the kept matches end.
    instances.sort_by(|a, b| by_place(&a.0, &b.0));
    let mut lowest_kept = keep_top(&mut hits, cap);
    let definitions = std::sync::Arc::new(figstudio_fig::resolve::definitions(doc));
    let mut renderer = figstudio_render::Renderer::new();
    let mut budget = MAX_INSIDE;
    for (top, instance) in instances {
        let Hit::Node(instance) = instance else {
            continue;
        };
        if budget == 0 || lowest_kept.is_some_and(|low| by_place(&top, &low).is_gt()) {
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
            // No bounds for a layer hidden in the instance, or under one: it is not drawn.
            if let Some(b) = renderer.path_bounds(doc, &inside.path) {
                let rect = Rect::from_min_max(
                    Pos2::new(b.x0 as f32, b.y0 as f32),
                    Pos2::new(b.x1 as f32, b.y1 as f32),
                );
                hits.push((place_of(Some(rect)), Hit::Inside(rect)));
            }
        }
        // Trimmed now and then, not per match: a cut costs a pass over what is kept.
        if hits.len() >= cap.saturating_mul(2) {
            lowest_kept = keep_top(&mut hits, cap);
        }
    }
    keep_top(&mut hits, cap);
    // Reading order: rows of the page from the top, left to right within a row.
    hits.sort_by(|a, b| by_place(&a.0, &b.0));
    hits.into_iter().map(|(_, hit)| hit).collect()
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
            crate::a11y::hide(&ui.interact(rect, egui::Id::new("find-bar"), Sense::CLICK));
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
                crate::a11y::name_field(&field, "Find on page");
                crate::a11y::author(&field, "find.query");
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
                        crate::a11y::author(
                            &response,
                            format!("find.{}", crate::a11y::slug(label)),
                        );
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
                    if !count.is_empty() {
                        ui.label(RichText::new(count).size(11.5).color(p.text_faint));
                    }
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

    /// The fixture with every node `hide` picks set hidden.
    fn fixture_hiding(hide: impl Fn(&figstudio_fig::Node) -> bool) -> Document {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig");
        let mut doc = Document::open(path).expect("fixture");
        let mut nodes = doc.nodes.clone();
        for node in nodes.iter_mut().filter(|n| hide(n)) {
            if let figstudio_fig::Val::Obj(data) = &mut node.data {
                data.insert("visible", figstudio_fig::Val::Bool(false));
            }
        }
        doc.set_nodes(nodes).expect("nodes");
        doc
    }

    #[test]
    fn hidden_layers_and_what_is_inside_them_are_not_found() {
        // The component's "ABC" text layer hidden: neither it nor the instances drawing it
        // (one of them with its text overridden to "XYZ") show any text.
        let abc = |n: &figstudio_fig::Node| crate::copy::text(n).is_some_and(|t| t == "ABC");
        let doc = fixture_hiding(abc);
        let page = doc.pages[0];
        assert!(
            find_text(&doc, page, "abc").is_empty(),
            "the layer and an instance"
        );
        assert!(
            find_text(&doc, page, "xyz").is_empty(),
            "inside an instance"
        );
        // The instances hidden: the component's own layer is still found, theirs are not
        // (they are not walked; `path_bounds` would give a hidden one no bounds either).
        let doc = fixture_hiding(|n| n.kind == "INSTANCE");
        let page = doc.pages[0];
        let found = find_text(&doc, page, "abc");
        assert!(matches!(found[..], [Hit::Node(_)]), "{found:?}");
        assert!(find_text(&doc, page, "xyz").is_empty());
        // On a page of its own: a visible "match", a hidden one, and one in a hidden frame.
        let doc = hidden_and_shown();
        let found = find_text(&doc, doc.pages[0], "match");
        assert!(
            matches!(found[..], [Hit::Node(id)] if id == Guid(9, 2)),
            "{found:?}"
        );
    }

    /// A page with a text layer "match" shown, one hidden, and one in a hidden frame.
    fn hidden_and_shown() -> Document {
        use figstudio_fig::{Node, Obj, Val};
        let obj = |fields: &[(&str, Val)]| {
            let mut o = Obj::new();
            for (k, v) in fields {
                o.insert(k, v.clone());
            }
            Val::Obj(o)
        };
        let (page, frame) = (Guid(9, 1), Guid(9, 5));
        let layer = |id: Guid, kind: &str, under: Guid, y: f64, shown: bool| {
            let mut fields = vec![
                ("guid", id.to_val()),
                ("type", Val::Str(kind.into())),
                ("name", Val::Str(kind.into())),
                (
                    "parentIndex",
                    obj(&[
                        ("guid", under.to_val()),
                        ("position", Val::Str(format!("{y}"))),
                    ]),
                ),
                (
                    "size",
                    obj(&[("x", Val::Float(80.0)), ("y", Val::Float(20.0))]),
                ),
                (
                    "transform",
                    obj(&[
                        ("m00", Val::Float(1.0)),
                        ("m11", Val::Float(1.0)),
                        ("m12", Val::Float(y)),
                    ]),
                ),
                ("visible", Val::Bool(shown)),
            ];
            if kind == "TEXT" {
                fields.push(("textData", obj(&[("characters", Val::Str("match".into()))])));
            }
            Node::from_data(obj(&fields)).expect("layer")
        };
        let nodes = vec![
            Node::from_data(obj(&[
                ("guid", page.to_val()),
                ("type", Val::Str("CANVAS".into())),
                ("name", Val::Str("Page".into())),
                (
                    "parentIndex",
                    obj(&[
                        ("guid", Guid(0, 0).to_val()),
                        ("position", Val::Str("0".into())),
                    ]),
                ),
            ]))
            .expect("page"),
            layer(Guid(9, 2), "TEXT", page, 0.0, true),
            layer(Guid(9, 3), "TEXT", page, 40.0, false),
            layer(frame, "FRAME", page, 80.0, false),
            layer(Guid(9, 4), "TEXT", frame, 0.0, true),
        ];
        let bytes = include_bytes!("../../../tests/fixtures/structure.fig");
        let mut doc = Document::from_bytes(bytes).expect("fixture");
        doc.set_nodes(nodes).expect("nodes");
        doc.pages = vec![page];
        doc
    }

    /// A page of text layers "match" stacked top to bottom, listed bottom first in the
    /// layer tree, so the walk meets the lowest first.
    fn stacked_text(n: u32) -> Document {
        use figstudio_fig::{Node, Obj, Val};
        let obj = |fields: &[(&str, Val)]| {
            let mut o = Obj::new();
            for (k, v) in fields {
                o.insert(k, v.clone());
            }
            Val::Obj(o)
        };
        let parent = |guid: Guid, position: String| {
            obj(&[("guid", guid.to_val()), ("position", Val::Str(position))])
        };
        let page = Guid(9, 1);
        let mut nodes = vec![
            Node::from_data(obj(&[
                ("guid", page.to_val()),
                ("type", Val::Str("CANVAS".into())),
                ("name", Val::Str("Page".into())),
                ("parentIndex", parent(Guid(0, 0), "0".into())),
            ]))
            .expect("page"),
        ];
        for i in 0..n {
            nodes.push(
                Node::from_data(obj(&[
                    ("guid", Guid(9, i + 2).to_val()),
                    ("type", Val::Str("TEXT".into())),
                    ("name", Val::Str("Caption".into())),
                    ("parentIndex", parent(page, format!("{i:03}"))),
                    (
                        "size",
                        obj(&[("x", Val::Float(80.0)), ("y", Val::Float(20.0))]),
                    ),
                    (
                        "transform",
                        obj(&[
                            ("m00", Val::Float(1.0)),
                            ("m11", Val::Float(1.0)),
                            ("m12", Val::Float(f64::from(i) * 40.0)),
                        ]),
                    ),
                    ("textData", obj(&[("characters", Val::Str("match".into()))])),
                ]))
                .expect("text"),
            );
        }
        let bytes = include_bytes!("../../../tests/fixtures/structure.fig");
        let mut doc = Document::from_bytes(bytes).expect("fixture");
        doc.set_nodes(nodes).expect("nodes");
        doc.pages = vec![page];
        doc
    }

    #[test]
    fn the_matches_kept_are_the_top_of_the_page_not_the_first_walked() {
        let doc = stacked_text(6);
        let page = doc.pages[0];
        let all = find_at_most(&doc, page, "match", usize::MAX);
        assert_eq!(all.len(), 6);
        let top = |h: &Hit| h.bounds(&doc).map(|b| b.top());
        assert_eq!(top(&all[0]), Some(0.0));
        for cap in 1..6 {
            assert_eq!(
                find_at_most(&doc, page, "match", cap),
                all[..cap],
                "at most {cap}: the top {cap} of the page"
            );
        }
        // And on a real file, with instances and frame names among the matches.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig");
        let doc = Document::open(path).expect("fixture");
        let page = doc.pages[0];
        for query in ["abc", "e", "a", "o"] {
            let all = find_at_most(&doc, page, query, usize::MAX);
            for cap in 1..all.len() {
                assert_eq!(
                    find_at_most(&doc, page, query, cap),
                    all[..cap],
                    "{query:?} {cap}"
                );
            }
        }
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
