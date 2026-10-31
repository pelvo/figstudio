//! Code Connect in the Properties panel: the selected component's local mapping (desktop: add or
//! remove it) and, with nothing selected, the page's components without one.
//!
//! Mappings live in `FILE.fig.codeconnect.json` next to the file (`codeconnect_store`), the
//! same store the MCP tools read and write. The browser has no file next to the `.fig`, so it
//! lists components but cannot store mappings.

use std::cell::RefCell;

use egui::{RichText, Ui};
use figstudio_fig::{Document, Guid, Node};
use figstudio_mcp_core::codeconnect_context::component_id;
#[cfg(not(target_arch = "wasm32"))]
use figstudio_mcp_core::codeconnect_context::component_set;
use figstudio_mcp_core::codeconnect_suggest::{ALL_CONNECTED, suggestions};
use serde_json::Value;

use crate::tab::Tab;
use crate::theme;

#[cfg(not(target_arch = "wasm32"))]
use figstudio_mcp_core::codeconnect_store::{self, Mapping, Mappings};
#[cfg(target_arch = "wasm32")]
type Mappings = std::collections::BTreeMap<String, ()>;

#[cfg(not(target_arch = "wasm32"))]
/// Labels Figma accepts for a mapping, React first.
pub const LABELS: [&str; 15] = [
    "React",
    "Web Components",
    "Vue",
    "Svelte",
    "Storybook",
    "Javascript",
    "SwiftUI",
    "Swift UIKit",
    "Objective-C UIKit",
    "Compose",
    "Kotlin",
    "Java",
    "Android XML Layout",
    "Flutter",
    "Markdown",
];

/// Most suggestions the Overview lists.
const SHOWN: usize = 12;

#[derive(Debug, Default)]
struct State {
    doc: u64,
    /// The sidecar's modification time when it was read (`None`: no sidecar).
    modified: Option<std::time::SystemTime>,
    mappings: Mappings,
    /// The page the suggestions are for, and the `get_code_connect_suggestions` reply.
    suggestions: Option<(Guid, Value)>,
}

/// The mappings and page suggestions, read again when the sidecar or the document changes.
#[derive(Debug, Default)]
pub struct Cache(RefCell<Option<State>>);

impl Cache {
    #[cfg(not(target_arch = "wasm32"))]
    /// Drops what was read, after this app wrote the sidecar.
    pub fn clear(&self) {
        *self.0.borrow_mut() = None;
    }

    fn with<T>(&self, tab: &Tab, f: impl FnOnce(&mut State) -> T) -> T {
        let modified = sidecar_modified(tab);
        let mut slot = self.0.borrow_mut();
        let stale = slot
            .as_ref()
            .is_none_or(|s| s.doc != tab.doc.instance || s.modified != modified);
        if stale {
            *slot = Some(State {
                doc: tab.doc.instance,
                modified,
                mappings: read_mappings(tab),
                suggestions: None,
            });
        }
        f(slot.as_mut().expect("filled above"))
    }

    /// The `get_code_connect_suggestions` reply for the tab's page (cached).
    pub fn page_suggestions(&self, tab: &Tab) -> Value {
        self.with(tab, |state| {
            if let Some((page, value)) = &state.suggestions
                && *page == tab.page
            {
                return value.clone();
            }
            let mappings = &state.mappings;
            let value = suggestions(
                &tab.doc,
                &tab.page.to_string(),
                |id| mappings.contains_key(&id.to_string()),
                true,
            )
            .structured
            .unwrap_or(Value::Null);
            state.suggestions = Some((tab.page, value.clone()));
            value
        })
    }

    /// The mapping of component `id` (or of its set), with the key it is stored under.
    #[cfg(not(target_arch = "wasm32"))]
    fn mapping(&self, tab: &Tab, id: Guid) -> Option<(String, Mapping)> {
        let set = component_set(&tab.doc, id).map(|s| s.id);
        self.with(tab, |state| {
            [Some(id), set].into_iter().flatten().find_map(|k| {
                let key = k.to_string();
                state.mappings.get(&key).map(|m| (key, m.clone()))
            })
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sidecar_modified(tab: &Tab) -> Option<std::time::SystemTime> {
    codeconnect_store::modified(&tab.path)
}

#[cfg(target_arch = "wasm32")]
fn sidecar_modified(_tab: &Tab) -> Option<std::time::SystemTime> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn read_mappings(tab: &Tab) -> Mappings {
    // An unreadable sidecar shows as empty; the MCP tools report its error.
    codeconnect_store::mapped(&tab.path).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
fn read_mappings(_tab: &Tab) -> Mappings {
    Mappings::new()
}

/// The component a node stands for in Code Connect: itself, its set, or an instance's main
/// component. `None` for other nodes.
pub fn component_of(doc: &Document, node: &Node) -> Option<Guid> {
    component_id(doc, &node.id.to_string()).ok()
}

#[cfg(not(target_arch = "wasm32"))]
/// The form's fields for one component, kept in egui's memory between frames.
#[derive(Clone, Debug, Default)]
struct Form {
    name: String,
    source: String,
    label: usize,
    error: Option<String>,
}

/// The "Code Connect" section for a component, component set or instance.
pub fn component_section(ui: &mut Ui, tab: &Tab, node: &Node, component: Guid) {
    let p = theme::of(ui);
    crate::panels::section(ui, "Code Connect", |ui| {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (tab, node, component);
            ui.label(
                RichText::new(
                    "Mappings are kept in a file next to the .fig. Open it in the desktop app to connect components to code.",
                )
                .color(p.text_faint)
                .small(),
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        desktop_section(ui, tab, node, component, p.text_faint);
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn desktop_section(ui: &mut Ui, tab: &Tab, node: &Node, component: Guid, faint: egui::Color32) {
    if let Some((key, mapping)) = tab.code_connect.mapping(tab, component) {
        crate::panels::kv(ui, "Component", mapping.component_name.clone());
        crate::panels::kv(ui, "Source", mapping.source.clone());
        crate::panels::kv(ui, "Label", mapping.label.clone());
        if key != component.to_string() {
            ui.label(
                RichText::new("Mapped on its component set")
                    .color(faint)
                    .small(),
            );
        }
        let error_id = egui::Id::new(("code-connect-error", component));
        if let Some(error) = ui.data(|d| d.get_temp::<String>(error_id)) {
            ui.label(
                RichText::new(error)
                    .color(ui.visuals().error_fg_color)
                    .small(),
            );
        }
        if crate::panels::quiet_button(ui, None, "Remove mapping").clicked() {
            match codeconnect_store::set(&tab.doc, &tab.path, &key, None) {
                Ok(_) => tab.code_connect.clear(),
                Err(e) => show_error(ui, component, e),
            }
        }
        return;
    }
    let id = egui::Id::new(("code-connect-form", tab.id, component));
    let mut form: Form = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| Form {
        name: code_name(
            tab.doc
                .node(component_set(&tab.doc, component).map_or(component, |s| s.id))
                .map_or(node.name.as_str(), |n| n.name.as_str()),
        ),
        ..Form::default()
    });
    ui.label(RichText::new("Not connected").color(faint).small());
    field_row(ui, "Component", |ui, label| {
        field(ui, label, &mut form.name, "");
    });
    field_row(ui, "Source", |ui, label| {
        field(ui, label, &mut form.source, "src/components/Button.tsx");
    });
    field_row(ui, "Label", |ui, _| {
        label_picker(
            ui,
            egui::Id::new(("code-connect-labels", tab.id, component)),
            &mut form.label,
        );
    });
    if let Some(error) = &form.error {
        ui.label(
            RichText::new(error)
                .color(ui.visuals().error_fg_color)
                .small(),
        );
    }
    ui.add_space(2.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
    let tint = theme::space_tint(tab.colour(), theme::of(ui).dark);
    let connect = crate::chrome::accent_button(
        ui,
        rect,
        ("code-connect", tab.id, component),
        "Connect",
        "Connect",
        tint,
    );
    if connect.clicked() {
        let mapping = Mapping {
            component_name: form.name.trim().to_owned(),
            source: form.source.trim().to_owned(),
            label: LABELS[form.label.min(LABELS.len() - 1)].to_owned(),
            snippet: None,
            snippet_imports: Vec::new(),
        };
        let target = component_set(&tab.doc, component).map_or(component, |s| s.id);
        match codeconnect_store::set(&tab.doc, &tab.path, &target.to_string(), Some(mapping)) {
            Ok(_) => {
                tab.code_connect.clear();
                ui.data_mut(|d| d.remove::<Form>(id));
                return;
            }
            Err(e) => form.error = Some(e.to_owned()),
        }
    }
    ui.data_mut(|d| d.insert_temp(id, form));
}

/// Width of the form's label column.
#[cfg(not(target_arch = "wasm32"))]
const LABEL_WIDTH: f32 = 72.0;

/// Height of a form field, as the layer filter.
#[cfg(not(target_arch = "wasm32"))]
const FIELD: f32 = 26.0;

/// A row of the form: a faint label on the left, as in the other sections, then the control.
#[cfg(not(target_arch = "wasm32"))]
fn field_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, egui::Id)) {
    let faint = theme::of(ui).text_faint;
    ui.horizontal(|ui| {
        let label = ui
            .allocate_ui_with_layout(
                egui::vec2(LABEL_WIDTH, FIELD),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    // The column keeps its width whatever the label, so the fields line up.
                    ui.set_min_width(LABEL_WIDTH);
                    ui.label(RichText::new(label).color(faint))
                },
            )
            .inner;
        add(ui, label.id);
    });
}

/// A text field drawn like the layer filter: a rounded, tinted frame around a plain edit.
#[cfg(not(target_arch = "wasm32"))]
fn field(ui: &mut Ui, label: egui::Id, text: &mut String, hint: &str) {
    let p = theme::of(ui);
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.add_sized(
                egui::vec2(ui.available_width(), FIELD),
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .vertical_align(egui::Align::Center)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::symmetric(0, 0)),
            )
            .labelled_by(label);
        });
}

/// The label picker: a field-shaped button that opens the app's menu with Figma's labels.
#[cfg(not(target_arch = "wasm32"))]
fn label_picker(ui: &mut Ui, id: egui::Id, chosen: &mut usize) {
    let p = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), FIELD),
        egui::Sense::click(),
    );
    let current = LABELS[(*chosen).min(LABELS.len() - 1)];
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ComboBox,
            true,
            format!("Label: {current}"),
        )
    });
    let (hover, _) = crate::icons::feedback(ui, &response);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(8),
        p.field.lerp_to_gamma(p.hover, hover * 0.6),
    );
    painter.text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        current,
        egui::FontId::proportional(13.0),
        p.text,
    );
    crate::icons::chevron(
        painter,
        egui::Rect::from_center_size(
            rect.right_center() - egui::vec2(14.0, 0.0),
            egui::vec2(10.0, 10.0),
        ),
        p.text_faint,
        1.0,
    );
    egui::Popup::menu(&response)
        .style(theme::menu_style)
        .id(id)
        .show(|ui| {
            for (i, label) in LABELS.iter().enumerate() {
                // One of several, as the app's other menus mark it (View → CJK font).
                if ui.radio(i == *chosen, *label).clicked() {
                    *chosen = i;
                    ui.close();
                }
            }
        });
}

#[cfg(not(target_arch = "wasm32"))]
fn show_error(ui: &mut Ui, component: Guid, error: &str) {
    let id = egui::Id::new(("code-connect-error", component));
    ui.data_mut(|d| d.insert_temp(id, error.to_owned()));
}

#[cfg(not(target_arch = "wasm32"))]
/// A component name as a code identifier: `Button / Primary` → `ButtonPrimary`.
pub fn code_name(name: &str) -> String {
    let base = name.split('=').next().unwrap_or(name);
    let out: String = base
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map_or(String::new(), |c| {
                c.to_uppercase().chain(chars).collect::<String>()
            })
        })
        .collect();
    if out.is_empty() {
        "Component".into()
    } else {
        out
    }
}

/// The Overview's list of the page's components without a mapping. Returns the one clicked.
pub fn overview_section(ui: &mut Ui, tab: &Tab) -> Option<Guid> {
    let value = tab.code_connect.page_suggestions(tab);
    let components = value["components"].as_array()?;
    let message = value["message"].as_str();
    if components.is_empty() && message != Some(ALL_CONNECTED) {
        return None;
    }
    let mut clicked = None;
    crate::panels::section(ui, "Code Connect", |ui| {
        let faint = theme::of(ui).text_faint;
        if components.is_empty() {
            ui.label(
                RichText::new("Every component on this page is connected.")
                    .color(faint)
                    .small(),
            );
            return;
        }
        let total = value["total"].as_u64().unwrap_or(components.len() as u64);
        ui.label(
            RichText::new(if total == 1 {
                "1 component on this page is not connected".to_owned()
            } else {
                format!("{total} components on this page are not connected")
            })
            .color(faint)
            .small(),
        );
        for c in components.iter().take(SHOWN) {
            if suggestion_row(ui, c) {
                clicked = c["mainComponentNodeId"]
                    .as_str()
                    .and_then(|id| id.parse().ok());
            }
        }
        if total > SHOWN as u64 {
            ui.label(
                RichText::new(format!("and {} more", total - SHOWN as u64))
                    .color(faint)
                    .small(),
            );
        }
    });
    clicked
}

/// One suggestion, drawn like a Layers row: the component icon and name, the number of uses
/// on the right, a fill on hover. Returns whether it was clicked.
fn suggestion_row(ui: &mut Ui, c: &Value) -> bool {
    let p = theme::of(ui);
    let name = c["name"].as_str().unwrap_or("?");
    let uses = c["instanceCount"].as_u64().unwrap_or(0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), crate::panels::ROW),
        egui::Sense::click(),
    );
    let label = format!("{name}  ·  {uses}×");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let hover = crate::panels::hover_fade(
        ui,
        egui::Id::new("code-connect-hover"),
        response.id,
        response.hovered(),
    );
    crate::icons::row_fill(ui, rect, hover, false);
    // Figma's layer list shows component sets with the component icon too.
    let kind = "SYMBOL";
    let mid = rect.center().y;
    crate::icons::paint(
        ui.painter(),
        crate::icons::for_kind(kind),
        egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0, mid), egui::vec2(13.0, 13.0)),
        p.component,
    );
    let count = ui.painter().layout_no_wrap(
        format!("{uses}×"),
        egui::FontId::proportional(12.0),
        p.text_faint,
    );
    let right = rect.right() - 8.0;
    ui.painter().galley(
        egui::pos2(right - count.size().x, mid - count.size().y / 2.0),
        count.clone(),
        p.text_faint,
    );
    let left = rect.left() + 26.0;
    let text = p.text_dim.lerp_to_gamma(p.text, 0.6 + hover * 0.4);
    let g = crate::chrome::elided(
        ui,
        name,
        egui::FontId::proportional(12.5),
        text,
        right - count.size().x - 8.0 - left,
    );
    ui.painter()
        .galley(egui::pos2(left, mid - g.size().y / 2.0), g, text);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_names_become_code_identifiers() {
        assert_eq!(code_name("Button / Primary"), "ButtonPrimary");
        assert_eq!(code_name("_Input.Container"), "InputContainer");
        assert_eq!(code_name("size=small, state=hover"), "Size");
        assert_eq!(code_name("---"), "Component");
    }
}
