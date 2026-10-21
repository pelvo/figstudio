use super::tests::{at_y, document, node};
use super::*;
use figstudio_fig::{Obj, Val};

fn rotated_blocker() -> figstudio_fig::Node {
    let mut n = node(2, 1, "FRAME", "Rotated", 300.0, 300.0);
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let mut t = Obj::new();
    for (key, value) in [
        ("m00", s),
        ("m10", s),
        ("m01", -s),
        ("m11", s),
        ("m02", 300.0),
        ("m12", 100.0),
    ] {
        t.insert(key, Val::Float(value));
    }
    n.data.set("transform", Val::Obj(t));
    n
}

fn names(doc: &Document, zoom: f32) -> Vec<String> {
    let ctx = egui::Context::default();
    let mut names = Vec::new();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        names = labels(
            ui.painter(),
            doc,
            Guid(9, 1),
            View {
                pan: Vec2::new(0.0, 100.0),
                zoom,
            },
            Pos2::ZERO,
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(2000.0)),
        )
        .into_iter()
        .map(|l| l.text)
        .collect();
    });
    out.textures_delta.clear();
    names
}

#[test]
fn rotated_neighbour_empty_aabb_corner_keeps_the_label() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        rotated_blocker(),
        at_y(node(3, 1, "FRAME", "Free", 440.0, 300.0), 145.0),
    ]);
    assert!(names(&doc, 1.0).contains(&"Free".to_string()));
}

#[test]
fn rotated_neighbour_actual_ink_hides_the_label() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        rotated_blocker(),
        at_y(node(3, 1, "FRAME", "Blocked", 360.0, 300.0), 200.0),
    ]);
    assert!(!names(&doc, 1.0).contains(&"Blocked".to_string()));
}

#[test]
fn colliding_labels_keep_only_the_first_in_page_order() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        node(2, 1, "FRAME", "First label", 0.0, 300.0),
        node(3, 1, "FRAME", "Second label", 10.0, 300.0),
    ]);
    assert_eq!(names(&doc, 1.0), ["First label"]);
}

#[test]
fn low_zoom_rotated_blocker_without_a_label_still_blocks_actual_ink_only() {
    for (x, y, visible) in [(440.0, 145.0, true), (360.0, 200.0, false)] {
        let doc = document(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            rotated_blocker(),
            at_y(node(3, 1, "FRAME", "Target", x, 1200.0), y),
        ]);
        let labels = names(&doc, 0.25);
        assert!(!labels.contains(&"Rotated".to_string()));
        assert_eq!(
            labels.contains(&"Target".to_string()),
            visible,
            "x={x} y={y}"
        );
    }
}
