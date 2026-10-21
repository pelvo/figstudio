use super::*;
use crate::{icons::Icon, theme};
use figstudio_fig::{Node, Obj, Val};

pub(crate) fn node(n: u32, parent: u32, kind: &str, name: &str, x: f64, w: f64) -> Node {
    let mut o = Obj::new();
    let guid = |n: u32| Guid(9, n).to_val();
    let mut pi = Obj::new();
    pi.insert("guid", guid(parent));
    pi.insert("position", Val::Str(format!("{n:03}")));
    let mut size = Obj::new();
    size.insert("x", Val::Float(w));
    size.insert("y", Val::Float(50.0));
    let mut tf = Obj::new();
    tf.insert("m00", Val::Float(1.0));
    tf.insert("m11", Val::Float(1.0));
    tf.insert("m02", Val::Float(x));
    o.insert("guid", guid(n));
    o.insert("type", Val::Str(kind.into()));
    o.insert("name", Val::Str(name.into()));
    o.insert("parentIndex", Val::Obj(pi));
    o.insert("size", Val::Obj(size));
    o.insert("transform", Val::Obj(tf));
    Node::from_data(Val::Obj(o)).unwrap()
}

pub(crate) fn document(nodes: Vec<Node>) -> Document {
    let mut doc =
        Document::from_bytes(include_bytes!("../../../tests/fixtures/structure.fig")).unwrap();
    doc.set_nodes(nodes).unwrap();
    doc
}

fn visible_labels(doc: &Document) -> Vec<Label> {
    let ctx = egui::Context::default();
    let mut out = Vec::new();
    let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
        out = labels(
            ui.painter(),
            doc,
            Guid(9, 1),
            View {
                pan: Vec2::new(0.0, 100.0),
                zoom: 1.0,
            },
            Pos2::ZERO,
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(2000.0)),
        );
    });
    frame.textures_delta.clear();
    out
}

pub(crate) fn at_y(mut n: Node, y: f64) -> Node {
    let mut tf = n.data.get("transform").cloned().unwrap();
    if let Val::Obj(o) = &mut tf {
        o.insert("m12", Val::Float(y));
    }
    n.data.set("transform", tf);
    n
}

#[test]
fn a_label_that_would_cover_another_frame_is_hidden() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        node(2, 1, "FRAME", "Upper", 0.0, 300.0),
        // Its label would sit on the bottom of "Upper", so it is not shown.
        at_y(node(3, 1, "FRAME", "Lower", 0.0, 300.0), 60.0),
        // Far enough below "Lower" for its label to sit in empty canvas.
        at_y(node(4, 1, "SECTION", "Free", 0.0, 300.0), 200.0),
        // Frames inside a section keep their labels on the section.
        at_y(node(5, 4, "FRAME", "Inside", 10.0, 280.0), 40.0),
    ]);
    let names: Vec<_> = visible_labels(&doc).into_iter().map(|l| l.text).collect();
    assert_eq!(names, ["Upper", "Free", "Inside"]);
}

#[test]
fn nested_sections_exempt_all_ancestors_without_exempting_siblings() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        node(2, 1, "SECTION", "Outer", 0.0, 1000.0),
        at_y(node(3, 2, "SECTION", "Inner", 20.0, 900.0), 30.0),
        at_y(node(4, 3, "FRAME", "Child", 20.0, 300.0), 30.0),
        at_y(node(5, 3, "FRAME", "Blocked sibling", 20.0, 300.0), 90.0),
    ]);
    let names: Vec<_> = visible_labels(&doc).into_iter().map(|l| l.text).collect();
    assert_eq!(names, ["Outer", "Inner", "Child"]);
}

#[test]
fn top_level_frames_and_sections_and_their_frames_get_labels() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        node(2, 1, "SECTION", "Section A", 0.0, 400.0),
        // Below the section pill, so its label sits inside the section, not on the pill.
        at_y(node(3, 2, "FRAME", "Frame in section", 10.0, 300.0), 40.0),
        node(4, 3, "FRAME", "Nested frame", 0.0, 300.0),
        node(5, 1, "FRAME", "Top frame", 500.0, 300.0),
        node(6, 1, "RECTANGLE", "Shape", 900.0, 300.0),
        node(7, 1, "FRAME", "Small frame", 1300.0, 200.0),
    ]);
    let names: Vec<_> = visible_labels(&doc).into_iter().map(|l| l.text).collect();
    assert_eq!(names, ["Section A", "Frame in section", "Top frame"]);
}

#[test]
fn label_style_distinguishes_components_instances_and_frames() {
    for (kind, expected) in [
        ("SYMBOL", Some(Icon::Component)),
        ("COMPONENT_SET", Some(Icon::Component)),
        ("INSTANCE", Some(Icon::Instance)),
        ("FRAME", None),
        ("SECTION", None),
    ] {
        let n = node(2, 1, kind, "Label", 0.0, 300.0);
        for (background, palette) in [
            (Color32::WHITE, theme::LIGHT),
            (Color32::from_gray(30), theme::DARK),
        ] {
            let (icon, color) = style(&n, background);
            assert_eq!(icon, expected, "{kind}");
            if expected.is_some() {
                assert_eq!(color, palette.component, "{kind}");
                assert!(theme::contrast(color, background) >= 4.5);
            } else if kind == "SECTION" {
                assert_eq!(color, Color32::WHITE, "section pills stay unchanged");
            } else {
                assert!(theme::contrast(color, background) >= 4.5);
            }
        }
    }
    let mut set = node(2, 1, "FRAME", "Variants", 0.0, 300.0);
    set.data.set("isStateGroup", Val::Bool(true));
    assert_eq!(
        style(&set, Color32::WHITE),
        (Some(Icon::Component), theme::LIGHT.component)
    );
    set.data.set("isStateGroup", Val::Bool(false));
    // Small text must remain readable even when a page has a mid-grey background.
    for gray in 0..=255 {
        let bg = Color32::from_gray(gray);
        assert!(
            theme::contrast(style(&set, bg).1, bg) >= 4.5,
            "page gray {gray}"
        );
    }
}

#[test]
fn component_label_paints_purple_text_and_an_icon_inside_its_find_rect() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        node(2, 1, "SYMBOL", "Synthetic component", 40.0, 300.0),
    ]);
    let mut labels = visible_labels(&doc);
    assert_eq!(labels.len(), 1);
    labels[0].at.y = 80.0;
    labels[0].max_width = 120.0;
    labels[0].layout = None;
    let ctx = egui::Context::default();
    let mut bounds = Rect::NOTHING;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        bounds = rect(ui.painter(), &labels[0]);
        paint(ui.painter(), &labels);
    });
    // run_ui output owns the font texture delta until the next frame.
    out.textures_delta.clear();
    let text = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == "Synthetic component" => Some(t),
            _ => None,
        })
        .expect("component name painted");
    assert_eq!(
        text.galley.job.sections[0].format.color,
        theme::LIGHT.component
    );
    let icon = out.shapes.iter().any(|s| match &s.shape {
        egui::Shape::Path(path) => {
            path.stroke.color == egui::epaint::ColorMode::Solid(theme::LIGHT.component)
                && path
                    .points
                    .iter()
                    .all(|p| bounds.contains(*p) && p.x < text.pos.x)
        }
        _ => false,
    });
    assert!(
        icon,
        "a component icon must be painted before its text inside the find rectangle"
    );
    assert!(bounds.contains_rect(Rect::from_min_size(text.pos, text.galley.size())));
    assert!(
        bounds.width() <= labels[0].max_width,
        "the icon must leave room for ellipsis"
    );
}

#[test]
fn plain_frame_labels_are_readable_on_light_and_dark_pages() {
    for gray in [30_u8, 255] {
        let mut page = node(1, 0, "CANVAS", "page", 0.0, 0.0);
        let c = f64::from(gray) / 255.0;
        let mut background = Obj::new();
        for channel in ["r", "g", "b"] {
            background.insert(channel, Val::Float(c));
        }
        background.insert("a", Val::Float(1.0));
        page.data.set("backgroundColor", Val::Obj(background));
        let doc = document(vec![page, node(2, 1, "FRAME", "Frame", 0.0, 300.0)]);
        let labels = visible_labels(&doc);
        let ctx = egui::Context::default();
        let mut color = Color32::TRANSPARENT;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let (galley, ..) = layout(ui.painter(), &labels[0]);
            color = galley.job.sections[0].format.color;
        });
        out.textures_delta.clear();
        assert!(
            theme::contrast(color, Color32::from_gray(gray)) >= 4.5,
            "frame label {color:?} lacks small-text contrast on page gray {gray}"
        );
    }
}

#[test]
fn frame_labels_inside_a_dark_section_read_on_the_section_not_the_page() {
    let mut section = node(2, 1, "SECTION", "Dark section", 0.0, 1000.0);
    let mut fill = Obj::new();
    let mut c = Obj::new();
    for channel in ["r", "g", "b"] {
        c.insert(channel, Val::Float(0.27));
    }
    c.insert("a", Val::Float(1.0));
    fill.insert("type", Val::Str("SOLID".into()));
    fill.insert("color", Val::Obj(c));
    section
        .data
        .set("fillPaints", Val::Arr(vec![Val::Obj(fill)]));
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        section,
        at_y(node(3, 2, "FRAME", "Inside", 20.0, 900.0), 60.0),
        at_y(node(4, 2, "SYMBOL", "Component", 20.0, 900.0), 300.0),
    ]);
    let labels = visible_labels(&doc);
    let find = |name: &str| labels.iter().find(|l| l.text == name).expect(name).color;
    let dark = Color32::from_gray(69);
    let inside = find("Inside");
    assert!(
        theme::contrast(inside, dark) >= 4.5,
        "frame label {inside:?} unreadable on its dark section"
    );
    // Component purple is a fixed palette colour; the section picks the dark one.
    assert_eq!(find("Component"), theme::DARK.component);
}
