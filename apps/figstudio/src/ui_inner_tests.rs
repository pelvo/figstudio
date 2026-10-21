use super::*;
use egui::{Key, Modifiers};
use figstudio_fig::resolve::{self, InstancePath};
use figstudio_render::Renderer;

fn path(instance: u32, keys: &[u32]) -> InstancePath {
    InstancePath::new(
        std::iter::once(Guid(1, instance))
            .chain(keys.iter().map(|n| Guid(1, *n)))
            .collect(),
    )
    .unwrap()
}

fn harness_for(instance: u32) -> Harness<'static, App> {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().reduce_motion = true;
    h.state_mut().tabs[0].fit_node = Some(Guid(1, instance));
    h.run_steps(4);
    h
}

fn inner_centre(h: &Harness<'_, App>, path: &InstancePath) -> Pos2 {
    let tab = &h.state().tabs[0];
    let b = Renderer::new().path_bounds(&tab.doc, path).unwrap();
    tab.view.to_screen(
        tab.canvas_rect.unwrap().min,
        Pos2::new(b.center().x as f32, b.center().y as f32),
    )
}

#[test]
fn instances_drill_to_text_then_copy_the_resolved_characters() {
    for (instance, expected) in [(38, "XYZ"), (45, "ABC")] {
        let (mut h, mut c) = (harness_for(instance), Vec::new());
        let text = path(instance, &[42]);
        assert_eq!(
            crate::text_select::characters(
                &resolve::resolve_path(&h.state().tabs[0].doc, &text)
                    .unwrap()
                    .data
            ),
            expected
        );
        let at = inner_centre(&h, &text);
        pick::click(&mut h, at, Modifiers::NONE);
        assert_eq!(h.state().tabs[0].selection, Some(Guid(1, instance)));
        assert!(h.state().tabs[0].inner.is_none());
        text_select::double_click(&mut h, &mut c, at);
        assert_eq!(h.state().tabs[0].inner, Some(text.clone()));
        assert!(h.state().tabs[0].text_select.is_none());
        text_select::double_click(&mut h, &mut c, at);
        assert!(h.state().tabs[0].text_select.is_some());
        text_select::key(&mut h, &mut c, Key::A, Modifiers::COMMAND);
        text_select::key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
        assert_eq!(c, [expected]);
        assert_eq!(h.state().tabs[0].selection, Some(Guid(1, instance)));
    }
}

#[test]
fn inner_sibling_click_parent_key_and_escape_preserve_outer_contract() {
    let (mut h, mut c) = (harness_for(38), Vec::new());
    let text = path(38, &[42]);
    let at = inner_centre(&h, &text);
    pick::click(&mut h, at, Modifiers::NONE);
    text_select::double_click(&mut h, &mut c, at);
    let shape = path(38, &[35]);
    let at = inner_centre(&h, &shape);
    pick::click(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].inner, Some(shape));
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::SHIFT);
    assert!(h.state().tabs[0].inner.is_none());
    assert_eq!(h.state().tabs[0].selection, Some(Guid(1, 38)));
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::NONE);
    assert!(h.state().tabs[0].inner.is_some());
    text_select::key(&mut h, &mut c, Key::Escape, Modifiers::NONE);
    assert!(h.state().tabs[0].inner.is_none());
    assert!(h.state().tabs[0].selection.is_none());
}

#[test]
fn changing_selection_or_page_clears_the_inner_path_and_text_mode() {
    let (mut h, mut c) = (harness_for(38), Vec::new());
    let text = path(38, &[42]);
    let at = inner_centre(&h, &text);
    pick::click(&mut h, at, Modifiers::COMMAND);
    assert_eq!(h.state().tabs[0].inner, Some(text.clone()));
    text_select::double_click(&mut h, &mut c, at);
    assert!(h.state().tabs[0].text_select.is_some());
    let outside = h.state().tabs[0].canvas_rect.unwrap().left_bottom() + egui::vec2(10.0, -10.0);
    pick::click(&mut h, outside, Modifiers::NONE);
    assert!(h.state().tabs[0].inner.is_none());
    assert!(h.state().tabs[0].text_select.is_none());
    let tab = &mut h.state_mut().tabs[0];
    tab.select(Some(Guid(1, 38)));
    tab.set_inner(Some(text));
    tab.set_page(Guid(0, 999));
    assert!(tab.inner.is_none());
}

#[test]
fn escape_leaves_inner_text_mode_before_clearing_the_selection() {
    let (mut h, mut c) = (harness_for(38), Vec::new());
    let text = path(38, &[42]);
    let at = inner_centre(&h, &text);
    pick::click(&mut h, at, Modifiers::COMMAND);
    text_select::double_click(&mut h, &mut c, at);
    text_select::key(&mut h, &mut c, Key::Escape, Modifiers::NONE);
    assert!(h.state().tabs[0].text_select.is_none());
    assert_eq!(h.state().tabs[0].inner, Some(text));
    text_select::key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["XYZ"]);
    text_select::key(&mut h, &mut c, Key::Escape, Modifiers::NONE);
    assert!(h.state().tabs[0].inner.is_none());
    assert!(h.state().tabs[0].selection.is_none());
}
