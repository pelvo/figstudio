use super::*;
use figstudio_fig::{Obj, Val};
use tests::{at_y, document, node};

fn obj(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (key, value)| {
        o.insert(key, value.clone());
        o
    }))
}

fn colour(gray: f64, alpha: f64) -> Val {
    obj(&[
        ("r", Val::Float(gray)),
        ("g", Val::Float(gray)),
        ("b", Val::Float(gray)),
        ("a", Val::Float(alpha)),
    ])
}

fn gradient(kind: &str, opacity: f64) -> Val {
    obj(&[
        ("type", Val::Str(kind.into())),
        ("opacity", Val::Float(opacity)),
        (
            "stops",
            Val::Arr(vec![
                obj(&[("position", Val::Float(0.0)), ("color", colour(0.2, 1.0))]),
                obj(&[("position", Val::Float(1.0)), ("color", colour(0.34, 1.0))]),
            ]),
        ),
    ])
}

#[test]
fn gradient_section_labels_use_a_readable_average_backdrop() {
    for kind in [
        "GRADIENT_LINEAR",
        "GRADIENT_RADIAL",
        "GRADIENT_ANGULAR",
        "GRADIENT_DIAMOND",
    ] {
        let mut section = node(2, 1, "SECTION", "Section", 0.0, 1000.0);
        section
            .data
            .set("fillPaints", Val::Arr(vec![gradient(kind, 1.0)]));
        let doc = document(vec![
            node(1, 0, "CANVAS", "Page", 0.0, 0.0),
            section,
            at_y(node(3, 2, "FRAME", "Inside", 20.0, 900.0), 60.0),
        ]);
        let ctx = egui::Context::default();
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            let labels = labels(
                ui.painter(),
                &doc,
                Guid(9, 1),
                View {
                    pan: Vec2::new(0.0, 100.0),
                    zoom: 1.0,
                },
                Pos2::ZERO,
                Rect::from_min_size(Pos2::ZERO, Vec2::splat(2000.0)),
            );
            let label = labels.iter().find(|l| l.id == Guid(9, 3)).unwrap();
            assert!(
                theme::contrast(label.color, Color32::from_gray(69)) >= 4.5,
                "{kind} label {:?} lacks dark-section contrast",
                label.color
            );
        });
        frame.textures_delta.clear();
    }
}

#[test]
fn gradient_opacity_and_transparent_stops_preserve_the_underlying_backdrop() {
    for opacity in [0.0, 0.5, 1.0] {
        let mut section = node(2, 1, "SECTION", "Section", 0.0, 1000.0);
        let mut paint = gradient("GRADIENT_LINEAR", opacity);
        paint.set(
            "stops",
            Val::Arr(vec![
                obj(&[("color", colour(0.0, 1.0))]),
                obj(&[("color", colour(1.0, 0.0))]),
            ]),
        );
        section.data.set(
            "fillPaints",
            Val::Arr(vec![paint, obj(&[("type", Val::Str("IMAGE".into()))])]),
        );
        let doc = document(vec![
            node(1, 0, "CANVAS", "Page", 0.0, 0.0),
            section,
            at_y(node(3, 2, "FRAME", "Inside", 20.0, 900.0), 60.0),
        ]);
        let named = candidates::collect(&doc, Guid(9, 1));
        let colours = backdrops(&doc, &named, Color32::WHITE);
        let expected = (255.0 * (1.0 - opacity * 0.5)).round() as u8;
        assert_eq!(
            colours[1],
            Color32::from_gray(expected),
            "opacity={opacity}"
        );
    }
}
