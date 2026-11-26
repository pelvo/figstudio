//! The Design panel edits the selection: fields, Mixed values, paint and effect lists.
use egui::{Key, Modifiers};

use super::multi_select::layout;
use super::*;

/// Types `text` into the field labelled `label` and presses Enter.
pub(super) fn type_into(h: &mut Harness<'_, App>, label: &str, text: &str) {
    reveal(h, label);
    h.get_by_label(label).click();
    // Fixed steps: the render workers keep the frame repainting.
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label(label).type_text(text);
    h.key_press(Key::Enter);
    h.run_steps(3);
}

fn data(h: &Harness<'_, App>, id: Guid) -> figstudio_fig::Val {
    h.state().tabs[0]
        .doc
        .node(id)
        .unwrap()
        .data
        .clone()
        .into_owned()
}

fn num(h: &Harness<'_, App>, id: Guid, path: &[&str]) -> f32 {
    data(h, id)
        .path(path)
        .and_then(figstudio_fig::Val::as_f32)
        .unwrap_or(f32::NAN)
}

#[test]
fn a_field_shows_mixed_and_sets_every_selected_layer_in_one_step() {
    let mut h = harness();
    let (a, b, _) = layout(&mut h);
    h.state_mut().tabs[0].select_all_of(&[a, b]);
    h.run_steps(3);
    // The rectangles sit at x 0 and 100: X is Mixed; both are 40 high, so H is shared.
    assert_eq!(h.get_by_label("X position").value().as_deref(), Some(""));
    assert_eq!(h.get_by_label("Height").value().as_deref(), Some("40"));
    let depth = h.state().tabs[0].history.undo_len();
    type_into(&mut h, "X position", "25");
    assert_eq!(
        (
            num(&h, a, &["transform", "m02"]),
            num(&h, b, &["transform", "m02"])
        ),
        (25.0, 25.0)
    );
    assert_eq!(h.state().tabs[0].history.undo_len(), depth + 1);
    // An expression applies to the shared value.
    type_into(&mut h, "Height", "*2");
    assert_eq!(
        (num(&h, a, &["size", "y"]), num(&h, b, &["size", "y"])),
        (80.0, 80.0)
    );
}

#[test]
fn opacity_aspect_lock_and_corners_edit_the_layer() {
    let mut h = harness();
    let (a, _, _) = layout(&mut h);
    h.state_mut().tabs[0].select(Some(a));
    h.run_steps(3);
    type_into(&mut h, "Opacity", "50");
    assert!((num(&h, a, &["opacity"]) - 0.5).abs() < 1e-6);

    h.get_by_label("Lock aspect ratio").click();
    h.run_steps(3);
    assert_eq!(data(&h, a).flag("proportionsConstrained"), Some(true));
    // 60 × 40 with the lock: width 120 doubles the height too.
    type_into(&mut h, "Width", "120");
    assert_eq!(
        (num(&h, a, &["size", "x"]), num(&h, a, &["size", "y"])),
        (120.0, 80.0)
    );

    type_into(&mut h, "Corner radius", "8");
    assert_eq!(num(&h, a, &["cornerRadius"]), 8.0);
}

#[test]
fn fills_strokes_and_effects_are_added_edited_and_removed() {
    let mut h = harness();
    let (a, _, _) = layout(&mut h);
    h.state_mut().tabs[0].select(Some(a));
    h.run_steps(3);
    let fills = |h: &Harness<'_, App>| data(h, a).get("fillPaints").map_or(0, |p| p.items().len());
    assert_eq!(fills(&h), 1);
    click_label(&mut h, &mut Vec::new(), "Add fill");
    assert_eq!(fills(&h), 2, "a second fill is added on top");
    // Listed top paint first, as in Figma (the file stores it last).
    let y = |h: &Harness<'_, App>, l: &str| h.get_by_label(l).rect().top();
    assert!(y(&h, "Fill colour hex") < y(&h, "Fill 2 colour hex"));
    // The top fill is listed first, labelled "Fill"; the one below it is "Fill 2".
    type_into(&mut h, "Fill colour hex", "#00FF00");
    let d = data(&h, a);
    let top = &d.get("fillPaints").unwrap().items()[1];
    assert_eq!(
        top.path(&["color", "g"])
            .and_then(figstudio_fig::Val::as_f32),
        Some(1.0)
    );
    click_label(&mut h, &mut Vec::new(), "Remove fill 2");
    assert_eq!(fills(&h), 1);

    click_label(&mut h, &mut Vec::new(), "Add stroke");
    assert_eq!(data(&h, a).get("strokePaints").unwrap().items().len(), 1);
    type_into(&mut h, "Stroke weight", "3");
    assert_eq!(num(&h, a, &["strokeWeight"]), 3.0);

    click_label(&mut h, &mut Vec::new(), "Add effect");
    assert_eq!(data(&h, a).get("effects").unwrap().items().len(), 1);
    type_into(&mut h, "Drop shadow 1 blur", "12");
    assert_eq!(
        data(&h, a).get("effects").unwrap().items()[0].num("radius"),
        Some(12.0)
    );
    click_label(&mut h, &mut Vec::new(), "Remove drop shadow 1");
    assert!(data(&h, a).get("effects").unwrap().items().is_empty());
}

#[test]
fn rename_auto_layout_and_child_options_edit_the_layers() {
    let mut h = harness();
    let (_, _, frame) = layout(&mut h);
    h.state_mut().tabs[0].select(Some(frame));
    h.run_steps(3);
    type_into(&mut h, "Layer name", "Card");
    assert_eq!(h.state().tabs[0].doc.node(frame).unwrap().name, "Card");

    click_label(&mut h, &mut Vec::new(), "Add auto layout");
    assert_eq!(data(&h, frame).text("stackMode"), Some("VERTICAL"));
    click_label(&mut h, &mut Vec::new(), "Horizontal layout");
    assert_eq!(data(&h, frame).text("stackMode"), Some("HORIZONTAL"));
    type_into(&mut h, "Gap", "16");
    assert_eq!(num(&h, frame, &["stackSpacing"]), 16.0);
    type_into(&mut h, "Left padding", "24");
    assert_eq!(num(&h, frame, &["stackHorizontalPadding"]), 24.0);
    // The other sides keep their padding.
    assert_eq!(num(&h, frame, &["stackPaddingRight"]), 10.0);

    // The frame's child now sits in auto layout: Fill container sets its grow.
    let child = h.state().tabs[0].doc.children(frame)[0];
    h.state_mut().tabs[0].select(Some(child));
    h.run_steps(3);
    click_label(&mut h, &mut Vec::new(), "Fill container");
    assert_eq!(num(&h, child, &["stackChildPrimaryGrow"]), 1.0);

    // Back on the frame, removing auto layout makes the child's section Constraints.
    h.state_mut().tabs[0].select(Some(frame));
    h.run_steps(3);
    click_label(&mut h, &mut Vec::new(), "Remove auto layout");
    assert_eq!(data(&h, frame).text("stackMode"), Some("NONE"));
    h.state_mut().tabs[0].select(Some(child));
    h.run_steps(3);
    assert!(h.query_by_label("Horizontal constraint").is_some());
    assert!(h.query_by_label("Fill container").is_none());
}

#[test]
fn text_settings_and_export_edit_the_layer() {
    let mut h = harness();
    let (a, _, _) = layout(&mut h);
    let page = h.state().tabs[0].page;
    let text = crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::CreateText {
            id: None,
            parent: page,
            x: 0.0,
            y: 100.0,
            characters: "Hello".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 12.0,
            fill_hex: "#000000".into(),
        }],
        None,
        None,
    )
    .unwrap()[0];
    h.state_mut().tabs[0].select(Some(text));
    h.run_steps(3);
    type_into(&mut h, "Font size", "24");
    assert_eq!(num(&h, text, &["fontSize"]), 24.0);
    type_into(&mut h, "Line height", "30");
    assert_eq!(num(&h, text, &["lineHeight", "value"]), 30.0);
    click_label(&mut h, &mut Vec::new(), "Align text center");
    assert_eq!(data(&h, text).text("textAlignHorizontal"), Some("CENTER"));
    type_into(&mut h, "Font style", "Bold");
    assert_eq!(
        data(&h, text)
            .path(&["fontName", "style"])
            .and_then(figstudio_fig::Val::as_str),
        Some("Bold")
    );

    // Export: a setting is added at 1x PNG; the button asks the app to export at that scale.
    h.state_mut().tabs[0].select(Some(a));
    h.run_steps(3);
    click_label(&mut h, &mut Vec::new(), "Add export setting");
    let settings = data(&h, a).get("exportSettings").unwrap().items().len();
    assert_eq!(settings, 1);
    let name = h.state().tabs[0].doc.node(a).unwrap().name.clone();
    // UI tests export into the temporary folder (`App::save_export`).
    let file = std::env::temp_dir().join(format!(
        "figstudio-ui-export-{}-{name}.png",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&file);
    reveal(&mut h, "Export layer");
    h.get_by_label("Export layer").click();
    let mut written = false;
    for _ in 0..200 {
        h.step();
        if file.exists() {
            written = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&file);
    assert!(written, "the export button exported the layer");
}
