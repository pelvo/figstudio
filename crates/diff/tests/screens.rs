mod support;
use figstudio_diff::{RenderOpts, ScreenStatus, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Document, Guid, Node, Val};
use support::{document, filled_rectangle_at, named, node, object, typed};

fn screen(id: u32, parent: u32, x: f64, kind: &str) -> Node {
    let n = filled_rectangle_at(id, parent, (1.0, 1.0, 1.0), x, 0.0, 400.0, 300.0);
    named(typed(n, kind), "Screen")
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
    let named = |n: Node, name: &str, kind: &str| named(typed(n, kind), name);
    let row = |id, parent, y| {
        named(
            filled_rectangle_at(id, parent, (1.0, 1.0, 1.0), 0.0, y, 200.0, 100.0),
            "Row",
            "FRAME",
        )
    };
    let dot = |id, parent| {
        named(
            filled_rectangle_at(id, parent, (1.0, 0.0, 0.0), 10.0, 10.0, 20.0, 20.0),
            "Dot",
            "RECTANGLE",
        )
    };
    let badge = named(
        filled_rectangle_at(9, 5, (0.0, 0.0, 1.0), 100.0, 10.0, 20.0, 20.0),
        "Badge",
        "RECTANGLE",
    );
    // Two rows keep their ids under the recreated root, so the diff lists their recreated
    // children on their own. Row 5's dot pairs and is churn; the badge is a real addition.
    // Row 60 lost a dot and the same-named row 50 gained one: both are real.
    let old = document(vec![
        screen(4, 1, 0.0, "FRAME"),
        row(5, 4, 0.0),
        dot(6, 5),
        row(50, 4, 100.0),
        row(60, 4, 200.0),
        dot(61, 60),
    ]);
    let new = document(vec![
        screen(7, 1, 0.0, "FRAME"),
        row(5, 7, 0.0),
        dot(8, 5),
        badge,
        row(50, 7, 100.0),
        dot(51, 50),
        row(60, 7, 200.0),
    ]);
    for render in [false, true] {
        let mut report = diff_documents(&old, &new);
        if render {
            let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
        }
        let page = &report.pages[0];
        assert_eq!(page.screens.len(), 1, "render {render}: {page:?}");
        let ids = |list: &[figstudio_diff::NodeRef]| {
            let mut ids: Vec<_> = list.iter().map(|n| n.id.1).collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(ids(&page.added), vec![7, 9, 51], "render {render}");
        assert_eq!(ids(&page.removed), vec![4, 61], "render {render}");
        let html = render_html(&report);
        let screen = html.split("id=\"page-0-s1\"").nth(1).unwrap();
        assert!(screen.contains(">Badge<"), "{screen}");
        assert_eq!(
            screen.matches(">Dot<").count(),
            2,
            "render {render}: {screen}"
        );
        assert!(!screen.contains("class=\"ct\""), "{screen}");
        assert_eq!(screen.matches("class=\"sg\">").count(), 3, "{screen}");
    }
}

#[test]
fn recreated_screen_lists_edits_of_layers_that_pair_by_name() {
    let dot = |id, parent, rgb| {
        named(
            filled_rectangle_at(id, parent, rgb, 10.0, 10.0, 20.0, 20.0),
            "Dot",
        )
    };
    let label = |id, parent, text: &str| {
        let mut n = node(id, Some(parent), "TEXT", "b");
        n.data.set("name", Val::Str("Label".into()));
        n.data
            .set("textData", object(&[("characters", Val::Str(text.into()))]));
        Node::from_data(n.data).unwrap()
    };
    let tinted = |mut n: Node| {
        n.data.set("fillPaints", support::solid(0.9, 0.9, 0.9));
        Node::from_data(n.data).unwrap()
    };
    // The screen is recreated with new ids and moved on the canvas. Its own fill, its dot's
    // colour and its label's text change; each pairs by name, so each is a change.
    let old = document(vec![
        screen(4, 1, 0.0, "FRAME"),
        dot(5, 4, (1.0, 0.0, 0.0)),
        label(6, 4, "Old"),
    ]);
    let new = document(vec![
        tinted(screen(7, 1, 600.0, "FRAME")),
        dot(8, 7, (0.0, 0.0, 1.0)),
        label(9, 7, "New"),
    ]);
    for render in [false, true] {
        let mut report = diff_documents(&old, &new);
        if render {
            let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
        }
        let page = &report.pages[0];
        assert_eq!(page.screens.len(), 1, "render {render}: {page:?}");
        let changed = |id| page.changed.iter().find(|n| n.id == Guid(0, id));
        let fields = |id| -> Vec<String> {
            changed(id).map_or_else(Vec::new, |n| {
                n.changes.iter().map(|c| c.field.clone()).collect()
            })
        };
        assert_eq!(fields(8), ["fillPaints.0.color.b", "fillPaints.0.color.r"]);
        let red = &changed(8).unwrap().changes[1];
        assert_eq!(
            (&red.old, &red.new),
            (&Some(Val::Float(1.0)), &Some(Val::Float(0.0)))
        );
        assert!(changed(9).unwrap().text.is_some(), "{page:?}");
        // The root's fill is an edit; where it sits on the canvas is not.
        assert!(
            fields(7).iter().all(|f| f.starts_with("fillPaints")),
            "{:?}",
            fields(7)
        );
        assert!(!fields(7).is_empty());
        for n in &page.changed {
            for c in &n.changes {
                let head = c.field.split('.').next().unwrap();
                assert!(
                    !["guid", "parentIndex", "overrideKey"].contains(&head),
                    "{c:?}"
                );
            }
        }
        assert_eq!(report.summary.nodes_changed, 3);
        assert_eq!(report.summary.properties_changed, 6);
        let html = render_html(&report);
        let screen = html.split("id=\"page-0-s1\"").nth(1).unwrap();
        assert!(screen.contains(">Dot<"), "{screen}");
    }
}

#[test]
fn a_recreated_screen_that_is_only_rotated_or_scaled_is_a_move_not_an_edit() {
    // A quarter turn and a 2x scale, each in place (no translation).
    for (m00, m01, m10, m11) in [(0.0, -1.0, 1.0, 0.0), (2.0, 0.0, 0.0, 2.0)] {
        let mut root = screen(7, 1, 0.0, "FRAME");
        root.data.set(
            "transform",
            object(&[
                ("m00", Val::Float(m00)),
                ("m01", Val::Float(m01)),
                ("m02", Val::Float(0.0)),
                ("m10", Val::Float(m10)),
                ("m11", Val::Float(m11)),
                ("m12", Val::Float(0.0)),
            ]),
        );
        // A small mark inside, so a leaked scale or turn would move it on the screen picture.
        let mark = |id, parent| {
            let mut n = filled_rectangle_at(id, parent, (0.0, 0.0, 0.0), 20.0, 20.0, 40.0, 30.0);
            n.data.set("name", Val::Str("Mark".into()));
            Node::from_data(n.data).unwrap()
        };
        let old = document(vec![screen(4, 1, 0.0, "FRAME"), mark(5, 4)]);
        let new = document(vec![Node::from_data(root.data).unwrap(), mark(8, 7)]);
        for render in [false, true] {
            let mut report = diff_documents(&old, &new);
            if render {
                let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
                assert!(report.pages[0].render.is_some());
            }
            let page = &report.pages[0];
            assert!(page.changed.iter().all(|n| n.id != Guid(0, 7)), "{page:?}");
            assert!(page.screens.is_empty(), "{m00} render {render}: {page:?}");
            assert_eq!((page.screens_unchanged, page.screens_moved), (1, 1));
            let summary = &report.summary;
            assert_eq!(
                (summary.nodes_changed, summary.properties_changed),
                (0, 0),
                "{summary:?}"
            );
        }
    }
}

#[test]
fn recreated_screen_keeps_repeated_names_silent_when_a_copy_is_inserted() {
    let card = |id, parent, y, text: &str, position: &str| {
        let mut frame = node(id, Some(parent), "FRAME", position);
        frame.data.set("name", Val::Str("Card".into()));
        frame
            .data
            .set("transform", object(&[("m12", Val::Float(y))]));
        let mut title = node(id + 1, Some(id), "TEXT", "a");
        title.data.set("name", Val::Str("Title".into()));
        title
            .data
            .set("textData", object(&[("characters", Val::Str(text.into()))]));
        [
            Node::from_data(frame.data).unwrap(),
            Node::from_data(title.data).unwrap(),
        ]
    };
    // A new card goes in front of the old one: walk order would pair the new card's title
    // with the old title and report a text edit that did not happen.
    let mut old = vec![screen(4, 1, 0.0, "FRAME")];
    old.extend(card(5, 4, 100.0, "Kept", "b"));
    let mut new = vec![screen(7, 1, 0.0, "FRAME")];
    new.extend(card(20, 7, 0.0, "Inserted", "a"));
    new.extend(card(8, 7, 100.0, "Kept", "b"));
    let report = diff_documents(&document(old), &document(new));
    let page = &report.pages[0];
    assert!(page.changed.is_empty(), "{:?}", page.changed);
    assert_eq!(report.summary.nodes_changed, 0);
    // The unchanged copy pairs with the old card, so the inserted one is the addition.
    let ids = |list: &[figstudio_diff::NodeRef]| list.iter().map(|n| n.id.1).collect::<Vec<_>>();
    assert_eq!(ids(&page.added), [7, 20], "{:?}", page.added);
    assert_eq!(ids(&page.removed), [4], "{:?}", page.removed);
}

#[test]
fn recreated_screen_lists_an_edit_of_a_repeated_name_in_tree_order() {
    let card = |id, parent, y, text: &str, position: &str| {
        let mut frame = node(id, Some(parent), "FRAME", position);
        frame.data.set("name", Val::Str("Card".into()));
        frame
            .data
            .set("transform", object(&[("m12", Val::Float(y))]));
        let mut title = node(id + 1, Some(id), "TEXT", "a");
        title.data.set("name", Val::Str("Title".into()));
        title
            .data
            .set("textData", object(&[("characters", Val::Str(text.into()))]));
        [
            Node::from_data(frame.data).unwrap(),
            Node::from_data(title.data).unwrap(),
        ]
    };
    let label = |id, parent, name: &str, text: &str, position: &str| {
        let mut n = node(id, Some(parent), "TEXT", position);
        n.data.set("name", Val::Str(name.into()));
        n.data
            .set("textData", object(&[("characters", Val::Str(text.into()))]));
        Node::from_data(n.data).unwrap()
    };
    // Two cards on each side, in the same order; only the second title changes. Zebra is
    // the earlier sibling of Apple, and both change.
    let mut old = vec![screen(4, 1, 0.0, "FRAME")];
    old.extend(card(5, 4, 0.0, "One", "a"));
    old.extend(card(10, 4, 100.0, "Two", "b"));
    old.push(label(12, 4, "Zebra", "z", "c"));
    old.push(label(13, 4, "Apple", "a", "d"));
    let mut new = vec![screen(7, 1, 0.0, "FRAME")];
    new.extend(card(20, 7, 0.0, "One", "a"));
    new.extend(card(30, 7, 100.0, "Two edited", "b"));
    new.push(label(32, 7, "Zebra", "z2", "c"));
    new.push(label(33, 7, "Apple", "a2", "d"));
    let report = diff_documents(&document(old), &document(new));
    let page = &report.pages[0];
    let rows: Vec<_> = page.changed.iter().map(|n| n.id.1).collect();
    assert_eq!(rows, [31, 32, 33], "{:?}", page.changed);
    assert_eq!(page.screens.len(), 1, "{page:?}");
    assert!(render_html(&report).contains("Two edited"));
    assert!(page.added.iter().all(|n| n.id.1 == 7), "{:?}", page.added);
}

#[test]
fn repeated_copies_share_one_comparison_budget_per_recreated_screen() {
    let label = |id, parent, name: &str, text: &str| {
        let mut n = node(id, Some(parent), "TEXT", "a");
        n.data.set("name", Val::Str(name.into()));
        n.data
            .set("textData", object(&[("characters", Val::Str(text.into()))]));
        Node::from_data(n.data).unwrap()
    };
    // 50 copies of "Alpha" and of "Beta" on each side: 2 × 2,500 comparisons is past the
    // screen's 4,096, so Alpha (first in path order) is compared and Beta pairs in walk
    // order without being compared. Copy 20 of each, and the one "Only" layer, is edited.
    let side = |new: bool| {
        let (root, base) = if new { (7, 20_000) } else { (4, 10_000) };
        let edit = |text: &str| {
            if new {
                format!("{text} edited")
            } else {
                text.to_owned()
            }
        };
        let mut nodes = vec![screen(root, 1, 0.0, "FRAME")];
        for (k, name) in [(0, "Alpha"), (100, "Beta")] {
            for i in 0..50 {
                let text = if i == 20 { edit(name) } else { name.to_owned() };
                nodes.push(label(base + k + i, root, name, &text));
            }
        }
        nodes.push(label(base + 999, root, "Only", &edit("Only")));
        document(nodes)
    };
    let report = diff_documents(&side(false), &side(true));
    let rows: Vec<_> = report.pages[0].changed.iter().map(|n| n.id.1).collect();
    // A unique name is still compared once the budget is spent.
    assert_eq!(rows, [20_020, 20_999], "{:?}", report.pages[0].changed);
}
