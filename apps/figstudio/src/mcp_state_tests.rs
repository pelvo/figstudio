use super::*;
use figstudio_fig::Document;
use std::{path::PathBuf, time::Duration};

fn tabs() -> Vec<Tab> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    vec![
        Tab::new(7, path.clone(), doc.clone(), None, Duration::ZERO),
        Tab::new(9, path, doc, None, Duration::ZERO),
    ]
}

#[test]
fn shared_key_tracks_selection_active_document_and_rounded_view() {
    let mut tabs = tabs();
    let initial = key(&tabs, 0);
    tabs[0].hover = Some(Guid(1, 25));
    assert_eq!(key(&tabs, 0), initial);
    tabs[0].select(Some(Guid(1, 25)));
    assert_ne!(key(&tabs, 0), initial);
    assert_ne!(key(&tabs, 0), key(&tabs, 1));
    tabs[0].canvas_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(800., 600.),
    ));
    let visible = key(&tabs, 0);
    tabs[0].view.pan.x += 0.2;
    assert_eq!(key(&tabs, 0), visible);
    tabs[0].view.pan.x += 2.;
    assert_ne!(key(&tabs, 0), visible);
    let before = key(&tabs, 0);
    tabs[0].doc = Arc::new(Document::open(&tabs[0].path).unwrap());
    assert_ne!(key(&tabs, 0), before);
}

#[test]
fn shared_reveal_selects_switches_page_and_fits_without_mutating_on_closed_tab() {
    let mut tabs = tabs();
    let mut active = 0;
    tabs[1].set_page(Guid(0, 99));
    reveal(&mut tabs, &mut active, 9, Guid(1, 25)).unwrap();
    assert_eq!(active, 1);
    assert_eq!(tabs[1].selection, Some(Guid(1, 25)));
    assert_eq!(tabs[1].page, Guid(0, 1));
    assert_eq!(tabs[1].fit_node, Some(Guid(1, 25)));
    let before = key(&tabs, active);
    assert_eq!(
        reveal(&mut tabs, &mut active, 42, Guid(1, 25)),
        Err("tab closed".into())
    );
    assert_eq!(key(&tabs, active), before);
}

#[test]
fn revealing_a_page_switches_to_it_instead_of_selecting_it() {
    let mut tabs = tabs();
    let mut active = 0;
    tabs[1].set_page(Guid(0, 99));
    reveal(&mut tabs, &mut active, 9, Guid(0, 1)).unwrap();
    assert_eq!((active, tabs[1].page), (1, Guid(0, 1)));
    assert_eq!((tabs[1].selection, tabs[1].fit_node), (None, None));
}
