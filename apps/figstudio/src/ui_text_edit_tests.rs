//! Editing text on the canvas in the real app: typing goes to the text (not to the tool and
//! delete shortcuts), and the Design panel styles a selected range.
use egui::{Event, Key, Modifiers};

use super::design::type_into;
use super::text_select::{double_click, key};
use super::*;

const TEXT: Guid = Guid(1, 42);

fn characters(h: &Harness<'_, App>) -> String {
    crate::text_select::characters(&h.state().tabs[0].doc.node(TEXT).unwrap().data).to_string()
}

fn edit_text() -> (Harness<'static, App>, Vec<String>) {
    let (mut h, mut c) = (harness_fixture(FIXTURE, 1.0 / 60.0), Vec::new());
    let at = centre_of(&h, TEXT);
    double_click(&mut h, &mut c, at);
    assert!(h.state().tabs[0].text_select.is_some(), "text mode");
    (h, c)
}

#[test]
fn typing_edits_the_text_and_backspace_never_deletes_the_layer() {
    let (mut h, mut c) = edit_text();
    key(&mut h, &mut c, Key::End, Modifiers::NONE);
    // "r" is the Rectangle tool's key; here it is a letter.
    h.event(Event::Key {
        key: Key::R,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::Text("r".into()));
    h.run_steps(2);
    assert_eq!(characters(&h), "ABCr");
    assert_eq!(
        h.state().tool,
        crate::canvas::Tool::Select,
        "no tool picked"
    );
    key(&mut h, &mut c, Key::Backspace, Modifiers::NONE);
    key(&mut h, &mut c, Key::Backspace, Modifiers::NONE);
    assert_eq!(characters(&h), "AB");
    assert!(
        h.state().tabs[0].doc.node(TEXT).is_some(),
        "the layer stays"
    );
    // ⌘Z still undoes (the whole typing run).
    key(&mut h, &mut c, Key::Z, Modifiers::COMMAND);
    assert_eq!(characters(&h), "ABC");
}

#[test]
fn the_panel_styles_only_the_selected_range() {
    let (mut h, _) = edit_text();
    h.state_mut().tabs[0].text_select = Some(crate::text_select::Selection {
        node: TEXT,
        anchor: 1,
        head: 3,
    });
    h.run_steps(2);
    let size = |h: &Harness<'_, App>| {
        h.state().tabs[0]
            .doc
            .node(TEXT)
            .unwrap()
            .data
            .num("fontSize")
    };
    let before = size(&h);
    type_into(&mut h, "Font size", "40");
    let data = h.state().tabs[0].doc.node(TEXT).unwrap().data.clone();
    assert_eq!(size(&h), before, "the layer's size stays");
    // "B" and "C" use a style run with the new size; "A" does not.
    let ids: Vec<u32> = data
        .path(&["textData", "characterStyleIDs"])
        .expect("a style run")
        .items()
        .iter()
        .map(|v| v.as_u32().unwrap_or(0))
        .collect();
    assert_eq!(ids.first().copied().unwrap_or(0), 0, "{ids:?}");
    let run = ids[1];
    assert_ne!(run, 0);
    assert_eq!(ids.get(2), Some(&run));
    let table = data
        .path(&["textData", "styleOverrideTable"])
        .unwrap()
        .items();
    let entry = table
        .iter()
        .find(|e| e.get("styleID").and_then(figstudio_fig::Val::as_u32) == Some(run))
        .unwrap();
    assert_eq!(entry.num("fontSize"), Some(40.0));
}
