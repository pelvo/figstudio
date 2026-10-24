mod support;
use figstudio_diff::{RenderOpts, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at, node, object};

fn screen(id: u32, parent: u32) -> Node {
    let mut n = filled_rectangle_at(id, parent, (1.0, 1.0, 1.0), 0.0, 0.0, 300.0, 200.0);
    n.data.set("type", Val::Str("FRAME".into()));
    n.data.set("name", Val::Str("Screen <&>".into()));
    Node::from_data(n.data).unwrap()
}

#[test]
fn added_screen_inside_new_section_gets_its_own_thumbnail_and_plus_line() {
    let old = document(vec![]);
    let new = document(vec![node(2, Some(1), "SECTION", ""), screen(3, 2)]);
    for (a, b, sign) in [(&old, &new, "+"), (&new, &old, "-")] {
        let mut report = diff_documents(a, b);
        let _ = attach_render_diffs(a, b, &mut report, &RenderOpts::default());
        let s = &mut report.pages[0].screens[0];
        s.thumb_old = Some("render/0-1-s0-before.png".into());
        s.thumb_new = Some("render/0-1-s0-after.png".into());
        let html = render_html(&report);
        let screen = html
            .split("id=\"page-0-s1\"")
            .nth(1)
            .unwrap()
            .split("Other changes on this page")
            .next()
            .unwrap();
        assert!(
            screen.contains(&format!("class=\"sg\">{sign}</span>")),
            "{screen}"
        );
        assert!(screen.contains("Screen &lt;&amp;&gt;"));
        assert!(screen.contains("class=\"screen-thumb\""));
        assert!(screen.contains("href=\"./render/0-1-s0-"));
    }
}

#[test]
fn non_screen_text_is_on_the_page_not_the_screen_and_screen_pixels_alone_show_page() {
    let side = |new| {
        let mut text = node(4, Some(1), "TEXT", "");
        text.data
            .set("fontSize", Val::Float(if new { 14.0 } else { 12.0 }));
        document(vec![
            screen(2, 1),
            filled_rectangle_at(
                3,
                2,
                if new {
                    (0.0, 1.0, 0.0)
                } else {
                    (1.0, 0.0, 0.0)
                },
                80.0,
                70.0,
                20.0,
                20.0,
            ),
            Node::from_data(text.data).unwrap(),
        ])
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let html = render_html(&report);
    let other = html.split("Other changes on this page").nth(1).unwrap();
    assert!(other.contains("layer-4") && !other.contains("layer-3"));
    assert!(html.contains("Screens: 1 changed · 0 added · 0 removed"));
    assert!(html.contains("1 screen changed</span>"));
    assert!(html.contains("id=\"page-0-s1-a1\""));
    assert_eq!(html, render_html(&report));
    assert!(html.contains("default-src 'none'; img-src 'self'; style-src 'unsafe-inline'"));
    assert!(!html.contains("<script"));
    report.pages[0].changed.clear();
    let html = render_html(&report);
    assert!(html.contains("<section"));
    assert!(html.contains(" with no matched change line"));
}

#[test]
fn page_level_text_only_and_move_only_screens_obey_visibility_rule() {
    let old = document(vec![screen(2, 1)]);
    let mut moved = screen(2, 1);
    moved
        .data
        .set("transform", object(&[("m02", Val::Float(1000.0))]));
    let new = document(vec![Node::from_data(moved.data).unwrap()]);
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let html = render_html(&report);
    assert!(!html.contains("<section"));
    assert!(html.contains("1 screen moved or re-parented without change (hidden)"));
    let side = |size| {
        let mut t = node(2, Some(1), "TEXT", "");
        t.data.set("fontSize", Val::Float(size));
        document(vec![Node::from_data(t.data).unwrap()])
    };
    let report = diff_documents(&side(12.0), &side(14.0));
    let html = render_html(&report);
    assert!(html.contains("<section"));
    assert!(!html.contains("class=\"sec screen\""));
    assert!(
        html.split("Other changes on this page")
            .nth(1)
            .unwrap()
            .contains("fontSize")
    );
}

#[test]
fn paint_opacity_and_visibility_rows_name_their_paint() {
    let paint = |opacity: f64, visible: bool| {
        let one = |extra: (&str, Val)| {
            Val::Arr(vec![object(&[
                ("type", Val::Str("SOLID".into())),
                (
                    "color",
                    object(&[
                        ("r", Val::Float(1.0)),
                        ("g", Val::Float(0.0)),
                        ("b", Val::Float(0.0)),
                        ("a", Val::Float(1.0)),
                    ]),
                ),
                extra,
            ])])
        };
        let mut n = filled_rectangle_at(4, 2, (1.0, 0.0, 0.0), 20.0, 20.0, 40.0, 40.0);
        n.data
            .set("fillPaints", one(("opacity", Val::Float(opacity))));
        n.data
            .set("strokePaints", one(("visible", Val::Bool(visible))));
        Node::from_data(n.data).unwrap()
    };
    let old = document(vec![screen(2, 1), paint(1.0, true)]);
    let new = document(vec![screen(2, 1), paint(0.5, false)]);
    let html = render_html(&diff_documents(&old, &new));
    assert!(html.contains("Fill opacity:"), "{html}");
    assert!(html.contains("Stroke visible:"), "{html}");
}
