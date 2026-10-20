use super::inner_fixture::path;
use super::*;
use egui::Modifiers;

#[test]
fn inner_hover_outlines_the_sibling_or_deep_click_target() {
    let mut h = inner_fixture::harness();
    let from = inner_nested::at(&h, &[103]);
    pick::click(&mut h, from, Modifiers::COMMAND);
    let at = inner_nested::at(&h, &[105]);
    h.event(Event::PointerMoved(at));
    for (modifiers, target) in [(Modifiers::NONE, 104), (Modifiers::COMMAND, 105)] {
        h.event(Event::ModifiersChanged(modifiers));
        h.step();
        let tab = &h.state().tabs[0];
        let page = tab.view.to_page(tab.canvas_rect.unwrap().min, at);
        assert_eq!(
            crate::canvas_pick::hover(
                tab,
                Some(figstudio_render::kurbo::Point::new(
                    f64::from(page.x),
                    f64::from(page.y)
                )),
                modifiers.command
            ),
            Some(path(&[target]))
        );
        let b = figstudio_render::Renderer::new()
            .path_bounds(&tab.doc, &path(&[target]))
            .unwrap();
        let origin = tab.canvas_rect.unwrap().min;
        let rect = egui::Rect::from_min_max(
            tab.view
                .to_screen(origin, egui::pos2(b.x0 as f32, b.y0 as f32)),
            tab.view
                .to_screen(origin, egui::pos2(b.x1 as f32, b.y1 as f32)),
        );
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(r) if r.rect == rect && r.stroke.width == 1.0)));
        assert_eq!(tab.inner, Some(path(&[103])));
    }
}
