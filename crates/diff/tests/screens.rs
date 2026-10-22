mod support;
use figstudio_diff::{RenderOpts, ScreenStatus, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Document, Guid, Node, Val};
use support::{document, filled_rectangle_at, node, object};

fn screen(id: u32, parent: u32, x: f64, kind: &str) -> Node {
    let mut n = filled_rectangle_at(id, parent, (1.0, 1.0, 1.0), x, 0.0, 400.0, 300.0);
    n.data.set("type", Val::Str(kind.into()));
    n.data.set("name", Val::Str("Screen".into()));
    Node::from_data(n.data).unwrap()
}
fn pair(recolored: bool) -> (Document, Document) {
    let side = |new: bool| {
        document(vec![
            node(2, Some(1), "SECTION", "a"),
            node(3, Some(1), "SECTION", "b"),
            screen(
                4,
                if new { 3 } else { 2 },
                if new { 1000.0 } else { 0.0 },
                "FRAME",
            ),
            filled_rectangle_at(
                5,
                4,
                if new && recolored {
                    (0.0, 1.0, 0.0)
                } else {
                    (1.0, 0.0, 0.0)
                },
                100.0,
                100.0,
                20.0,
                20.0,
            ),
            filled_rectangle_at(6, 4, (0.0, 0.0, 1.0), 200.0, 100.0, 20.0, 20.0),
        ])
    };
    (side(false), side(true))
}

#[test]
fn moved_screen_is_hidden_but_recolored_child_has_one_local_region_and_sharp_crop() {
    for changed in [false, true] {
        let (old, new) = pair(changed);
        let mut report = diff_documents(&old, &new);
        let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
        let page = &report.pages[0];
        if !changed {
            assert!(page.screens.is_empty());
            assert_eq!((page.screens_unchanged, page.screens_moved), (1, 1));
        } else {
            let s = &page.screens[0];
            assert_eq!(s.status, ScreenStatus::Changed);
            assert!(s.moved);
            assert_eq!((s.width, s.height), (400, 300));
            assert_eq!(s.pixels_changed, Some(400));
            assert_eq!(s.regions.len(), 1);
            let r = &s.regions[0];
            assert_eq!(r.nodes, vec![Guid(0, 5)]);
            assert!(r.x < 100 && r.x + r.width >= 120);
            let crop =
                image::load_from_memory(&images[&page.id].screens[0].regions[0].after).unwrap();
            assert_eq!(
                (Some(crop.width()), Some(crop.height())),
                (r.image_width, r.image_height)
            );
            assert!(crop.width() >= 640 && crop.height() >= 320);
            assert!(
                crop.to_rgba8()
                    .pixels()
                    .filter(|p| p.0 == [0, 255, 0, 255])
                    .count()
                    >= 1500
            );
        }
    }
}

#[test]
fn recreated_screen_ignores_identity_churn_and_added_removed_screens_have_thumbnails() {
    let old = document(vec![screen(4, 1, 0.0, "FRAME")]);
    let new = document(vec![screen(7, 1, 900.0, "FRAME")]);
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    assert!(report.pages[0].screens.is_empty());
    assert_eq!(report.pages[0].screens_unchanged, 1);
    assert!(!render_html(&report).contains("<section"));
    let new = document(vec![screen(8, 1, 0.0, "GROUP")]);
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let s = &report.pages[0].screens;
    assert_eq!(
        s.iter().map(|s| s.status).collect::<Vec<_>>(),
        vec![ScreenStatus::Removed, ScreenStatus::Added]
    );
    let pics = &images[&Guid(0, 1)].screens;
    assert!(pics[0].before.is_some() && pics[0].after.is_none());
    assert!(pics[1].after.is_some() && pics[1].before.is_none());
}

#[test]
fn inherited_instance_change_is_not_erased_by_moving_its_section() {
    let side = |new| {
        let mut section = node(2, Some(1), "SECTION", "");
        section.data.set(
            "transform",
            object(&[("m02", Val::Float(if new { 1000.0 } else { 0.0 }))]),
        );
        let mut instance = screen(4, 2, 0.0, "INSTANCE");
        // The master changes; the instance's own stored deltas are ignored geometry churn.
        instance
            .data
            .set("symbolData", object(&[("symbolID", Guid(0, 10).to_val())]));
        instance.data.set(
            "derivedSymbolData",
            Val::Arr(vec![object(&[(
                "fillGeometry",
                Val::Arr(vec![object(&[(
                    "styleID",
                    Val::Int(if new { 2 } else { 1 }),
                )])]),
            )])]),
        );
        let mut master = screen(10, 11, 0.0, "SYMBOL");
        master.data.set(
            "fillPaints",
            support::solid(if new { 0.0 } else { 1.0 }, 0.0, 0.0),
        );
        if let Val::Obj(data) = &mut instance.data {
            data.remove("fillPaints");
        }
        support::two_page_document(
            vec![
                Node::from_data(section.data).unwrap(),
                Node::from_data(instance.data).unwrap(),
            ],
            vec![
                Node::from_data(master.data).unwrap(),
                filled_rectangle_at(
                    12,
                    10,
                    if new {
                        (0.0, 1.0, 0.0)
                    } else {
                        (1.0, 0.0, 0.0)
                    },
                    100.0,
                    100.0,
                    20.0,
                    20.0,
                ),
            ],
        )
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let page = report.pages.iter().find(|p| p.id == Guid(0, 1)).unwrap();
    assert_eq!(page.screens.len(), 1);
    assert_eq!(page.screens[0].status, ScreenStatus::Changed);
    assert!(!page.screens[0].regions.is_empty());
    assert_eq!(page.screens[0].pixels_changed, Some(400));
    assert!(page.screens[0].moved);
    let html = render_html(&report);
    assert!(html.contains("<section"));
    assert!(html.contains(" with no matched change line"));
}

#[test]
fn recreated_screen_lists_layers_it_gained_or_lost_but_not_identity_churn() {
    let named = |mut n: Node, name: &str| {
        n.data.set("name", Val::Str(name.into()));
        Node::from_data(n.data).unwrap()
    };
    let dot = |id, parent| {
        named(
            filled_rectangle_at(id, parent, (1.0, 0.0, 0.0), 10.0, 10.0, 20.0, 20.0),
            "Dot",
        )
    };
    let old = document(vec![screen(4, 1, 0.0, "FRAME"), dot(5, 4)]);
    // Same screen recreated with new ids: the dot pairs by name; the badge is new and sits
    // outside the frame, so it draws no pixel and only its + line can show it.
    let badge = named(
        filled_rectangle_at(9, 7, (0.0, 0.0, 1.0), 900.0, 10.0, 20.0, 20.0),
        "Badge",
    );
    let new = document(vec![screen(7, 1, 0.0, "FRAME"), dot(8, 7), badge]);
    for render in [false, true] {
        let mut report = diff_documents(&old, &new);
        if render {
            let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
        }
        let page = &report.pages[0];
        assert_eq!(page.screens.len(), 1, "render {render}: {page:?}");
        assert_eq!(
            page.screens[0].matched_by,
            Some(figstudio_diff::MatchedBy::NameSize)
        );
        let html = render_html(&report);
        let screen = html.split("id=\"page-0-s1\"").nth(1).unwrap();
        assert!(screen.contains(">Badge<"), "{screen}");
        assert!(!screen.contains(">Dot<"), "{screen}");
        assert_eq!(screen.matches("class=\"sg\">").count(), 1, "{screen}");
    }
    // Losing a layer shows a - line the same way.
    let mut report = diff_documents(&new, &old);
    let _ = attach_render_diffs(&new, &old, &mut report, &RenderOpts::default());
    let html = render_html(&report);
    assert!(html.contains("class=\"sg\">-</span><span class=\"hd\"><span class=\"nm\">Badge"));
}

#[test]
fn recreated_screen_does_not_list_churn_below_a_layer_that_kept_its_id() {
    let named = |mut n: Node, name: &str, kind: &str| {
        n.data.set("name", Val::Str(name.into()));
        n.data.set("type", Val::Str(kind.into()));
        Node::from_data(n.data).unwrap()
    };
    let group = |parent| {
        named(
            filled_rectangle_at(5, parent, (1.0, 1.0, 1.0), 0.0, 0.0, 200.0, 200.0),
            "Group",
            "FRAME",
        )
    };
    let dot = |id| {
        named(
            filled_rectangle_at(id, 5, (1.0, 0.0, 0.0), 10.0, 10.0, 20.0, 20.0),
            "Dot",
            "RECTANGLE",
        )
    };
    // The group keeps its id under the recreated root, so the diff lists its recreated dot
    // on its own. The dot pairs by path and is churn; the badge is a real addition.
    let old = document(vec![screen(4, 1, 0.0, "FRAME"), group(4), dot(6)]);
    let badge = named(
        filled_rectangle_at(9, 5, (0.0, 0.0, 1.0), 100.0, 10.0, 20.0, 20.0),
        "Badge",
        "RECTANGLE",
    );
    let new = document(vec![screen(7, 1, 0.0, "FRAME"), group(7), dot(8), badge]);
    for render in [false, true] {
        let mut report = diff_documents(&old, &new);
        if render {
            let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
        }
        let page = &report.pages[0];
        assert_eq!(page.screens.len(), 1, "render {render}: {page:?}");
        let html = render_html(&report);
        assert!(html.contains(">Badge<"), "{html}");
        assert!(!html.contains(">Dot<"), "render {render}: {html}");
    }
}
