//! Screen-reader details egui leaves to the app: names for widgets it builds without one (text
//! fields, the panels' resize handles) and pointer-only areas kept out of the AccessKit tree.
//!
//! Each call edits the node egui made for `id` this frame; with no screen reader attached
//! (AccessKit off) egui keeps no tree and the calls do nothing. `ui_a11y_tests.rs` checks that
//! every control has a name.

use egui::accesskit::Role;
use egui::{Context, Id, Response, Ui};

use crate::tab::Tab;

/// Leaves `response` (a window drag strip, a click catcher behind a pill, a modal backdrop)
/// out of what screen readers read: it only takes the pointer.
pub fn hide(response: &Response) {
    response
        .ctx
        .accesskit_node_builder(response.id, |n| n.set_hidden());
}

/// Makes the layer of `ui` (a modal's backdrop) modal: Tab, screen readers' focus and the
/// pointer stay on it and the layers above it (the dialog), not the app behind. Call it every
/// frame the modal is open.
pub fn modal(ui: &Ui) {
    let layer = ui.layer_id();
    ui.ctx().memory_mut(|m| m.set_modal_layer(layer));
}

/// What [`restore_focus`] remembers about one modal.
#[derive(Clone, Copy, Debug, Default)]
struct ModalFocus {
    open: bool,
    /// What had focus when it opened; after it closed, still to be given back.
    saved: Option<Id>,
    /// Frames left to give it back in (the opener may be gone for good).
    left: u8,
}

fn modal_key(modal: &str) -> Id {
    Id::new(("modal-focus", modal))
}

/// Remembers what has keyboard focus when the modal `modal` opens, so [`give_focus_back`]
/// can return it there once it closes (WCAG 2.4.3): Escape out of Settings lands on the
/// control that opened it, not nowhere. Call it every frame with whether the modal is open,
/// before the modal moves focus.
pub fn restore_focus(ctx: &Context, modal: &str, open: bool) {
    let key = modal_key(modal);
    let mut state = ctx
        .data(|d| d.get_temp::<ModalFocus>(key))
        .unwrap_or_default();
    if open && !state.open {
        state = ModalFocus {
            open: true,
            saved: ctx.memory(|m| m.focused()),
            left: 0,
        };
    } else if !open && state.open {
        state.open = false;
        state.left = 60;
    }
    ctx.data_mut(|d| d.insert_temp(key, state));
}

/// After the frame is drawn: gives focus back to what `modal` saved, once it is drawn again
/// and nothing outside a closing modal (a foreground layer) took focus meanwhile. Only a
/// widget drawn this frame gets it: focus on a widget with no node would break the
/// accessibility tree (AccessKit requires the focused node to exist).
pub fn give_focus_back(ctx: &Context, modal: &str) {
    let key = modal_key(modal);
    let Some(mut state) = ctx.data(|d| d.get_temp::<ModalFocus>(key)) else {
        return;
    };
    let Some(saved) = state.saved.filter(|_| !state.open && state.left > 0) else {
        return;
    };
    let drawn = |id: Id| ctx.viewport(|v| v.this_pass.widgets.get(id).is_some());
    let taken = ctx.memory(|m| m.focused()).is_some_and(|f| {
        drawn(f)
            && ctx
                .read_response(f)
                .is_some_and(|r| r.layer_id.order != egui::Order::Foreground)
    });
    if taken {
        state.saved = None;
    } else if drawn(saved) {
        ctx.memory_mut(|m| m.request_focus(saved));
        state.saved = None;
    } else {
        state.left -= 1;
    }
    ctx.data_mut(|d| d.insert_temp(key, state));
}

/// Leaves out the title bar of the `egui::Window` whose id is `window` (shown this frame):
/// a drag strip without a name. The window itself is named after its title.
pub fn hide_window_title_bar(ctx: &Context, window: Id) {
    ctx.accesskit_node_builder(window.with("__title_click"), |n| n.set_hidden());
}

/// Names a menu egui drew (`popup` is the response `Popup::show` returned): egui gives a
/// popup's node no role or name, and the items sit inside it.
pub fn name_menu(popup: &Response, label: &str) {
    popup.ctx.accesskit_node_builder(popup.id, |n| {
        n.set_role(Role::Menu);
        n.set_label(label);
    });
}

/// `Ui::menu_button` inside a menu, with the submenu it opens named `label`.
pub fn submenu<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let (_, popup) = egui::containers::menu::SubMenuButton::new(label).ui(ui, add);
    popup.map(|popup| {
        name_menu(&popup.response, label);
        popup.inner
    })
}

/// Names a text field: its placeholder disappears as soon as the user types, so a screen
/// reader needs a label of its own.
pub fn name_field(response: &Response, label: &str) {
    response
        .ctx
        .accesskit_node_builder(response.id, |n| n.set_label(label));
}

/// Names the resize handle of the side panel `panel` (the id given to `egui::Panel`), and
/// rings it while it has focus (egui's handle takes Tab focus but shows none). Call it only
/// when the panel drew its handle this frame: a node egui did not make would be added with
/// no place on screen.
pub fn panel_handle(ctx: &Context, panel: &str, label: &str) {
    let id = Id::new(panel).with("__resize");
    ctx.accesskit_node_builder(id, |n| {
        n.set_role(Role::Splitter);
        n.set_label(label);
    });
    if ctx.memory(|m| m.has_focus(id))
        && let Some(handle) = ctx.read_response(id)
    {
        let layer = egui::LayerId::new(egui::Order::Foreground, id.with("focus"));
        ctx.layer_painter(layer).rect_stroke(
            handle.rect,
            egui::CornerRadius::same(3),
            egui::Stroke::new(2.0, crate::theme::palette(ctx).focus),
            egui::StrokeKind::Inside,
        );
    }
}

/// Makes `response` a tab (the Settings sections): screen readers hear "tab", and which is
/// selected, instead of a toggle button.
pub fn tab(response: &Response, selected: bool) {
    response.ctx.accesskit_node_builder(response.id, |n| {
        n.set_role(Role::Tab);
        n.clear_toggled();
        n.set_selected(selected);
    });
}

/// Marks `response` (a layer row with children) expanded or collapsed.
pub fn set_expanded(response: &Response, expanded: bool) {
    response
        .ctx
        .accesskit_node_builder(response.id, |n| n.set_expanded(expanded));
}

/// Names the canvas after its page, and describes the zoom and the selected layer, so a
/// screen reader says where the user is and what a click picked.
pub fn describe_canvas(response: &Response, tab: &Tab) {
    response.ctx.accesskit_node_builder(response.id, |n| {
        let page = tab
            .doc
            .node(tab.page)
            .map(crate::panels::display_name)
            .unwrap_or_default();
        let selected = tab
            .selection
            .and_then(|id| tab.doc.node(id))
            .map_or_else(|| "nothing".to_owned(), crate::panels::display_name);
        let zoom = (tab.view.zoom * 100.0).round();
        // A group, not Role::Canvas: macOS reads a canvas as an image, and an image's
        // children (the layers, `canvas_a11y.rs`) are not looked into.
        n.set_role(Role::Group);
        n.set_role_description("canvas");
        n.set_author_id("canvas");
        n.set_label(format!("Canvas, page {page}"));
        n.set_description(format!("Zoom {zoom}%. Selected: {selected}."));
    });
}

/// Reads a Properties row as one item, "Width: 120", instead of a key and a value that a screen
/// reader or agent would have to pair up by position: the key's node is hidden and the
/// value's says both.
pub fn key_value(key: &Response, value: &Response, k: &str, v: &str) {
    hide(key);
    value.ctx.accesskit_node_builder(value.id, |n| {
        n.set_value(format!("{k}: {v}"));
    });
}
