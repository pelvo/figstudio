//! Side panels: Pages, Layers tree and Properties.

use egui::{Color32, RichText, Sense, Ui, vec2};
use figstudio_fig::{Document, Guid, Node, Val};
use figstudio_render::geom;

use crate::tab::Tab;
use crate::theme;

/// Short glyph for a node type, shown before its name.
fn type_icon(kind: &str) -> &'static str {
    match kind {
        "FRAME" => "#",
        "SECTION" => "§",
        "GROUP" => "▣",
        "SYMBOL" => "❖",
        "INSTANCE" => "◇",
        "TEXT" => "T",
        "VECTOR" | "BOOLEAN_OPERATION" | "STAR" | "REGULAR_POLYGON" | "LINE" => "✎",
        "ELLIPSE" => "○",
        "RECTANGLE" | "ROUNDED_RECTANGLE" => "□",
        "SLICE" => "⊞",
        _ => "·",
    }
}

fn display_name(node: &Node) -> String {
    let name = node.data.text("name").unwrap_or("");
    if name.is_empty() {
        node.kind.to_lowercase()
    } else {
        name.to_string()
    }
}

pub fn pages(ui: &mut Ui, tab: &mut Tab) {
    let doc = tab.doc.clone();
    for &page in &doc.pages {
        let name = doc.node(page).map(display_name).unwrap_or_default();
        let selected = tab.page == page;
        if ui.selectable_label(selected, name).clicked() {
            tab.set_page(page);
        }
    }
    if !doc.internal_pages.is_empty() {
        ui.label(
            RichText::new(format!(
                "{} internal page(s) hidden",
                doc.internal_pages.len()
            ))
            .color(theme::TEXT_FAINT)
            .small(),
        );
    }
}

pub fn layers(ui: &mut Ui, tab: &mut Tab, filter: &str) {
    let doc = tab.doc.clone();
    let filter = filter.trim().to_lowercase();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
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
                    row(ui, tab, &doc, id, depth);
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
                    ui.label(RichText::new("No matching layers").color(theme::TEXT_FAINT));
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
            flat_row(ui, tab, node);
        }
        stack.extend(doc.children(child).iter().rev().copied());
    }
}

fn flat_row(ui: &mut Ui, tab: &mut Tab, node: &Node) {
    let selected = tab.selection == Some(node.id);
    let text = format!("{} {}", type_icon(&node.kind), display_name(node));
    let r = ui.add(egui::Button::selectable(selected, text).frame_when_inactive(false));
    if r.clicked() {
        tab.select(Some(node.id));
    }
    if r.hovered() {
        tab.hover = Some(node.id);
    }
}

fn row(ui: &mut Ui, tab: &mut Tab, doc: &Document, id: Guid, depth: usize) {
    let Some(node) = doc.node(id) else { return };
    let children = doc.children(id);
    let has_children = !children.is_empty();
    let expanded = tab.expanded.contains(&id);
    let selected = tab.selection == Some(id);
    let hidden = node.data.flag("visible") == Some(false);

    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width().max(160.0), height),
        Sense::click(),
    );
    if tab.reveal == Some(id) {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, theme::ROW_SELECTED);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, theme::HOVER);
    }
    let indent = 8.0 + depth as f32 * 14.0;
    let mid = rect.center().y;
    if has_children {
        let arrow = if expanded { "⏷" } else { "⏵" };
        let arrow_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + indent + 5.0, mid),
            vec2(14.0, height),
        );
        painter.text(
            arrow_rect.center(),
            egui::Align2::CENTER_CENTER,
            arrow,
            egui::FontId::proportional(10.0),
            theme::TEXT_DIM,
        );
        let arrow_hit = ui.interact(arrow_rect, ui.id().with(("arrow", id)), Sense::click());
        if arrow_hit.clicked() {
            toggle(tab, id);
        }
    }
    let color = if hidden {
        theme::TEXT_FAINT
    } else if node.kind == "SYMBOL" || node.kind == "INSTANCE" {
        Color32::from_rgb(170, 140, 255)
    } else {
        theme::TEXT
    };
    painter.text(
        egui::pos2(rect.left() + indent + 14.0, mid),
        egui::Align2::LEFT_CENTER,
        type_icon(&node.kind),
        egui::FontId::proportional(12.0),
        theme::TEXT_DIM,
    );
    painter.text(
        egui::pos2(rect.left() + indent + 30.0, mid),
        egui::Align2::LEFT_CENTER,
        display_name(node),
        egui::FontId::proportional(13.0),
        color,
    );
    if response.clicked() {
        tab.select(Some(id));
    }
    if response.double_clicked() && has_children {
        toggle(tab, id);
    }
    if response.hovered() {
        tab.hover = Some(id);
    }
    if expanded {
        for &child in children.iter().rev() {
            row(ui, tab, doc, child, depth + 1);
        }
    }
}

fn toggle(tab: &mut Tab, id: Guid) {
    if !tab.expanded.remove(&id) {
        tab.expanded.insert(id);
    }
}

pub fn properties(ui: &mut Ui, tab: &Tab) {
    let doc = &tab.doc;
    let Some(node) = tab.selection.and_then(|id| doc.node(id)) else {
        document_info(ui, tab);
        return;
    };
    let d = &node.data;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(RichText::new(display_name(node)).strong().size(14.0));
            ui.label(
                RichText::new(format!("{}  ·  {}", node.kind, node.id))
                    .color(theme::TEXT_FAINT)
                    .small(),
            );
            ui.add_space(6.0);

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
                    if let Some(s) = d.path(&["textData", "characters"]).and_then(Val::as_str) {
                        let preview: String = s.chars().take(400).collect();
                        ui.label(RichText::new(preview).color(theme::TEXT_DIM));
                    }
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
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{kids} nested layers"))
                        .color(theme::TEXT_FAINT)
                        .small(),
                );
            }
        });
}

fn document_info(ui: &mut Ui, tab: &Tab) {
    let doc = &tab.doc;
    ui.label(RichText::new(tab.title()).strong().size(14.0));
    ui.add_space(6.0);
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
    ui.add_space(8.0);
    ui.label(
        RichText::new("Click a layer on the canvas or in the Layers panel.")
            .color(theme::TEXT_FAINT)
            .small(),
    );
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).color(theme::TEXT_DIM).small().strong());
    ui.separator();
    body(ui);
    ui.add_space(4.0);
}

fn grid(ui: &mut Ui, id: &str, items: &[(&str, String)]) {
    egui::Grid::new(id)
        .num_columns(4)
        .spacing(vec2(8.0, 4.0))
        .show(ui, |ui| {
            for (i, (k, v)) in items.iter().enumerate() {
                ui.label(RichText::new(*k).color(theme::TEXT_FAINT));
                ui.label(v);
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
}

fn kv(ui: &mut Ui, k: &str, v: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).color(theme::TEXT_FAINT));
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
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                match swatch {
                    Some(c) => {
                        ui.painter().rect_filled(r, 2.0, c);
                    }
                    None => {
                        ui.painter().rect_filled(r, 2.0, Color32::from_gray(90));
                    }
                }
                ui.painter().rect_stroke(
                    r,
                    2.0,
                    egui::Stroke::new(1.0, theme::SEPARATOR),
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
                    text.color(theme::TEXT_FAINT).strikethrough()
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.0}%", opacity * 100.0)).color(theme::TEXT_DIM),
                    );
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
                text.color(theme::TEXT_FAINT).strikethrough()
            });
        }
    });
}
