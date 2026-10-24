mod support;
use figstudio_diff::{MAX_RENDER_PIXELS, RenderOpts, attach_render_diffs, diff_documents};
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at};

#[test]
fn several_sharp_crops_share_one_budget_including_both_sides() {
    let side = |rgb| {
        let mut frame = support::node(10, Some(1), "FRAME", "");
        frame.data.set(
            "size",
            support::object(&[("x", Val::Float(3000.0)), ("y", Val::Float(3000.0))]),
        );
        document(
            std::iter::once(Node::from_data(frame.data).unwrap())
                .chain((0..4).map(|i| {
                    filled_rectangle_at(
                        i + 2,
                        10,
                        rgb,
                        f64::from(i % 2) * 1500.0,
                        f64::from(i / 2) * 1500.0,
                        1000.0,
                        1000.0,
                    )
                }))
                .collect(),
        )
    };
    let (old, new) = (side((1.0, 0.0, 0.0)), side((0.0, 1.0, 0.0)));
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let crops = &images[&report.pages[0].id].screens[0].regions;
    assert_eq!(crops.len(), 4);
    let mut total = 0;
    for crop in crops {
        let before = image::load_from_memory(&crop.before).unwrap();
        let after = image::load_from_memory(&crop.after).unwrap();
        assert_eq!(
            (before.width(), before.height()),
            (after.width(), after.height())
        );
        assert!(after.width().max(after.height()) <= 2048);
        assert!(
            after.width().min(after.height()) > 1000,
            "later areas are not starved"
        );
        total += after.width() as usize * after.height() as usize * 2;
    }
    assert!(total <= MAX_RENDER_PIXELS);
    assert!(report.truncated.iter().any(|n| n.contains("crops capped")));
}
