use super::*;
use egui::{Key, Modifiers};
use text_select::{double_click, glyph_at, key, text_harness};

#[test]
fn double_click_and_drag_share_the_scalar_cluster_mapping() {
    let (mut h, mut c) = (text_harness(), Vec::new());
    let mut doc = figstudio_fig::Document::open(fixture("noise_and_grain.fig")).unwrap();
    let data = &mut doc
        .nodes
        .iter_mut()
        .find(|n| n.id == Guid(1, 5))
        .unwrap()
        .data;
    data.get_mut("textData")
        .unwrap()
        .set("characters", figstudio_fig::Val::Str("é😀fiZ".into()));
    if let figstudio_fig::Val::Arr(glyphs) = data
        .get_mut("derivedTextData")
        .unwrap()
        .get_mut("glyphs")
        .unwrap()
    {
        glyphs.truncate(4);
        for (g, ordinal) in glyphs.iter_mut().zip([0, 1, 2, 4]) {
            g.set("firstCharacter", figstudio_fig::Val::Int(ordinal));
        }
    }
    h.state_mut().tabs[0].doc = std::sync::Arc::new(doc);
    let at = glyph_at(&h, 3);
    double_click(&mut h, &mut c, at);
    let s = h.state().tabs[0].text_select.unwrap();
    assert_eq!((s.anchor, s.head), (8, 8));
    let from = glyph_at(&h, 2);
    let to = glyph_at(&h, 3);
    h.run_steps(60);
    h.event(Event::PointerMoved(from));
    step(&mut h, &mut c);
    press(&h, from, PointerButton::Primary, true);
    step(&mut h, &mut c);
    h.event(Event::PointerMoved(to));
    step(&mut h, &mut c);
    press(&h, to, PointerButton::Primary, false);
    step(&mut h, &mut c);
    key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["fi"]);
}

#[test]
fn a_new_drag_anchors_at_the_press_not_the_old_caret() {
    for (start, end, expected) in [(6, 9, "ith"), (9, 6, "ith")] {
        let (mut h, mut c) = (text_harness(), Vec::new());
        let old = glyph_at(&h, 2);
        double_click(&mut h, &mut c, old);
        assert_eq!(h.state().tabs[0].text_select.unwrap().anchor, 2);
        let from = glyph_at(&h, start);
        let to = glyph_at(&h, end);
        h.run_steps(60);
        h.event(Event::PointerMoved(from));
        step(&mut h, &mut c);
        press(&h, from, PointerButton::Primary, true);
        step(&mut h, &mut c);
        // Cross the drag threshold in one frame: the current pointer is not the origin.
        h.event(Event::PointerMoved(to));
        step(&mut h, &mut c);
        press(&h, to, PointerButton::Primary, false);
        step(&mut h, &mut c);
        key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
        assert_eq!(c, [expected]);
        let s = h.state().tabs[0].text_select.unwrap();
        assert_eq!((s.anchor, s.head), (start, end));
        assert!(h.state().tabs[0].marquee.rect().is_none());
    }
}
