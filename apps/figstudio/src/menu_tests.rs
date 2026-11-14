//! A menu must not offer document commands with no document or selection.
use super::*;

fn find(items: &[Item], action: Action) -> Option<&Item> {
    items.iter().find_map(|item| {
        if item.action == Some(action) {
            Some(item)
        } else {
            find(&item.children, action)
        }
    })
}

#[test]
fn empty_app_disables_document_commands_and_tracks_panels() {
    let mut h = egui_kittest::Harness::builder().build_eframe(|cc| App::new(cc, vec![]));
    let tree = tree(h.state());
    assert!(!find(&tree, Action::CloseTab).unwrap().enabled);
    assert!(!find(&tree, Action::ZoomFit).unwrap().enabled);
    assert!(!find(&tree, Action::ActualSize).unwrap().enabled);
    assert!(find(&tree, Action::Open).unwrap().enabled);
    assert!(find(&tree, Action::OpenLink).unwrap().enabled);
    assert!(!tree[0].children[2].enabled, "page export needs a tab");
    assert!(
        !tree[0].children[3].enabled,
        "selection export needs a selection"
    );
    assert_eq!(
        find(&tree, Action::ToggleLayers).unwrap().checked,
        Some(true)
    );
    assert_eq!(
        find(&tree, Action::ToggleProperties).unwrap().checked,
        Some(true)
    );
    let ctx = egui::Context::default();
    h.state_mut().apply(Action::ToggleLayers, &ctx);
    h.state_mut().apply(Action::ToggleProperties, &ctx);
    let tree = super::tree(h.state());
    assert_eq!(
        find(&tree, Action::ToggleLayers).unwrap().checked,
        Some(false)
    );
    assert_eq!(
        find(&tree, Action::ToggleProperties).unwrap().checked,
        Some(false)
    );
}

#[test]
fn export_selection_follows_selection_on_the_active_tab() {
    let mut h = crate::ui_tests::harness();
    assert!(tree(h.state())[0].children[2].enabled);
    assert!(!tree(h.state())[0].children[3].enabled);
    h.state_mut().tabs[0].select(Some(figstudio_fig::Guid(1, 25)));
    assert!(tree(h.state())[0].children[3].enabled);
    h.state_mut().tabs[0].select(None);
    assert!(!tree(h.state())[0].children[3].enabled);
}

#[test]
fn editing_actions_duplicate_delete_undo_redo_and_save_copy_round_trip() {
    let mut h = crate::ui_tests::harness();
    let ctx = egui::Context::default();
    assert!(!find(&tree(h.state()), Action::Undo).unwrap().enabled);
    assert!(!find(&tree(h.state()), Action::Redo).unwrap().enabled);
    assert!(
        !find(&tree(h.state()), Action::DuplicateSelection)
            .unwrap()
            .enabled
    );
    assert!(find(&tree(h.state()), Action::SaveCopy).unwrap().enabled);

    let orig_count = h.state().tabs[0].doc.nodes.len();
    h.state_mut().tabs[0].select(Some(figstudio_fig::Guid(1, 25)));
    assert!(
        find(&tree(h.state()), Action::DuplicateSelection)
            .unwrap()
            .enabled
    );

    h.state_mut().apply(Action::DuplicateSelection, &ctx);
    let dup_sel = h.state().tabs[0].selection;
    assert!(dup_sel.is_some());
    assert!(h.state().tabs[0].doc.nodes.len() > orig_count);
    assert!(h.state().tabs[0].dirty_unsaved);
    assert!(find(&tree(h.state()), Action::Undo).unwrap().enabled);

    h.state_mut().apply(Action::Undo, &ctx);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), orig_count);
    assert!(find(&tree(h.state()), Action::Redo).unwrap().enabled);

    h.state_mut().apply(Action::Redo, &ctx);
    assert!(h.state().tabs[0].doc.nodes.len() > orig_count);

    // Save copy in test mode writes to temp_dir and clears dirty_unsaved.
    h.state_mut().apply(Action::SaveCopy, &ctx);
    assert!(!h.state().tabs[0].dirty_unsaved);

    // T16: Undoing after saving moves away from `saved_revision`, so `dirty_unsaved` MUST be true!
    h.state_mut().apply(Action::Undo, &ctx);
    assert!(
        h.state().tabs[0].dirty_unsaved,
        "undoing past saved_revision must mark tab dirty_unsaved"
    );
    h.state_mut().apply(Action::Redo, &ctx);
    assert!(
        !h.state().tabs[0].dirty_unsaved,
        "redoing back to saved_revision must clear dirty_unsaved"
    );

    // Reselect the duplicated layer, delete it, and undo it back.
    let dup_count = h.state().tabs[0].doc.nodes.len();
    h.state_mut().tabs[0].select(dup_sel);
    h.state_mut().apply(Action::DeleteSelection, &ctx);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), orig_count);
    h.state_mut().apply(Action::Undo, &ctx);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), dup_count);
}

#[test]
fn cmd_v_paste_event_creates_text_layer_or_pastes_figma_html() {
    let mut h = crate::ui_tests::harness();
    let orig_count = h.state().tabs[0].doc.nodes.len();

    // 1. Plain text arriving via `egui::Event::Paste` (as macOS/Linux/web send on ⌘V / Ctrl+V)
    // creates a new TEXT layer on the page!
    h.event(egui::Event::Paste("Hello Paste Shortcut".to_owned()));
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), orig_count + 1);
    let new_sel = h.state().tabs[0].selection.expect("pasted text selected");
    assert_eq!(
        h.state().tabs[0]
            .doc
            .node(new_sel)
            .unwrap()
            .data
            .path(&["textData", "characters"])
            .and_then(figstudio_fig::Val::as_str),
        Some("Hello Paste Shortcut")
    );

    // 2. Figma HTML markup arriving via `egui::Event::Paste` pastes the full node subtree!
    let html = figstudio_fig::clipboard::encode_clipboard_html(
        &h.state().tabs[0].doc,
        &[figstudio_fig::Guid(1, 25)],
        1,
        None,
    )
    .unwrap();
    let before_figma_paste = h.state().tabs[0].doc.nodes.len();
    h.state_mut().tabs[0].select(None);
    h.event(egui::Event::Paste(html));
    h.run_steps(2);
    assert!(h.state().tabs[0].doc.nodes.len() > before_figma_paste);
}

#[cfg(feature = "mcp")]
#[test]
fn live_mcp_save_copy_enforces_path_safety_and_clears_dirty_unsaved() {
    use figstudio_mcp::live::LiveMutation;

    let tmp_dir = tempfile::tempdir().unwrap();
    let src_copy = tmp_dir.path().join("live_src.fig");
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/structure.fig");
    std::fs::copy(fixture, &src_copy).unwrap();

    let mut h = crate::ui_tests::harness();
    h.state_mut().tabs[0].real_path = src_copy.clone();
    h.state_mut().tabs[0].path = src_copy.clone();
    let tab_id = h.state().tabs[0].id;

    // Make an edit so the tab is dirty.
    let ctx = egui::Context::default();
    h.state_mut().tabs[0].select(Some(figstudio_fig::Guid(1, 25)));
    h.state_mut().apply(Action::DuplicateSelection, &ctx);
    assert!(h.state().tabs[0].dirty_unsaved);

    // Reject overwriting source, non-.fig, and .. traversal!
    assert!(
        crate::editor::handle_live_mutation(
            &mut h.state_mut().tabs,
            tab_id,
            LiveMutation::SaveCopy {
                out_path: src_copy.clone(),
                overwrite: true,
            },
        )
        .is_err()
    );
    assert!(
        crate::editor::handle_live_mutation(
            &mut h.state_mut().tabs,
            tab_id,
            LiveMutation::SaveCopy {
                out_path: tmp_dir.path().join("evil.txt"),
                overwrite: false,
            },
        )
        .is_err()
    );
    assert!(
        crate::editor::handle_live_mutation(
            &mut h.state_mut().tabs,
            tab_id,
            LiveMutation::SaveCopy {
                out_path: std::path::PathBuf::from("../escape.fig"),
                overwrite: false,
            },
        )
        .is_err()
    );

    // Valid save inside the tab's directory succeeds and clears `dirty_unsaved`!
    let valid_out = tmp_dir.path().join("live_copy.fig");
    let ok = crate::editor::handle_live_mutation(
        &mut h.state_mut().tabs,
        tab_id,
        LiveMutation::SaveCopy {
            out_path: valid_out.clone(),
            overwrite: false,
        },
    )
    .expect("valid live SaveCopy");
    assert_eq!(ok["unsaved"], serde_json::json!(false));
    assert!(!h.state().tabs[0].dirty_unsaved);

    // Second save without `overwrite: true` fails!
    assert!(
        crate::editor::handle_live_mutation(
            &mut h.state_mut().tabs,
            tab_id,
            LiveMutation::SaveCopy {
                out_path: valid_out,
                overwrite: false,
            },
        )
        .is_err()
    );
}
