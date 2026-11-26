//! The sidebar's Assets tab: local components (click to place one in the middle of the view,
//! or drag one onto the canvas), styles (click to apply to the selection) and variables (a
//! table of values per mode, editable in place).

use std::sync::Arc;

use egui::{Color32, Id, Pos2, RichText, Sense, Ui, vec2};
use figstudio_fig::edit::EditOp;
use figstudio_fig::edit::variables::{VarType, VarValue};
use figstudio_fig::tokens::{StyleKind, Tokens};
use figstudio_fig::{Guid, Val};

use crate::editor::apply_tab_ops;
use crate::tab::Tab;
use crate::theme;

/// What the Assets tab lists, built once per document revision.
#[derive(Clone, Default)]
pub(crate) struct Index {
    revision: u64,
    doc: u64,
    pub components: Vec<(Guid, String)>,
    styles: Vec<StyleRow>,
    collections: Vec<CollectionRow>,
}

#[derive(Clone)]
struct StyleRow {
    id: Guid,
    name: String,
    /// The slot it binds: `fill`, `text`, `effect` or `grid`.
    slot: &'static str,
    swatch: Option<Color32>,
}

#[derive(Clone)]
struct CollectionRow {
    id: Guid,
    name: String,
    modes: Vec<(Guid, String)>,
    variables: Vec<VariableRow>,
}

#[derive(Clone)]
struct VariableRow {
    id: Guid,
    name: String,
    kind: Option<VarType>,
    /// Per mode, in the collection's mode order.
    values: Vec<Option<Val>>,
}

/// The index for `tab`'s document, rebuilt when the document changes.
pub(crate) fn index(ctx: &egui::Context, tab: &Tab) -> Arc<Index> {
    let key = Id::new(("assets-index", tab.id));
    let (doc, revision) = (tab.doc.instance, tab.doc.revision);
    if let Some(i) = ctx.data(|d| d.get_temp::<Arc<Index>>(key))
        && i.revision == revision
        && i.doc == doc
    {
        return i;
    }
    let i = Arc::new(build(tab, doc, revision));
    ctx.data_mut(|d| d.insert_temp(key, i.clone()));
    i
}

fn build(tab: &Tab, doc_id: u64, revision: u64) -> Index {
    let doc = &tab.doc;
    let tokens = Tokens::new(doc);
    let mut styles: Vec<StyleRow> = tokens
        .styles()
        .filter_map(|s| {
            let slot = match s.kind {
                StyleKind::Fill | StyleKind::Stroke => "fill",
                StyleKind::Text => "text",
                StyleKind::Effect => "effect",
                StyleKind::Grid => "grid",
                StyleKind::Other(_) => return None,
            };
            let swatch = s
                .paints
                .as_ref()
                .and_then(|p| p.items().first().map(|p| p.clone().into_owned()))
                .filter(|p| p.text("type") == Some("SOLID"))
                .map(|p| {
                    let [r, g, b, a] = crate::editor::rgba_of(
                        p.get("color"),
                        p.num("opacity").map_or(1.0, f64::from),
                    );
                    Color32::from_rgba_unmultiplied(r, g, b, a)
                });
            Some(StyleRow {
                id: s.id,
                name: s.name.clone(),
                slot,
                swatch,
            })
        })
        .collect();
    styles.sort_by(|a, b| (a.slot, a.name.to_lowercase()).cmp(&(b.slot, b.name.to_lowercase())));
    let mut collections: Vec<CollectionRow> = tokens
        .collections()
        .map(|c| {
            let modes: Vec<(Guid, String)> =
                c.modes.iter().map(|m| (m.id, m.name.clone())).collect();
            let mut variables: Vec<VariableRow> = tokens
                .variables()
                .filter(|v| v.collection == Some(c.id))
                .map(|v| VariableRow {
                    id: v.id,
                    name: v.name.clone(),
                    kind: match v.resolved_type.as_deref() {
                        Some("COLOR") => Some(VarType::Color),
                        Some("FLOAT") => Some(VarType::Number),
                        Some("STRING") => Some(VarType::String),
                        Some("BOOLEAN") => Some(VarType::Bool),
                        _ => None,
                    },
                    values: modes
                        .iter()
                        .map(|(m, _)| {
                            v.values
                                .iter()
                                .find(|mv| mv.mode == Some(*m))
                                .and_then(|mv| mv.data.get("value").map(|x| x.clone().into_owned()))
                        })
                        .collect(),
                })
                .collect();
            variables.sort_by_key(|v| v.name.to_lowercase());
            CollectionRow {
                id: c.id,
                name: c.name.clone(),
                modes,
                variables,
            }
        })
        .collect();
    collections.sort_by_key(|c| c.name.to_lowercase());
    Index {
        revision,
        doc: doc_id,
        components: crate::editor::local_components(doc),
        styles,
        collections,
    }
}

/// The Assets tab.
pub fn assets(ui: &mut Ui, tab: &mut Tab, filter: &str) {
    let filter = filter.trim().to_lowercase();
    let index = index(ui.ctx(), tab);
    let shown = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
    egui::ScrollArea::vertical()
        .id_salt("assets")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::CollapsingHeader::new(RichText::new("Components").strong())
                .default_open(true)
                .show(ui, |ui| {
                    let mut any = false;
                    for (id, label) in index.components.iter().filter(|(_, l)| shown(l)) {
                        any = true;
                        component_row(ui, tab, *id, label);
                    }
                    if !any {
                        ui.label(RichText::new("No components").color(theme::of(ui).text_dim));
                    }
                });
            egui::CollapsingHeader::new(RichText::new("Styles").strong())
                .default_open(true)
                .show(ui, |ui| {
                    let mut any = false;
                    for s in index.styles.iter().filter(|s| shown(&s.name)) {
                        any = true;
                        style_row(ui, tab, s);
                    }
                    if !any {
                        ui.label(RichText::new("No styles").color(theme::of(ui).text_dim));
                    }
                });
            egui::CollapsingHeader::new(RichText::new("Variables").strong())
                .default_open(true)
                .show(ui, |ui| {
                    for c in &index.collections {
                        collection(ui, tab, c, &shown);
                    }
                    let r = ui.button("+ Collection");
                    crate::a11y::author(&r, "assets.add-collection");
                    if r.clicked() {
                        let n = index.collections.len() + 1;
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::CreateVariableCollection {
                                name: format!("Collection {n}"),
                                modes: Vec::new(),
                            }],
                            Some("Add collection"),
                            None,
                        );
                    }
                });
        });
}

/// Places an instance of `symbol` centred on page point `at`, on the current page, selected.
pub(crate) fn place(tab: &mut Tab, symbol: Guid, at: Pos2) {
    let (w, h) = tab.doc.node(symbol).map_or((0.0, 0.0), |n| {
        let (_, _, w, h) = crate::editor::local_xywh(&n.data);
        (w, h)
    });
    let page = tab.page;
    if let Ok(ids) = apply_tab_ops(
        tab,
        &[EditOp::InstantiateComponent {
            symbol,
            parent: page,
            x: (f64::from(at.x) - w / 2.0).round(),
            y: (f64::from(at.y) - h / 2.0).round(),
        }],
        Some("Place component"),
        None,
    ) && let Some(&id) = ids.first()
    {
        tab.select(Some(id));
    }
}

fn component_row(ui: &mut Ui, tab: &mut Tab, id: Guid, label: &str) {
    let p = theme::of(ui);
    let r = ui.add(
        egui::Label::new(RichText::new(format!("◇  {label}")).color(p.component))
            .sense(Sense::click_and_drag())
            .truncate(),
    );
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Place {label}"))
    });
    crate::a11y::author(&r, format!("assets.component.{id}"));
    let r = r.on_hover_text("Click to place in the middle of the view, or drag onto the canvas");
    let canvas = tab.canvas_rect;
    if r.clicked()
        && let Some(c) = canvas
    {
        let at = tab.view.to_page(c.min, c.center());
        place(tab, id, at);
    }
    if r.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        // The component's name follows the pointer while it is dragged.
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            Id::new("assets-drag"),
        ));
        let g = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), p.text);
        let rect = egui::Rect::from_min_size(pos + vec2(12.0, 8.0), g.size() + vec2(12.0, 6.0));
        painter.rect_filled(rect, 4.0, p.field);
        painter.galley(rect.min + vec2(6.0, 3.0), g, p.text);
    }
    if r.drag_stopped()
        && let (Some(c), Some(pos)) = (canvas, ui.ctx().pointer_latest_pos())
        && c.contains(pos)
    {
        let at = tab.view.to_page(c.min, pos);
        place(tab, id, at);
    }
}

fn style_row(ui: &mut Ui, tab: &mut Tab, s: &StyleRow) {
    let p = theme::of(ui);
    let r = ui
        .horizontal(|ui| {
            let (sw, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            match s.swatch {
                Some(c) => {
                    ui.painter().rect_filled(sw, 2.0, c);
                }
                None => {
                    let icon = match s.slot {
                        "text" => crate::icons::Icon::Text,
                        "grid" => crate::icons::Icon::Grid,
                        _ => crate::icons::Icon::Sparkle,
                    };
                    crate::icons::paint(ui.painter(), icon, sw, p.text_faint);
                }
            }
            ui.add(egui::Label::new(&s.name).sense(Sense::click()).truncate())
        })
        .inner;
    let what = if s.slot == "fill" {
        "Click: fill · Shift-click: stroke"
    } else {
        "Click to apply to the selection"
    };
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Apply {}", s.name))
    });
    crate::a11y::author(&r, format!("assets.style.{}", s.id));
    let r = r.on_hover_text(what);
    if r.clicked() {
        let slot = if s.slot == "fill" && ui.input(|i| i.modifiers.shift) {
            "stroke"
        } else {
            s.slot
        };
        let ops: Vec<EditOp> = tab
            .selected()
            .into_iter()
            .filter(|id| slot != "text" || tab.doc.node(*id).is_some_and(|n| n.kind == "TEXT"))
            .map(|target| EditOp::BindStyle {
                target,
                slot: slot.into(),
                style: Some(s.id),
            })
            .collect();
        if !ops.is_empty() {
            let _ = apply_tab_ops(tab, &ops, Some("Apply style"), None);
        }
    }
}

fn hex_with_alpha(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

/// One collection: its variables by mode, add a variable or a mode.
fn collection(ui: &mut Ui, tab: &mut Tab, c: &CollectionRow, shown: &dyn Fn(&str) -> bool) {
    egui::CollapsingHeader::new(&c.name)
        .id_salt(("assets-collection", c.id))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt(("assets-table", c.id))
                .show(ui, |ui| {
                    egui::Grid::new(("assets-grid", c.id))
                        .striped(true)
                        // A field never grows past its column, so the columns start wide enough.
                        .min_col_width(72.0)
                        .spacing(vec2(8.0, 4.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new("Name").color(theme::of(ui).text_dim));
                            for (m, name) in &c.modes {
                                let label = format!("{} mode name", c.name);
                                if let Some(n) = crate::editor::text_field(
                                    ui,
                                    &format!("{label} {m}"),
                                    name,
                                    70.0,
                                ) {
                                    let _ = apply_tab_ops(
                                        tab,
                                        &[EditOp::RenameVariableMode {
                                            collection: c.id,
                                            mode: *m,
                                            name: n,
                                        }],
                                        Some("Rename mode"),
                                        None,
                                    );
                                }
                            }
                            ui.end_row();
                            for v in c.variables.iter().filter(|v| shown(&v.name)) {
                                variable_row(ui, tab, c, v);
                                ui.end_row();
                            }
                        });
                });
            ui.horizontal_wrapped(|ui| {
                for (text, kind, value) in [
                    (
                        "+ Colour",
                        VarType::Color,
                        VarValue::Color("#000000".into()),
                    ),
                    ("+ Number", VarType::Number, VarValue::Number(0.0)),
                    ("+ Text", VarType::String, VarValue::String(String::new())),
                    ("+ Boolean", VarType::Bool, VarValue::Bool(false)),
                ] {
                    let r = ui.small_button(text);
                    crate::a11y::name_field(
                        &r,
                        &format!("Add {} variable to {}", &text[2..], c.name),
                    );
                    crate::a11y::author(
                        &r,
                        format!("assets.add-{}.{}", crate::a11y::slug(&text[2..]), c.id),
                    );
                    if r.clicked() {
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::CreateVariable {
                                collection: c.id,
                                name: format!("{} {}", &text[2..], c.variables.len() + 1)
                                    .to_lowercase(),
                                kind,
                                value,
                            }],
                            Some("Add variable"),
                            None,
                        );
                    }
                }
                let r = ui.small_button("+ Mode");
                crate::a11y::name_field(&r, &format!("Add mode to {}", c.name));
                crate::a11y::author(&r, format!("assets.add-mode.{}", c.id));
                if r.clicked() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::AddVariableMode {
                            collection: c.id,
                            name: format!("Mode {}", c.modes.len() + 1),
                        }],
                        Some("Add mode"),
                        None,
                    );
                }
            });
        });
}

fn variable_row(ui: &mut Ui, tab: &mut Tab, c: &CollectionRow, v: &VariableRow) {
    if let Some(n) = crate::editor::text_field(ui, &format!("{} name", v.name), &v.name, 100.0) {
        let _ = apply_tab_ops(
            tab,
            &[EditOp::RenameVariable {
                variable: v.id,
                name: n,
            }],
            Some("Rename variable"),
            None,
        );
    }
    for ((m, mode), value) in c.modes.iter().zip(&v.values) {
        let name = format!("{} in {mode}", v.name);
        let key = format!("var:{}:{m}", v.id);
        let set = |tab: &mut Tab, value: VarValue| {
            let _ = apply_tab_ops(
                tab,
                &[EditOp::SetVariableValue {
                    variable: v.id,
                    mode: *m,
                    value,
                }],
                Some("Set variable"),
                Some(&key),
            );
        };
        if let Some(alias) = value
            .as_ref()
            .and_then(|x| x.path(&["alias", "guid"]))
            .and_then(Val::as_guid)
        {
            let target = tab
                .doc
                .node(alias)
                .map_or("?".to_string(), |n| n.name.clone());
            ui.label(format!("→ {target}"));
            continue;
        }
        match v.kind {
            Some(VarType::Color) => {
                let [r, g, b, a] = crate::editor::rgba_of(
                    value.as_ref().and_then(|x| x.get("colorValue")),
                    value
                        .as_ref()
                        .and_then(|x| x.path(&["colorValue", "a"]))
                        .and_then(Val::as_f32)
                        .map_or(1.0, f64::from),
                );
                let mut col = Color32::from_rgba_unmultiplied(r, g, b, a);
                let resp = egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut col,
                    egui::color_picker::Alpha::OnlyBlend,
                );
                crate::a11y::name_field(&resp, &name);
                crate::a11y::author(&resp, format!("assets.var.{}.{m}", v.id));
                if resp.changed() {
                    set(tab, VarValue::Color(hex_with_alpha(col)));
                }
            }
            Some(VarType::Number) => {
                let cur = value
                    .as_ref()
                    .and_then(|x| x.get("floatValue"))
                    .and_then(Val::as_f32)
                    .map(f64::from);
                if let Some(n) = crate::editor::num_field(ui, "#", &name, cur, 0.1, 48.0) {
                    set(tab, VarValue::Number(n));
                }
            }
            Some(VarType::String) => {
                let cur = value
                    .as_ref()
                    .and_then(|x| x.get("textValue"))
                    .and_then(Val::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(t) = crate::editor::text_field(ui, &name, &cur, 80.0) {
                    set(tab, VarValue::String(t));
                }
            }
            Some(VarType::Bool) => {
                let mut b = value
                    .as_ref()
                    .and_then(|x| x.get("boolValue"))
                    .and_then(Val::as_bool)
                    .unwrap_or(false);
                let resp = ui.checkbox(&mut b, "");
                crate::a11y::name_field(&resp, &name);
                crate::a11y::author(&resp, format!("assets.var.{}.{m}", v.id));
                if resp.changed() {
                    set(tab, VarValue::Bool(b));
                }
            }
            None => {
                ui.label("—");
            }
        }
    }
    if crate::editor::button(ui, "−", &format!("Delete {}", v.name)).clicked() {
        let _ = apply_tab_ops(
            tab,
            &[EditOp::DeleteVariable { variable: v.id }],
            Some("Delete variable"),
            None,
        );
    }
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
