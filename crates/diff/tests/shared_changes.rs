mod support;
use figstudio_diff::{DiffReport, RenderOpts, attach_render_diffs, diff_documents, render_html};
use figstudio_fig::{Document, Node, Val};
use support::{filled_rectangle_at, two_page_document};

fn screen(id: u32, page: u32, shade: f64) -> Node {
    let mut n = filled_rectangle_at(id, page, (shade, shade, shade), 0.0, 0.0, 440.0, 300.0);
    n.data.set("type", Val::Str("FRAME".into()));
    n.data.set("name", Val::Str(format!("Screen <{id}>")));
    Node::from_data(n.data).unwrap()
}

fn pair(extra: bool) -> (Document, Document) {
    let side = |new| {
        let children = |root, page, shade| {
            let mut nodes = vec![screen(root, page, shade)];
            for (offset, x, color) in [
                (
                    1,
                    80.0,
                    if new {
                        (0.0, 1.0, 0.0)
                    } else {
                        (1.0, 0.0, 0.0)
                    },
                ),
                (
                    2,
                    240.0,
                    if new {
                        (1.0, 1.0, 0.0)
                    } else {
                        (0.0, 0.0, 1.0)
                    },
                ),
            ] {
                if offset == 1 || extra {
                    nodes.push(filled_rectangle_at(
                        root + offset,
                        root,
                        color,
                        x,
                        80.0,
                        20.0,
                        20.0,
                    ));
                }
            }
            if extra && root == 20 {
                nodes.push(filled_rectangle_at(
                    23,
                    root,
                    if new {
                        (1.0, 0.0, 1.0)
                    } else {
                        (0.0, 1.0, 1.0)
                    },
                    80.0,
                    220.0,
                    20.0,
                    20.0,
                ));
            }
            nodes
        };
        two_page_document(children(2, 1, 1.0), children(20, 11, 0.5))
    };
    (side(false), side(true))
}

fn rendered(extra: bool) -> DiffReport {
    let (old, new) = pair(extra);
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    report
}

#[test]
fn repeated_change_ignores_unchanged_background_and_renders_once() {
    let report = rendered(false);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["shared_changes"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["shared_changes"][0]["count"], 2);
    for page in &json["pages"].as_array().unwrap()[..] {
        assert_eq!(page["screens"][0]["regions"][0]["shared"], 0);
    }
    let html = render_html(&report);
    assert_eq!(html.matches("Shared change #1</a>").count(), 2);
    assert!(!html.contains("class=\"area\""));
    assert!(!html.contains("class=\"mini\""));
    assert!(html.contains("Appears in 2 screens on 2 pages"));
    assert!(html.contains("href=\"#page-0-s1\""));
    assert!(html.contains("id=\"page-0-s1\""));
    assert!(html.contains("Screen &lt;2&gt;"));
    assert!(!html.contains("Screen <2>"));
    assert!(html.contains("Other changes in this screen"));
}

#[test]
fn distinct_shared_changes_keep_unrelated_area_and_order_is_deterministic() {
    let report = rendered(true);
    let json = serde_json::to_value(&report).unwrap();
    let groups = json["shared_changes"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["id"], 0);
    assert_eq!(groups[1]["id"], 1);
    assert_eq!(groups[0]["occurrences"][0]["region_index"], 0);
    assert_eq!(groups[1]["occurrences"][0]["region_index"], 1);
    let html = render_html(&report);
    assert_eq!(html.matches("class=\"area\"").count(), 1);
    assert_eq!(html.matches("Shared change #1</a>").count(), 2);
    assert_eq!(html.matches("Shared change #2</a>").count(), 2);
    assert_eq!(report, rendered(true));
    assert_eq!(html, render_html(&rendered(true)));
}

#[test]
fn grouped_region_lines_stay_in_other_changes_even_if_their_bounds_span_another_area() {
    let mut report = rendered(true);
    let screen = &mut report.pages[1].screens[0];
    let shared_node = screen.regions[0].nodes[0];
    screen
        .regions
        .iter_mut()
        .find(|r| r.shared.is_none())
        .unwrap()
        .nodes
        .push(shared_node);
    let html = render_html(&report);
    let area = html
        .split("<article class=\"area\"")
        .nth(1)
        .unwrap()
        .split("</article>")
        .next()
        .unwrap();
    assert!(!area.contains("layer-21"));
    assert!(
        html.split("Other changes in this screen</h4>")
            .last()
            .unwrap()
            .contains("layer-21")
    );
}

#[test]
fn shared_only_screens_fold_but_pages_stay_in_index() {
    let mut report = rendered(false);
    for page in &mut report.pages {
        page.changed.clear();
    }
    let html = render_html(&report);
    assert_eq!(
        html.matches("1 screens only show shared changes:").count(),
        2
    );
    assert_eq!(html.matches("shared only</span>").count(), 2);
    assert!(!html.contains("class=\"sec screen\""));
    assert!(html.contains("<section class=\"page\" id=\"page-0\""));
    assert!(html.contains("Screen &lt;2&gt;"));
    assert!(html.contains("href=\"#shared-1\""));
}

#[test]
fn shared_json_legacy_defaults_and_streamed_links_reuse_first_crop() {
    use figstudio_diff::{attach_render_diffs_with_sink, write_render_images};
    let (old, new) = pair(false);
    let mut report = diff_documents(&old, &new);
    let dir = std::env::temp_dir().join(format!("figstudio-shared-links-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("render")).unwrap();
    let mut written = Vec::new();
    attach_render_diffs_with_sink(
        &old,
        &new,
        &mut report,
        &RenderOpts::default(),
        &|_| None,
        &|_| None,
        &mut |id, images| written.push((id, write_render_images(&dir, id, images).unwrap())),
    );
    for (id, links) in written {
        links.attach(&mut report, id);
    }
    let group = &report.shared_changes[0];
    let first = group.occurrences[0];
    let region =
        &report.pages[first.page_index].screens[first.screen_index].regions[first.region_index];
    assert_eq!(group.old_png, region.old_png);
    assert_eq!(group.new_png, region.new_png);
    for link in [&group.old_png, &group.new_png] {
        let crop = image::open(dir.join(link.as_ref().unwrap())).unwrap();
        assert_eq!(
            (Some(crop.width()), Some(crop.height())),
            (region.image_width, region.image_height)
        );
        assert!(crop.width() >= 640 && crop.height() >= 320);
    }
    assert_eq!(std::fs::read_dir(dir.join("render")).unwrap().count(), 14);
    let html = render_html(&report);
    assert_eq!(html.matches("class=\"cmp\"").count(), 1);
    assert!(html.contains(&format!("src=\"./{}\"", group.old_png.as_ref().unwrap())));
    assert_eq!(
        serde_json::from_str::<DiffReport>(&report.to_json().unwrap()).unwrap(),
        report
    );
    let mut json = serde_json::to_value(&report).unwrap();
    json.as_object_mut().unwrap().remove("shared_changes");
    for page in json["pages"].as_array_mut().unwrap() {
        for screen in page["screens"].as_array_mut().unwrap() {
            for region in screen["regions"].as_array_mut().unwrap() {
                region.as_object_mut().unwrap().remove("shared");
            }
        }
    }
    let legacy: DiffReport = serde_json::from_value(json).unwrap();
    assert!(legacy.shared_changes.is_empty());
    assert!(
        legacy
            .pages
            .iter()
            .flat_map(|p| &p.screens)
            .flat_map(|s| &s.regions)
            .all(|r| r.shared.is_none())
    );
    std::fs::remove_dir_all(dir).unwrap();
}
