//! Sidebar lists (Pages, the Layers tree) and the Properties card.

use egui::{Color32, CornerRadius, FontId, Id, Margin, RichText, Sense, Shape, Ui, vec2};
use figstudio_fig::{Document, Guid, Node, Val};
use figstudio_render::geom;

use crate::anim::{self, Spring};
use crate::copy::Item;
use crate::copy_menu;
use crate::icons::{self, Icon};
use crate::tab::Tab;
use crate::theme::{self, Palette};

/// Height of a layer row.
const ROW: f32 = 26.0;

fn display_name(node: &Node) -> String {
    let name = node.data.text("name").unwrap_or("");
    if name.is_empty() {
        node.kind.to_lowercase()
    } else {
        name.to_string()
    }
}

/// The selected-row pill: white with a soft shadow in light mode, a lighter fill in dark.
fn selected_pill(p: &Palette, rect: egui::Rect) -> Shape {
    let radius = CornerRadius::same(8);
    let fill = Shape::rect_filled(rect, radius, p.selected);
    if p.dark {
        return fill;
    }
    let shadow = egui::Shadow {
        offset: [0, 1],
        blur: 6,
        spread: 0,
        color: Color32::from_black_alpha(22),
    };
    Shape::Vec(vec![shadow.as_shape(rect, radius).into(), fill])
}

/// Fade-in of the hovered row of a list: one state per list, not per row, so long trees do
/// not keep an animation per row.
fn hover_fade(ui: &Ui, list: Id, row: Id, hovered: bool) -> f32 {
    if !hovered {
        return 0.0;
    }
    let ctx = ui.ctx();
    let time = ctx.input(|i| i.time);
    let since = ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_insert_with(list, || (row, time));
        if state.0 != row {
            *state = (row, time);
        }
        state.1
    });
    let t = ((time - since) as f32 / 0.12).clamp(0.0, 1.0);
    if t < 1.0 && !anim::reduced(ctx) {
        ctx.request_repaint();
        egui::emath::easing::cubic_out(t)
    } else {
        1.0
    }
}

/// The page list; the selected page's pill slides when the page changes.
pub fn pages(ui: &mut Ui, tab: &mut Tab) {
    let p = theme::of(ui);
    let doc = tab.doc.clone();
    ui.spacing_mut().item_spacing.y = 1.0;
    let pill = ui.painter().add(Shape::Noop);
    let origin = ui.min_rect().min.to_vec2();
    let mut lit = None;
    for &page in &doc.pages {
        let name = doc.node(page).map(display_name).unwrap_or_default();
        let selected = tab.page == page;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name)
        });
        let hover = hover_fade(ui, Id::new("pages-hover"), response.id, response.hovered());
        if selected {
            lit = Some(rect);
        } else {
            icons::row_fill(ui, rect, hover, false);
        }
        let color = if selected {
            p.text
        } else {
            p.text_dim.lerp_to_gamma(p.text, hover)
        };
        let icon = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 16.0, rect.center().y),
            vec2(14.0, 14.0),
        );
        icons::paint(ui.painter(), Icon::Page, icon, color);
        let font = if selected {
            theme::semibold(13.0)
        } else {
            FontId::proportional(13.0)
        };
        let g = crate::chrome::elided(ui, &name, font, color, rect.width() - 44.0);
        ui.painter().galley(
            egui::pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0),
            g,
            color,
        );
        if response.clicked() {
            tab.set_page(page);
        }
    }
    if let Some(rect) = lit {
        let r = anim::spring_rect(
            ui.ctx(),
            Id::new("page-pill"),
            rect.translate(-origin),
            Spring::SMOOTH,
        )
        .translate(origin);
        ui.painter().set(pill, selected_pill(&p, r));
    }
    if !doc.internal_pages.is_empty() {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "{} internal page(s) hidden",
                    doc.internal_pages.len()
                ))
                .color(p.text_dim)
                .small(),
            );
        });
    }
}

pub fn layers(ui: &mut Ui, tab: &mut Tab, filter: &str) {
    let doc = tab.doc.clone();
    let filter = filter.trim().to_lowercase();
    // Vertical only: sideways swipes over the sidebar switch spaces (`swipe.rs`).
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if filter.is_empty() {
                // Figma lists the topmost layer first, so children are drawn in reverse.
                let mut stack: Vec<_> = doc
                    .children(tab.page)
                    .iter()
                    .rev()
                    .map(|id| (*id, 0usize))
                    .collect();
                while let Some((id, depth)) = stack.pop() {
                    if doc.node(id).is_none() {
                        continue;
                    }
                    row(ui, tab, &doc, id, depth, true);
                    if tab.expanded.contains(&id) {
                        stack.extend(
                            doc.children(id)
                                .iter()
                                .rev()
                                .map(|child| (*child, depth + 1)),
                        );
                    }
                }
            } else {
                let mut shown = 0;
                search(ui, tab, &doc, tab.page, &filter, &mut shown);
                if shown == 0 {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new("No matching layers").color(theme::of(ui).text_dim));
                    });
                }
            }
        });
    tab.reveal = None;
}

fn search(
    ui: &mut Ui,
    tab: &mut Tab,
    doc: &Document,
    parent: Guid,
    filter: &str,
    shown: &mut usize,
) {
    let mut stack: Vec<Guid> = doc.children(parent).iter().rev().copied().collect();
    while let Some(child) = stack.pop() {
        let Some(node) = doc.node(child) else {
            continue;
        };
        if *shown < 2000 && display_name(node).to_lowercase().contains(filter) {
            *shown += 1;
            row(ui, tab, doc, child, 0, false);
        }
        stack.extend(doc.children(child).iter().rev().copied());
    }
}

/// One layer row: chevron (when `tree` and it has children), type icon and name.
fn row(ui: &mut Ui, tab: &mut Tab, doc: &Document, id: Guid, depth: usize, tree: bool) {
    let Some(node) = doc.node(id) else { return };
    let p = theme::of(ui);
    let has_children = tree && !doc.children(id).is_empty();
    let expanded = tab.expanded.contains(&id);
    let selected = tab.selection == Some(id);
    let hidden = node.data.flag("visible") == Some(false);
    let name = display_name(node);

    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width().max(180.0), ROW), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name)
    });
    if tab.reveal == Some(id) {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    if !ui.is_rect_visible(rect) {
        handle_row(tab, &response, id, has_children);
        return;
    }
    let hover = hover_fade(ui, Id::new("layers-hover"), response.id, response.hovered());
    if selected {
        ui.painter().add(selected_pill(&p, rect));
    } else {
        icons::row_fill(ui, rect, hover, false);
    }
    let indent = 6.0 + depth as f32 * 14.0;
    let mid = rect.center().y;
    if has_children {
        let arrow = egui::Rect::from_center_size(
            egui::pos2(rect.left() + indent + 7.0, mid),
            vec2(14.0, ROW),
        );
        let turn = ui
            .ctx()
            .animate_bool_with_time(response.id.with("open"), expanded, 0.15);
        let hit = ui.interact(arrow, ui.id().with(("arrow", id)), Sense::click());
        let color = if hit.hovered() { p.text } else { p.text_faint };
        icons::chevron(
            ui.painter(),
            egui::Rect::from_center_size(arrow.center(), vec2(10.0, 10.0)),
            color,
            turn,
        );
        if hit.clicked() {
            toggle(tab, id);
        }
    }
    let component = node.kind == "SYMBOL" || node.kind == "INSTANCE";
    let (icon_color, color) = if hidden {
        (p.text_faint, p.text_faint)
    } else if component {
        (p.component, p.component)
    } else if selected {
        (p.text_dim, p.text)
    } else {
        (
            p.text_faint,
            p.text_dim.lerp_to_gamma(p.text, 0.6 + hover * 0.4),
        )
    };
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + indent + 22.0, mid),
        vec2(13.0, 13.0),
    );
    icons::paint(ui.painter(), icons::for_kind(&node.kind), icon, icon_color);
    let font = if selected {
        theme::semibold(12.5)
    } else {
        FontId::proportional(12.5)
    };
    let left = rect.left() + indent + 36.0;
    let g = crate::chrome::elided(ui, &name, font, color, rect.right() - left - 6.0);
    ui.painter()
        .galley(egui::pos2(left, mid - g.size().y / 2.0), g, color);
    handle_row(tab, &response, id, has_children);
}

fn handle_row(tab: &mut Tab, response: &egui::Response, id: Guid, has_children: bool) {
    if response.clicked() {
        tab.select(Some(id));
    }
    if response.double_clicked() && has_children {
        toggle(tab, id);
    }
    if response.hovered() {
        tab.hover = Some(id);
    }
    let mut picked = None;
    copy_menu::on_context(response, tab, id, &mut picked);
    tab.pending_copy = picked.or(tab.pending_copy.take());
}

fn toggle(tab: &mut Tab, id: Guid) {
    if !tab.expanded.remove(&id) {
        tab.expanded.insert(id);
    }
}

/// Returns the Copy menu item picked this frame, if any.
pub fn properties(ui: &mut Ui, tab: &Tab) -> Option<Item> {
    let doc = &tab.doc;
    let Some(node) = tab.selection.and_then(|id| doc.node(id)) else {
        document_info(ui, tab);
        return None;
    };
    let d = &node.data;
    let mut picked: Option<Item> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let p = theme::of(ui);
            let header = ui.add(
                egui::Label::new(RichText::new(display_name(node)).font(theme::semibold(15.0)))
                    .wrap(),
            );
            copy_menu::on_context(&header, tab, node.id, &mut picked);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                icons::paint(ui.painter(), icons::for_kind(&node.kind), r, p.text_faint);
                ui.label(
                    RichText::new(format!(
                        "{}  ·  {}",
                        node.kind.to_lowercase().replace('_', " "),
                        node.id
                    ))
                    .color(p.text_faint)
                    .small(),
                );
            });
            ui.add_space(8.0);

            section(ui, "Layout", |ui| {
                let (w, h) = geom::size(d);
                let b = figstudio_render::node_bounds(doc, node.id);
                let (x, y) = b.map_or((0.0, 0.0), |b| (b.x0, b.y0));
                grid(
                    ui,
                    "layout",
                    &[("X", fmt(x)), ("Y", fmt(y)), ("W", fmt(w)), ("H", fmt(h))],
                );
                let tf = geom::affine(d.get("transform"));
                let c = tf.as_coeffs();
                let angle = (-c[1]).atan2(c[0]).to_degrees();
                if angle.abs() > 0.01 {
                    kv(ui, "Rotation", format!("{angle:.1}°"));
                }
                if let Some(r) = d.num("cornerRadius").filter(|r| *r > 0.0) {
                    kv(ui, "Corner radius", fmt(f64::from(r)));
                }
                if geom::clips_content(&node.kind, d) {
                    kv(ui, "Clip content", "yes".into());
                }
                if let Some(mode) = d.text("stackMode").filter(|m| *m != "NONE") {
                    let gap = d.num("stackSpacing").unwrap_or(0.0);
                    kv(
                        ui,
                        "Auto layout",
                        format!("{} · gap {}", mode.to_lowercase(), fmt(f64::from(gap))),
                    );
                }
            });

            section(ui, "Layer", |ui| {
                let opacity = d.num("opacity").unwrap_or(1.0);
                kv(ui, "Opacity", format!("{:.0}%", opacity * 100.0));
                if let Some(mode) = d
                    .text("blendMode")
                    .filter(|m| *m != "PASS_THROUGH" && *m != "NORMAL")
                {
                    kv(ui, "Blend", mode.to_lowercase());
                }
                if d.flag("visible") == Some(false) {
                    kv(ui, "Visible", "hidden".into());
                }
                if d.flag("mask") == Some(true) {
                    kv(ui, "Mask", "yes".into());
                }
            });

            paints(ui, "Fill", d.get("fillPaints"));
            if d.get("strokePaints").is_some_and(|p| !p.items().is_empty()) {
                paints(ui, "Stroke", d.get("strokePaints"));
                kv(
                    ui,
                    "Weight",
                    fmt(f64::from(d.num("strokeWeight").unwrap_or(0.0))),
                );
            }
            effects(ui, d.get("effects"));

            if node.kind == "TEXT" {
                section(ui, "Text", |ui| {
                    copy_menu::text_block(ui, tab, node, &mut picked);
                    if let Some(f) = d.path(&["fontName", "family"]).and_then(Val::as_str) {
                        let style = d
                            .path(&["fontName", "style"])
                            .and_then(Val::as_str)
                            .unwrap_or("");
                        kv(ui, "Font", format!("{f} {style}"));
                    }
                    if let Some(size) = d.num("fontSize") {
                        kv(ui, "Size", fmt(f64::from(size)));
                    }
                });
            }
            if node.kind == "INSTANCE" {
                section(ui, "Component", |ui| {
                    let sym = d.path(&["symbolData", "symbolID"]).and_then(Val::as_guid);
                    match sym.and_then(|s| doc.node(s)) {
                        Some(c) => kv(ui, "Main", display_name(c)),
                        None => kv(ui, "Main", "missing".into()),
                    }
                    let n = d
                        .path(&["symbolData", "symbolOverrides"])
                        .map_or(0, |o| o.items().len());
                    kv(ui, "Overrides", n.to_string());
                });
            }
            let kids = doc.descendant_count(node.id);
            if kids > 0 {
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!("{kids} nested layers"))
                        .color(theme::of(ui).text_faint)
                        .small(),
                );
            }
        });
    picked
}

fn document_info(ui: &mut Ui, tab: &Tab) {
    let p = theme::of(ui);
    let doc = &tab.doc;
    ui.add(egui::Label::new(RichText::new(tab.title()).font(theme::semibold(15.0))).wrap());
    ui.label(RichText::new("Document").color(p.text_faint).small());
    ui.add_space(8.0);
    section(ui, "Overview", |ui| {
        kv(ui, "Pages", doc.pages.len().to_string());
        kv(ui, "Layers", doc.nodes.len().to_string());
        kv(ui, "Images", doc.images.len().to_string());
        kv(
            ui,
            "Format",
            format!("{} v{}", doc.prelude, doc.format_version),
        );
        kv(
            ui,
            "Opened in",
            format!("{:.0} ms", tab.load_time.as_secs_f64() * 1000.0),
        );
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new("Click a layer on the canvas or in the Layers panel.")
            .color(p.text_faint)
            .small(),
    );
}

/// A titled group of rows on a rounded well, like a settings pane.
fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    let p = theme::of(ui);
    ui.add_space(2.0);
    ui.label(
        RichText::new(title)
            .font(theme::semibold(11.5))
            .color(p.text_faint),
    );
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(p.well)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            body(ui);
        });
    ui.add_space(8.0);
}

/// Label-value pairs, two per row.
fn grid(ui: &mut Ui, id: &str, items: &[(&str, String)]) {
    let p = theme::of(ui);
    egui::Grid::new(id)
        .num_columns(4)
        .spacing(vec2(8.0, 6.0))
        .show(ui, |ui| {
            for (i, (k, v)) in items.iter().enumerate() {
                ui.label(RichText::new(*k).color(p.text_faint));
                ui.add_sized(vec2(64.0, 18.0), egui::Label::new(v.as_str()));
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
}

fn kv(ui: &mut Ui, k: &str, v: String) {
    let p = theme::of(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).color(p.text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(v);
        });
    });
}

fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}

fn paints(ui: &mut Ui, title: &str, list: Option<&Val>) {
    let items = list.map(Val::items).unwrap_or_default();
    if items.is_empty() {
        return;
    }
    section(ui, title, |ui| {
        for p in items {
            let kind = p.text("type").unwrap_or("?");
            let visible = p.flag("visible") != Some(false);
            let opacity = p.num("opacity").unwrap_or(1.0);
            ui.horizontal(|ui| {
                let swatch = match kind {
                    "SOLID" => figstudio_render::paint::color(p.get("color"), 1.0)
                        .map(crate::canvas::to_color32),
                    _ => None,
                };
                let p = theme::of(ui);
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                let radius = CornerRadius::same(5);
                match swatch {
                    Some(c) => {
                        ui.painter().rect_filled(r, radius, c);
                    }
                    None => {
                        ui.painter().rect_filled(r, radius, Color32::from_gray(140));
                    }
                }
                ui.painter().rect_stroke(
                    r,
                    radius,
                    egui::Stroke::new(1.0, p.card_stroke),
                    egui::StrokeKind::Inside,
                );
                let label = match (kind, swatch) {
                    ("SOLID", Some(c)) => format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()),
                    _ => kind.to_lowercase().replace('_', " "),
                };
                let text = RichText::new(label);
                ui.label(if visible {
                    text
                } else {
                    text.color(p.text_faint).strikethrough()
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.0}%", opacity * 100.0)).color(p.text_dim));
                });
            });
        }
    });
}

fn effects(ui: &mut Ui, list: Option<&Val>) {
    let items = list.map(Val::items).unwrap_or_default();
    if items.is_empty() {
        return;
    }
    section(ui, "Effects", |ui| {
        for e in items {
            let kind = e
                .text("type")
                .unwrap_or("?")
                .to_lowercase()
                .replace('_', " ");
            let radius = e.num("radius").unwrap_or(0.0);
            let visible = e.flag("visible") != Some(false);
            let text = RichText::new(format!("{kind} · {}", fmt(f64::from(radius))));
            ui.label(if visible {
                text
            } else {
                text.color(theme::of(ui).text_faint).strikethrough()
            });
        }
    });
}
