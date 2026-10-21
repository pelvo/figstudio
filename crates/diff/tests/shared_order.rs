mod support;
use figstudio_diff::{RenderOpts, attach_render_diffs, diff_documents};
use figstudio_fig::{Node, Val};
use support::{document, filled_rectangle_at};

#[test]
fn a_change_on_more_screens_ranks_above_one_with_more_occurrences() {
    let side = |new: bool| {
        let mut nodes = Vec::new();
        for screen in 0..5_u32 {
            let root = 10 + screen * 10;
            let mut frame = filled_rectangle_at(
                root,
                1,
                (1.0, 1.0, 1.0),
                f64::from(screen) * 500.0,
                0.0,
                400.0,
                300.0,
            );
            frame.data.set("type", Val::Str("FRAME".into()));
            nodes.push(Node::from_data(frame.data).unwrap());
            let (b_from, b_to) = ((0.0, 0.0, 1.0), (1.0, 1.0, 0.0));
            nodes.push(filled_rectangle_at(
                root + 1,
                root,
                if new { b_to } else { b_from },
                60.0,
                200.0,
                24.0,
                24.0,
            ));
            // Three 20x20 areas, but only on the first two screens.
            if screen < 2 {
                for k in 0..3_u32 {
                    nodes.push(filled_rectangle_at(
                        root + 2 + k,
                        root,
                        if new {
                            (0.0, 1.0, 0.0)
                        } else {
                            (1.0, 0.0, 0.0)
                        },
                        60.0 + f64::from(k) * 120.0,
                        40.0,
                        20.0,
                        20.0,
                    ));
                }
            }
        }
        document(nodes)
    };
    let (old, new) = (side(false), side(true));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let counts: Vec<_> = report
        .shared_changes
        .iter()
        .map(|s| (s.count, s.occurrences.len()))
        .collect();
    assert_eq!(counts, [(5, 5), (2, 6)]);
}
