//! Typing, deleting and moving the caret in a text layer, one undo step per typing run.

use std::sync::Arc;
use std::time::Duration;

use egui::{Event, Key, Modifiers};
use figstudio_fig::edit::EditOp;
use figstudio_fig::{Document, Guid};

use super::*;

/// A tab on `structure.fig` with a new text layer `characters`, being edited with the caret
/// at the end.
fn editing_tab(characters: &str) -> (Tab, Guid) {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Document::open(path).unwrap();
    let mut tab = Tab::new(1, "t.fig".into(), Arc::new(doc), None, Duration::ZERO);
    let page = tab.page;
    let id = crate::editor::apply_tab_ops(
        &mut tab,
        &[EditOp::CreateText {
            id: None,
            parent: page,
            x: 0.0,
            y: 0.0,
            characters: characters.into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 20.0,
            fill_hex: "#000000".into(),
        }],
        None,
        None,
    )
    .unwrap()[0];
    tab.select(Some(id));
    let end = characters.len();
    tab.text_select = Some(Selection {
        node: id,
        anchor: end,
        head: end,
    });
    (tab, id)
}

fn text(tab: &Tab, id: Guid) -> String {
    text_select::characters(&tab.doc.node(id).unwrap().data).to_string()
}

fn key(tab: &mut Tab, key: Key, modifiers: Modifiers) {
    apply(
        &egui::Context::default(),
        tab,
        &Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
    );
}

fn typed(tab: &mut Tab, t: &str) {
    apply(&egui::Context::default(), tab, &Event::Text(t.into()));
}

fn caret(tab: &Tab) -> (usize, usize) {
    let s = tab.text_select.unwrap();
    (s.anchor, s.head)
}

#[test]
fn typing_inserts_at_the_caret_and_replaces_a_selection_in_one_undo_step() {
    let (mut tab, id) = editing_tab("Hello");
    typed(&mut tab, ",");
    typed(&mut tab, " you");
    assert_eq!(text(&tab, id), "Hello, you");
    assert_eq!(caret(&tab), (10, 10));
    // Select "Hello" and type over it.
    tab.text_select = Some(Selection {
        node: id,
        anchor: 0,
        head: 5,
    });
    typed(&mut tab, "Bye");
    assert_eq!(text(&tab, id), "Bye, you");
    // The whole run is one step.
    crate::editor::undo_tab(&mut tab).unwrap();
    assert_eq!(text(&tab, id), "Hello");
}

#[test]
fn backspace_and_delete_remove_whole_characters_beyond_one_utf16_unit() {
    let (mut tab, id) = editing_tab("a😀é");
    key(&mut tab, Key::Backspace, Modifiers::NONE);
    assert_eq!(text(&tab, id), "a😀");
    key(&mut tab, Key::Backspace, Modifiers::NONE);
    assert_eq!(text(&tab, id), "a");
    key(&mut tab, Key::ArrowLeft, Modifiers::NONE);
    key(&mut tab, Key::Delete, Modifiers::NONE);
    assert_eq!(text(&tab, id), "");
    // Nothing left: both keys do nothing, and nothing breaks.
    key(&mut tab, Key::Backspace, Modifiers::NONE);
    key(&mut tab, Key::Delete, Modifiers::NONE);
    assert_eq!(text(&tab, id), "");
}

#[test]
fn arrows_move_shift_extends_and_cmd_a_selects_all() {
    let (mut tab, _) = editing_tab("ab😀");
    key(&mut tab, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(caret(&tab), (2, 2), "one character back, over the emoji");
    key(&mut tab, Key::ArrowLeft, Modifiers::SHIFT);
    assert_eq!(caret(&tab), (2, 1));
    // Left on a range collapses it to its start.
    key(&mut tab, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(caret(&tab), (1, 1));
    key(&mut tab, Key::A, Modifiers::COMMAND);
    assert_eq!(caret(&tab), (0, 6));
    key(&mut tab, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(caret(&tab), (6, 6));
}

#[test]
fn enter_breaks_the_line_and_escape_leaves_the_text() {
    let (mut tab, id) = editing_tab("ab");
    key(&mut tab, Key::Enter, Modifiers::NONE);
    typed(&mut tab, "c");
    assert_eq!(text(&tab, id), "ab\nc");
    key(&mut tab, Key::Escape, Modifiers::NONE);
    assert!(tab.text_select.is_none());
    assert_eq!(tab.selection, Some(id), "the layer stays selected");
}

#[test]
fn edited_text_is_laid_out_again_for_the_caret_and_lines() {
    let (mut tab, id) = editing_tab("ab");
    key(&mut tab, Key::Enter, Modifiers::NONE);
    typed(&mut tab, "cd");
    assert!(
        tab.doc
            .node(id)
            .unwrap()
            .data
            .get("derivedTextData")
            .is_none(),
        "the edit drops the stored glyphs"
    );
    let (_, _, data) = tab.text_layout().unwrap();
    // The caret before "c" is at the start of the second line, below "a".
    let (x0, top0, _) = text_select::caret(&data, 0).expect("laid out with installed fonts");
    let (x3, top3, _) = text_select::caret(&data, 3).unwrap();
    assert!(
        (x3 - x0).abs() < 0.01 && top3 > top0 + 10.0,
        "{x0},{top0} {x3},{top3}"
    );
    // Up from the end of the second line goes to the first; Home/End find its edges.
    key(&mut tab, Key::ArrowUp, Modifiers::NONE);
    assert!(caret(&tab).1 <= 2, "{:?}", caret(&tab));
    key(&mut tab, Key::End, Modifiers::NONE);
    assert_eq!(
        caret(&tab),
        (2, 2),
        "the end of the first line, before the break"
    );
    key(&mut tab, Key::Home, Modifiers::NONE);
    assert_eq!(caret(&tab), (0, 0));
}

#[test]
fn the_range_is_counted_in_utf16_units_for_styling() {
    let (mut tab, id) = editing_tab("😀ab");
    tab.text_select = Some(Selection {
        node: id,
        anchor: 4,
        head: 6,
    });
    assert_eq!(utf16_range(&tab), Some((id, 2, 4)));
    // Text inside an instance is not edited.
    tab.inner = figstudio_fig::resolve::InstancePath::new(vec![Guid(1, 38), id]);
    assert!(editing(&tab).is_none());
}

#[test]
fn moving_the_caret_ends_the_typing_run_and_cut_removes_the_range() {
    let (mut tab, id) = editing_tab("ab");
    typed(&mut tab, "x");
    key(&mut tab, Key::ArrowLeft, Modifiers::NONE);
    typed(&mut tab, "y");
    assert_eq!(text(&tab, id), "abyx");
    crate::editor::undo_tab(&mut tab).unwrap();
    assert_eq!(text(&tab, id), "abx", "only the second run is undone");
    key(&mut tab, Key::A, Modifiers::COMMAND);
    apply(&egui::Context::default(), &mut tab, &Event::Cut);
    assert_eq!(text(&tab, id), "");
}
