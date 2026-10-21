//! Name hits retain a useful highlight when overlap hiding suppresses their label.
use super::*;

#[test]
fn a_hidden_name_label_find_hit_highlights_the_frame_outline() {
    use crate::labels::tests::{at_y, document, node};
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        node(2, 1, "FRAME", "Upper", 0.0, 300.0),
        at_y(node(3, 1, "FRAME", "Hidden", 0.0, 300.0), 60.0),
    ]);
    let mut tab = Tab::new(
        1,
        std::path::PathBuf::from("synthetic.fig"),
        std::sync::Arc::new(doc),
        None,
        std::time::Duration::ZERO,
    );
    tab.page = Guid(9, 1);
    tab.view = View {
        pan: Vec2::new(0.0, 100.0),
        zoom: 1.0,
    };
    tab.find_hits = vec![crate::find::Hit::Name(Guid(9, 3))];
    tab.find_fade = 1.0;
    tab.find_current = None;
    let ctx = egui::Context::default();
    crate::anim::set_reduced(&ctx, true);
    let expected = corners(&tab, Pos2::ZERO, Guid(9, 3)).unwrap();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let labels = crate::labels::labels(
            ui.painter(),
            &tab.doc,
            tab.page,
            tab.view,
            Pos2::ZERO,
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(2000.0)),
        );
        assert!(!labels.iter().any(|l| l.id == Guid(9, 3)));
        find_highlights(ui.painter(), &tab, Pos2::ZERO, &labels);
    });
    output.textures_delta.clear();
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Path(p) if p.points == expected)),
        "hidden name hit must retain the frame outline"
    );
}
