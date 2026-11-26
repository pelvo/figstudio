//! The Design panel edits the selection: fields, Mixed values, paint and effect lists.
use egui::{Key, Modifiers};

use super::multi_select::layout;
use super::*;

/// Types `text` into the field labelled `label` and presses Enter.
fn type_into(h: &mut Harness<'_, App>, label: &str, text: &str) {
    reveal(h, label);
    h.get_by_label(label).click();
    h.run();
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
