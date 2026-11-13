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

    // Reselect the duplicated layer, delete it, and undo it back.
    let dup_count = h.state().tabs[0].doc.nodes.len();
    h.state_mut().tabs[0].select(dup_sel);
    h.state_mut().apply(Action::DeleteSelection, &ctx);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), orig_count);
    h.state_mut().apply(Action::Undo, &ctx);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), dup_count);
}
