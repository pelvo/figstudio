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
