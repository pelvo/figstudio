//! The command bar (⌘L / Ctrl+L, or a click on the sidebar's top field), Arc style: one search
//! over the app's commands, the open files, and the pages and layers of the active file.

use egui::{
    Color32, CornerRadius, Id, Key, KeyboardShortcut, Margin, Modifiers, Rect, Sense, Shape,
    Stroke, WidgetInfo, WidgetType, pos2, vec2,
};
use figstudio_fig::Guid;

use crate::anim::{self, Spring};
use crate::chrome::elided;
use crate::icons::{self, Icon};
use crate::menu::{self, Action};
use crate::{App, theme};

/// Most rows the bar lists, and most layers it looks at per search.
const MAX_RESULTS: usize = 60;
const MAX_LAYERS_SCANNED: usize = 200_000;

/// What a result list was made for: the query, the active file (`Tab::id`) and its page, and
/// how many files are open.
type ResultsKey = (String, Option<(u64, Guid)>, usize);

#[derive(Debug, Default)]
pub struct CommandBar {
    open: bool,
    query: String,
    selected: usize,
    /// Give the field focus on the next frame (the bar just opened).
    focus: bool,
    /// The entries for `results_for` (the query, the active file and its page, how many files
    /// are open), so a big file is not searched every frame and a list never outlives its
    /// file or page (an agent may turn the page while the bar is open).
    results: Vec<Entry>,
    results_for: Option<ResultsKey>,
}

impl CommandBar {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        *self = Self {
            open: true,
            focus: true,
            ..Self::default()
        };
        anim::reset(ctx, Id::new("command-pop"), 0.0);
    }

    /// Closes it. The results stay on screen while it fades away; `show` starts afresh.
    pub fn hide(&mut self) {
        self.open = false;
        self.results_for = None;
    }

    /// What the bar lists now.
    #[cfg(test)]
    pub fn results(&self) -> &[Entry] {
        &self.results
    }

    /// Open, or still fading out: modal until it is gone.
    pub fn is_up(&self, ctx: &egui::Context) -> bool {
        self.open || anim::on_screen(ctx, Id::new("command-pop"))
    }

    pub fn toggle(&mut self, ctx: &egui::Context) {
        if self.open {
            self.hide()
        } else {
            self.show(ctx)
        }
    }
}

/// What a row does when picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The URL this result was made for, independent of later edits to the search field.
    Link(String),
    Action(Action),
    Settings(crate::settings::Tab),
    File(usize),
    Page(Guid),
    Layer(Guid),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub title: String,
    pub detail: String,
    pub icon: Icon,
    pub target: Target,
    pub shortcut: Option<KeyboardShortcut>,
    /// The space's colour and symbol, for file rows.
    pub space: Option<(crate::theme::Colour, Option<crate::spaces::Symbol>)>,
    score: u32,
}

/// How well `text` matches `query`, ignoring case: a prefix beats a word start beats any
/// substring beats the letters in order. `None` when it does not match.
pub fn score(text: &str, query: &str) -> Option<u32> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(1);
    }
    score_lower(text, &query, &mut String::new())
}

/// [`score`] for a query already trimmed and lowercased, lowercasing `text` into `buf` (kept
/// between calls, so a walk over many layers allocates once).
fn score_lower(text: &str, query: &str, buf: &mut String) -> Option<u32> {
    buf.clear();
    buf.extend(text.chars().flat_map(char::to_lowercase));
    let text = buf.as_str();
    if text.starts_with(query) {
        return Some(400 - text.len().saturating_sub(query.len()).min(100) as u32);
    }
    if let Some(at) = text.find(query) {
        let word_start = text[..at]
            .chars()
            .last()
            .is_some_and(|c| !c.is_alphanumeric());
        return Some(if word_start { 300 } else { 200 } - at.min(100) as u32);
    }
    let mut rest = text.chars();
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .all(|c| rest.any(|t| t == c))
        .then_some(50)
}

/// Every enabled menu command, titled like "Export page as PNG @2x…".
fn commands(
    items: &[menu::Item],
    group: &str,
    parent: Option<&str>,
    keys: &crate::keymap::Keymap,
    out: &mut Vec<Entry>,
) {
    for item in items {
        if item.label.is_empty() || !item.enabled {
            continue;
        }
        // "Zoom to fit   ⇧1": the hint goes, the shortcut column shows it.
        let label = item.label.split("   ").next().unwrap_or(item.label);
        let title = match parent {
            Some(parent) => format!("{parent} {label}"),
            None => label.to_owned(),
        };
        let Some(action) = item.action else {
            commands(&item.children, group, Some(&title), keys, out);
            continue;
        };
        if action == Action::CommandBar {
            continue;
        }
        let detail = match item.checked {
            Some(true) => format!("{group} · on"),
            Some(false) => format!("{group} · off"),
            None => group.to_owned(),
        };
        out.push(Entry {
            title,
            detail,
            icon: Icon::Dot,
            target: Target::Action(action),
            shortcut: keys.for_action(action),
            space: None,
            score: 0,
        });
    }
}

/// What the command bar lists for `query`, best first. An empty query lists the other open
/// files, the pages, then the commands; layers need a query. A supported URL naming a file
/// offers the existing local-link opener instead of searching layer names for the URL.
pub fn entries(app: &App, query: &str) -> Vec<Entry> {
    let query = query.trim();
    if figstudio_mcp_core::link::parse_link(query).is_ok_and(|link| link.file.is_some()) {
        return vec![Entry {
            title: "Open Figma link".into(),
            detail: "Open the matching local .fig file".into(),
            icon: Icon::Page,
            target: Target::Link(query.to_owned()),
            shortcut: None,
            space: None,
            score: 0,
        }];
    }
    let mut all = Vec::new();
    for (i, tab) in app.tabs.iter().enumerate() {
        let renamed = tab.style.name.is_some();
        let detail = match (i == app.active, renamed) {
            (true, false) => "Open file · current".to_owned(),
            (false, false) => "Open file".to_owned(),
            (true, true) => format!("{} · current", tab.title()),
            (false, true) => tab.title(),
        };
        all.push(Entry {
            title: tab.space_name(),
            detail,
            icon: Icon::Dot,
            target: Target::File(i),
            shortcut: None,
            space: Some((tab.colour(), tab.style.symbol.clone())),
            score: 0,
        });
    }
    let tab = app.tabs.get(app.active);
    if let Some(tab) = tab {
        for &page in tab.doc.pages.iter().chain(&tab.doc.internal_pages) {
            let name = tab
                .doc
                .node(page)
                .map(|n| n.name.clone())
                .unwrap_or_default();
            all.push(Entry {
                title: name,
                detail: match (tab.doc.internal_pages.contains(&page), page == tab.page) {
                    (true, true) => "Internal page · current",
                    (true, false) => "Internal page",
                    (false, true) => "Page · current",
                    (false, false) => "Page",
                }
                .into(),
                icon: Icon::Page,
                target: Target::Page(page),
                shortcut: None,
                space: None,
                score: 0,
            });
        }
    }
    for group in menu::tree(app) {
        commands(
            &group.children,
            group.label,
            None,
            &app.prefs.keymap,
            &mut all,
        );
    }
    for tab in crate::settings::Tab::ALL {
        all.push(Entry {
            title: format!("{} settings", tab.label()),
            detail: "Settings".into(),
            icon: tab.icon(),
            target: Target::Settings(tab),
            shortcut: None,
            space: None,
            score: 0,
        });
    }
    let mut kept: Vec<Entry> = all
        .into_iter()
        .filter_map(|mut e| {
            e.score = score(&e.title, query)?;
            Some(e)
        })
        .filter(|e| !query.is_empty() || !matches!(e.target, Target::File(i) if i == app.active))
        .collect();
    if let Some(tab) = tab.filter(|_| !query.is_empty()) {
        kept.extend(layers(tab, query));
    }
    // Stable: equal scores keep files, pages, commands, layers in that order.
    kept.sort_by_key(|e| std::cmp::Reverse(e.score));
    kept.truncate(MAX_RESULTS);
    kept
}

/// The page's layers matching `query`, best first, at most [`MAX_RESULTS`]. Each name is
/// scored during the walk and only matches are kept, so a keystroke on a big file stays
/// cheap.
fn layers(tab: &crate::tab::Tab, query: &str) -> Vec<Entry> {
    let doc = &tab.doc;
    let query = query.to_lowercase();
    let mut buf = String::new();
    // Best first, then in layer order; only the best [`MAX_RESULTS`] are kept as the walk
    // goes (trimmed now and then), so a short query on a big page does not hold every match.
    let order = |(score, seq, _): &(u32, usize, Guid)| (std::cmp::Reverse(*score), *seq);
    let keep = |found: &mut Vec<(u32, usize, Guid)>| {
        if found.len() > MAX_RESULTS {
            found.select_nth_unstable_by_key(MAX_RESULTS - 1, order);
            found.truncate(MAX_RESULTS);
        }
    };
    let mut found: Vec<(u32, usize, Guid)> = Vec::new();
    let mut stack: Vec<Guid> = doc.children(tab.page).iter().rev().copied().collect();
    let mut scanned = 0;
    while let Some(id) = stack.pop() {
        scanned += 1;
        if scanned > MAX_LAYERS_SCANNED {
            break;
        }
        let Some(node) = doc.node(id) else { continue };
        if !node.name.is_empty()
            && let Some(score) = score_lower(&node.name, &query, &mut buf)
        {
            found.push((score, scanned, id));
            if found.len() >= 4 * MAX_RESULTS {
                keep(&mut found);
            }
        }
        stack.extend(doc.children(id).iter().rev().copied());
    }
    keep(&mut found);
    found.sort_by_key(order);
    found
        .into_iter()
        .filter_map(|(score, _, id)| {
            let node = doc.node(id)?;
            Some(Entry {
                title: node.name.clone(),
                detail: format!("Layer · {}", node.kind.to_lowercase().replace('_', " ")),
                icon: icons::for_kind(&node.kind),
                target: Target::Layer(id),
                shortcut: None,
                space: None,
                score,
            })
        })
        .collect()
}

impl App {
    /// Refresh the results when their query or file changed. Returns whether the selection
    /// was reset; called again after editing so paste and Enter in one frame use that paste.
    fn refresh_command_results(&mut self) -> bool {
        let key = (
            self.command.query.clone(),
            self.tabs.get(self.active).map(|t| (t.id, t.page)),
            self.tabs.len(),
        );
        if self.command.results_for.as_ref() == Some(&key) {
            return false;
        }
        self.command.results = entries(self, &self.command.query);
        self.command.results_for = Some(key);
        self.command.selected = 0;
        true
    }

    /// Draws the command bar while it is open, and runs what gets picked.
    pub(crate) fn command_bar(&mut self, ctx: &egui::Context) {
        // It pops in, and after closing fades and shrinks away taking no input.
        let open = self.command.open;
        let pop = anim::presence(ctx, Id::new("command-pop"), open);
        if !open && pop <= 0.001 {
            self.command.results.clear();
            return;
        }
        if open {
            self.refresh_command_results();
        }
        let (down, up, enter, escape) = if open {
            ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            })
        } else {
            (false, false, false, false)
        };
        let bar = &mut self.command;
        let count = bar.results.len();
        let keyed = down || up;
        if count > 0 {
            if down {
                bar.selected = (bar.selected + 1) % count;
            }
            if up {
                bar.selected = (bar.selected + count - 1) % count;
            }
        }
        let mut run = enter.then_some(bar.selected).filter(|&i| i < count);
        let mut close = escape;

        let p = theme::palette(ctx);
        let screen = ctx.content_rect();
        // Both stay interactable while they fade (inert, sensing only hover): a click then
        // does nothing rather than land on the app under the dimmed screen.
        egui::Area::new(Id::new("command-backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let sense = if open { Sense::CLICK } else { Sense::hover() };
                let response = ui.allocate_rect(screen, sense);
                crate::a11y::hide(&response);
                let dim = if p.dark { 110.0 } else { 40.0 };
                let alpha = (dim * pop.clamp(0.0, 1.0)) as u8;
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(alpha));
                close |= response.clicked();
            });
        let width = (screen.width() - 48.0).clamp(280.0, 600.0);
        let top = screen.top() + (screen.height() * 0.16).max(40.0);
        let moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        let area = egui::Area::new(Id::new("command-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(screen.center().x - width / 2.0, top))
            .fade_in(false)
            .show(ctx, |ui| {
                if !open {
                    anim::inert(ui);
                }
                ui.set_width(width);
                ui.multiply_opacity(pop.clamp(0.0, 1.0));
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .corner_radius(CornerRadius::same(16))
                    .stroke(Stroke::new(1.0, p.card_stroke))
                    .shadow(egui::Shadow {
                        offset: [0, 18],
                        blur: 48,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 150 } else { 50 }),
                    })
                    .inner_margin(Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(width - 12.0);
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            let (r, _) = ui.allocate_exact_size(vec2(18.0, 44.0), Sense::hover());
                            let icon = Rect::from_center_size(r.center(), vec2(17.0, 17.0));
                            icons::paint(ui.painter(), Icon::Search, icon, p.text_dim);
                            let edit = ui.add_sized(
                                vec2(ui.available_width() - 8.0, 44.0),
                                egui::TextEdit::singleline(&mut bar.query)
                                    .hint_text("Search or paste a Figma link")
                                    .font(egui::FontId::proportional(17.0))
                                    .vertical_align(egui::Align::Center)
                                    .frame(egui::Frame::NONE)
                                    .margin(Margin::symmetric(6, 0)),
                            );
                            crate::a11y::name_field(&edit, "Search or paste a Figma link");
                            if std::mem::take(&mut bar.focus) {
                                edit.request_focus();
                            }
                        });
                        if bar.results.is_empty() {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                ui.label(egui::RichText::new("No matches").color(p.text_faint));
                            });
                            ui.add_space(10.0);
                            return;
                        }
                        let line = ui.cursor().top();
                        ui.painter().hline(
                            ui.max_rect().x_range(),
                            line,
                            Stroke::new(1.0, p.separator),
                        );
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height(400.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let highlight = ui.painter().add(Shape::Noop);
                                let origin = ui.min_rect().min;
                                let mut lit = None;
                                for (i, e) in bar.results.iter().enumerate() {
                                    let (rect, response) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), 40.0),
                                        Sense::click(),
                                    );
                                    response.widget_info(|| {
                                        WidgetInfo::labeled(WidgetType::Button, true, &e.title)
                                    });
                                    if response.hovered() && moved {
                                        bar.selected = i;
                                    }
                                    if i == bar.selected {
                                        lit = Some(rect);
                                        if keyed {
                                            response.scroll_to_me(None);
                                        }
                                    }
                                    if response.clicked() {
                                        run = Some(i);
                                    }
                                    paint_row(ui, rect, e, i == bar.selected, &p);
                                }
                                if let Some(rect) = lit {
                                    let r = anim::spring_rect(
                                        ui.ctx(),
                                        Id::new("command-highlight"),
                                        rect.translate(-origin.to_vec2()),
                                        Spring::SMOOTH,
                                    )
                                    .translate(origin.to_vec2());
                                    let fill = if p.dark {
                                        p.selected
                                    } else {
                                        theme::ACCENT.gamma_multiply(0.10)
                                    };
                                    ui.painter().set(
                                        highlight,
                                        Shape::rect_filled(r, CornerRadius::same(10), fill),
                                    );
                                }
                            });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new("↑↓ to move · Enter to open · Esc to close")
                                    .size(11.0)
                                    .color(p.text_faint),
                            );
                        });
                        ui.add_space(2.0);
                    });
            });
        let layer = area.response.layer_id;
        ctx.move_to_top(layer);
        let scale = 0.94 + 0.06 * pop;
        let centre = area.response.rect.center().to_vec2();
        ctx.set_transform_layer(
            layer,
            egui::emath::TSTransform::new(
                centre * (1.0 - scale) - vec2(0.0, (1.0 - pop) * 8.0),
                scale,
            ),
        );

        if !open {
            return;
        }
        let clicked = run
            .and_then(|i| self.command.results.get(i))
            .map(|entry| entry.target.clone());
        if self.refresh_command_results() {
            // Enter uses the new query; a click is valid only if its row still does the same thing.
            run = if enter {
                Some(0)
            } else {
                run.filter(|&i| {
                    self.command
                        .results
                        .get(i)
                        .is_some_and(|entry| Some(&entry.target) == clicked.as_ref())
                })
            };
            ctx.request_repaint();
        }
        if let Some(entry) = run.and_then(|i| self.command.results.get(i)) {
            let target = entry.target.clone();
            self.command.hide();
            self.run_target(target, ctx);
        } else if close {
            self.command.hide();
        }
    }

    fn run_target(&mut self, target: Target, ctx: &egui::Context) {
        match target {
            Target::Link(link) => {
                self.link.text = link;
                self.apply(Action::OpenLink, ctx);
                self.submit_link(ctx);
            }
            Target::Action(action) => self.apply(action, ctx),
            Target::Settings(tab) => self.settings.show(Some(tab), ctx),
            Target::File(i) => {
                if i < self.tabs.len() {
                    self.active = i;
                }
            }
            Target::Page(page) => {
                if let Some(tab) = self.tab() {
                    tab.set_page(page);
                }
            }
            Target::Layer(id) => {
                if let Some(tab) = self.tab() {
                    tab.select(Some(id));
                    tab.fit_node = Some(id);
                }
            }
        }
    }
}

fn paint_row(ui: &egui::Ui, rect: Rect, e: &Entry, selected: bool, p: &theme::Palette) {
    let painter = ui.painter();
    let tile = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(26.0, 26.0));
    match &e.space {
        Some((colour, symbol)) => {
            let tint = theme::space_tint(*colour, p.dark);
            painter.rect_filled(
                tile,
                CornerRadius::same(8),
                tint.top.lerp_to_gamma(tint.accent, 0.25),
            );
            crate::chrome::paint_badge(
                painter,
                tile.shrink(3.0),
                symbol.as_ref(),
                tint.accent,
                p.text,
            );
        }
        None => {
            painter.rect_filled(tile, CornerRadius::same(8), p.well);
            let icon = if matches!(e.target, Target::Action(_)) {
                command_icon(e)
            } else {
                e.icon
            };
            icons::paint(painter, icon, tile.shrink(6.0), p.text_dim);
        }
    }
    let shortcut = e.shortcut.map(|s| ui.ctx().format_shortcut(&s));
    let right = shortcut.as_deref().unwrap_or(&e.detail);
    let detail = elided(
        ui,
        right,
        egui::FontId::proportional(11.5),
        p.text_faint,
        160.0,
    );
    let detail_at = pos2(
        rect.right() - 12.0 - detail.size().x,
        rect.center().y - detail.size().y / 2.0,
    );
    let width = detail_at.x - (rect.left() + 46.0) - 12.0;
    let font = if selected {
        theme::semibold(13.5)
    } else {
        egui::FontId::proportional(13.5)
    };
    let title = elided(ui, &e.title, font, p.text, width);
    painter.galley(
        pos2(rect.left() + 46.0, rect.center().y - title.size().y / 2.0),
        title,
        p.text,
    );
    painter.galley(detail_at, detail, p.text_faint);
}

/// An icon that hints at what a command does.
fn command_icon(e: &Entry) -> Icon {
    match e.target {
        Target::Action(Action::Open) => Icon::Plus,
        Target::Action(Action::ExportPage(_) | Action::ExportSelection(_)) => Icon::Image,
        Target::Action(Action::CloseTab | Action::Quit) => Icon::Close,
        Target::Action(Action::ZoomFit | Action::ActualSize) => Icon::Fit,
        Target::Action(Action::ToggleLayers) => Icon::Sidebar,
        Target::Action(Action::ToggleProperties) => Icon::Inspector,
        Target::Action(Action::SpaceIcon | Action::SpaceColour | Action::ResetSpace) => {
            Icon::Ellipse
        }
        Target::Action(Action::RenameSpace) => Icon::Text,
        Target::Action(Action::FindOnPage) => Icon::Search,
        Target::Action(Action::SelectTool(crate::canvas::Tool::Hand)) => Icon::Hand,
        _ => Icon::More,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page of `n` layers: every third named "match …" (a prefix), the others "a match"
    /// (a word start), so many share each score.
    fn many_layers(n: u32) -> crate::tab::Tab {
        use figstudio_fig::{Document, Node, Obj, Val};
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
            let name = if i % 3 == 0 {
                format!("match {i}")
            } else {
                format!("a match {i}")
            };
            nodes.push(
                Node::from_data(obj(&[
                    ("guid", Guid(9, i + 2).to_val()),
                    ("type", Val::Str("FRAME".into())),
                    ("name", Val::Str(name)),
                    ("parentIndex", parent(page, format!("{i:04}"))),
                ]))
                .expect("layer"),
            );
        }
        let bytes = include_bytes!("../../../tests/fixtures/structure.fig");
        let mut doc = Document::from_bytes(bytes).expect("fixture");
        doc.set_nodes(nodes).expect("nodes");
        doc.pages = vec![page];
        let mut tab = crate::tab::Tab::new(
            1,
            "a.fig".into(),
            std::sync::Arc::new(doc),
            None,
            std::time::Duration::ZERO,
        );
        tab.page = page;
        tab
    }

    #[test]
    fn the_layers_kept_are_the_best_in_layer_order() {
        let tab = many_layers(600);
        let got: Vec<String> = layers(&tab, "match").into_iter().map(|e| e.title).collect();
        // All 200 prefixes score alike and beat the word starts: the first 60 in the tree.
        let want: Vec<String> = (0..MAX_RESULTS as u32)
            .map(|k| format!("match {}", k * 3))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_prefix_beats_a_word_start_beats_a_substring_beats_letters_in_order() {
        let prefix = score("Zoom to fit", "zoom").unwrap();
        let word = score("Actual zoom", "zoom").unwrap();
        let inner = score("Autozoom", "zoom").unwrap();
        let letters = score("Zero overlap moves", "zoom").unwrap();
        assert!(
            prefix > word && word > inner && inner > letters,
            "{prefix} {word} {inner} {letters}"
        );
        assert_eq!(score("Pages", "xyz"), None);
        assert_eq!(score("PAGE 1", "page"), score("page 1", "PAGE"));
        assert!(
            score("anything", "  ").is_some(),
            "an empty query matches all"
        );
    }

    #[test]
    fn internal_pages_are_command_bar_targets_and_request_fit() {
        let mut h = crate::ui_tests::harness();
        let normal = h.state().tabs[0].doc.pages[0];
        let internal = h.state().tabs[0].doc.internal_pages[0];
        let detail = |app: &crate::App, page| {
            entries(app, "")
                .into_iter()
                .find(|e| e.target == Target::Page(page))
                .unwrap()
                .detail
        };
        assert_eq!(detail(h.state(), normal), "Page · current");
        assert_eq!(detail(h.state(), internal), "Internal page");
        let name = h.state().tabs[0].doc.node(internal).unwrap().name.clone();
        assert!(
            entries(h.state(), &name)
                .iter()
                .any(|e| e.target == Target::Page(internal))
        );
        h.state_mut().tabs[0].fit_pending = false;
        let ctx = h.ctx.clone();
        h.state_mut().run_target(Target::Page(internal), &ctx);
        assert_eq!(h.state().tabs[0].page, internal);
        assert_eq!(h.state().tabs[0].selection, None);
        assert!(h.state().tabs[0].fit_pending);
        assert_eq!(detail(h.state(), internal), "Internal page · current");
        assert_eq!(detail(h.state(), normal), "Page");
    }

    #[test]
    fn the_bar_finds_commands_pages_and_layers_of_the_open_file() {
        let h = crate::ui_tests::harness();
        let app = h.state();
        let titles =
            |q: &str| -> Vec<String> { entries(app, q).into_iter().map(|e| e.title).collect() };
        assert!(titles("").iter().any(|t| t == "Page 1"), "{:?}", titles(""));
        assert!(
            titles("").iter().all(|t| t != "structure.fig"),
            "the current file is not offered"
        );
        let fit = entries(app, "zoom to fit");
        assert_eq!(
            fit.first().map(|e| &e.target),
            Some(&Target::Action(Action::ZoomFit))
        );
        assert!(
            titles("export page")
                .iter()
                .any(|t| t == "Export page as PNG @2x…"),
            "{:?}",
            titles("export page")
        );
        let layer = entries(app, "symbols and images");
        assert!(
            matches!(layer.first().map(|e| &e.target), Some(Target::Layer(_))),
            "{layer:?}"
        );
        assert!(titles("no such thing anywhere").is_empty());
    }
}
