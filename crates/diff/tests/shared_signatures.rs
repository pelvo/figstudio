mod support;
use figstudio_diff::{RenderOpts, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Document, Node, Val};
use support::{document, filled_rectangle_at};

fn frame(id: u32) -> Node {
    let mut n = filled_rectangle_at(id, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 400.0, 300.0);
    n.data.set("type", Val::Str("FRAME".into()));
    Node::from_data(n.data).unwrap()
}

fn compare(old: &Document, new: &Document) -> figstudio_diff::DiffReport {
    let mut report = diff_documents(old, new);
    let _ = attach_render_diffs(old, new, &mut report, &RenderOpts::default());
    report
}

#[test]
fn signatures_quantize_rgba_but_keep_direction_dimensions_and_mask() {
    for variant in ["quantized", "old", "new", "size", "mask", "reversed"] {
        let side = |new| {
            let mut nodes = Vec::new();
            for root in [2, 20] {
                nodes.push(frame(root));
                let second = root == 20;
                let reversed = second && variant == "reversed";
                let green = new != reversed;
                let shade = if second && variant == "quantized" {
                    0.98
                } else {
                    1.0
                };
                let color = if green {
                    if second && variant == "new" {
                        (1.0, 1.0, 0.0)
                    } else {
                        (0.0, shade, 0.0)
                    }
                } else if second && variant == "old" {
                    (0.0, 0.0, 1.0)
                } else {
                    (shade, 0.0, 0.0)
                };
                let width = if second && variant == "size" {
                    21.0
                } else {
                    20.0
                };
                nodes.push(filled_rectangle_at(
                    root + 1,
                    root,
                    color,
                    80.0,
                    80.0,
                    width,
                    20.0,
                ));
                if variant == "mask" {
                    nodes.push(filled_rectangle_at(
                        root + 2,
                        root,
                        (0.0, 1.0, 0.0),
                        if second { 92.0 } else { 88.0 },
                        88.0,
                        4.0,
                        4.0,
                    ));
                }
            }
            document(nodes)
        };
        let report = compare(&side(false), &side(true));
        assert_eq!(
            report.shared_changes.len(),
            usize::from(variant == "quantized"),
            "{variant}"
        );
    }
}

#[test]
fn groups_need_distinct_screens_and_sort_by_frequency_before_first_occurrence() {
    let side = |new, roots: &[u32]| {
        let mut nodes = Vec::new();
        for &root in roots {
            nodes.push(frame(root));
            for (offset, x, y, green) in [
                (1, 80.0, 80.0, false),
                (2, 240.0, 80.0, true),
                (3, 80.0, 220.0, true),
            ] {
                // Red-to-blue first occurs earlier, but green-to-yellow is more frequent.
                if root == 30 && offset != 2 {
                    continue;
                }
                let color = match (new, green) {
                    (false, false) => (1.0, 0.0, 0.0),
                    (true, false) => (0.0, 0.0, 1.0),
                    (false, true) => (0.0, 1.0, 0.0),
                    (true, true) => (1.0, 1.0, 0.0),
                };
                nodes.push(filled_rectangle_at(
                    root + offset,
                    root,
                    color,
                    x,
                    y,
                    20.0,
                    20.0,
                ));
            }
        }
        document(nodes)
    };
    let single = compare(&side(false, &[2]), &side(true, &[2]));
    assert!(single.shared_changes.is_empty());
    let report = compare(&side(false, &[2, 20, 30]), &side(true, &[2, 20, 30]));
    assert_eq!(report.shared_changes.len(), 2);
    assert_eq!(report.shared_changes[0].id, 0);
    assert_eq!(report.shared_changes[0].count, 3);
    assert_eq!(report.shared_changes[0].occurrences.len(), 5);
    assert_eq!(report.shared_changes[0].occurrences[0].region_index, 1);
    assert_eq!(report.shared_changes[1].count, 2);
    assert_eq!(report.shared_changes[1].occurrences[0].region_index, 0);
    let html = render_html(&report);
    assert_eq!(html.matches("Shared change #1</a>").count(), 3);
    assert!(html.contains("also in 2 other screens"));
    assert!(html.contains("Appears in 3 screens on 1 pages"));
}
