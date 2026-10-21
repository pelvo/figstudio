mod support;
use figstudio_diff::{
    DiffReport, RenderOpts, SHARED_IMAGES_ID, attach_render_diffs, attach_render_diffs_with_sink,
    diff_documents, render_html, write_render_images,
};
use figstudio_fig::{Document, Guid, Node, Val};
use support::{filled_rectangle_at, node, object, two_page_document};

fn frame(id: u32, width: f64, x: f64) -> Node {
    let mut n = filled_rectangle_at(id, 1, (1.0, 1.0, 1.0), x, 0.0, width, 200.0);
    n.data.set("type", Val::Str("FRAME".into()));
    Node::from_data(n.data).unwrap()
}

fn instance(id: u32, parent: u32, x: f64, y: f64, new: bool) -> Node {
    let mut n = node(id, Some(parent), "INSTANCE", "a");
    n.data.set(
        "size",
        object(&[("x", Val::Float(60.0)), ("y", Val::Float(40.0))]),
    );
    n.data.set(
        "transform",
        object(&[("m02", Val::Float(x)), ("m12", Val::Float(y))]),
    );
    n.data
        .set("symbolData", object(&[("symbolID", Guid(0, 10).to_val())]));
    // Figma re-derives an instance's cached content when its component changes.
    n.data.set(
        "derivedSymbolData",
        Val::Arr(vec![object(&[(
            "fillGeometry",
            Val::Arr(vec![object(&[("styleID", Val::Int(i64::from(new)))])]),
        )])]),
    );
    Node::from_data(n.data).unwrap()
}

/// Two screens of different widths use component 10; a cover hides half of the first
/// screen's instance, so the two changed areas are cut differently.
fn pair(old_has_master: bool) -> (Document, Document) {
    let side = |new: bool, master: bool| {
        let mut page = vec![
            frame(2, 300.0, 0.0),
            frame(3, 420.0, 500.0),
            instance(4, 2, 20.0, 20.0, new),
            instance(5, 3, 30.0, 40.0, new),
            filled_rectangle_at(6, 2, (0.0, 0.0, 1.0), 20.0, 20.0, 30.0, 40.0),
        ];
        let mut symbol = node(10, Some(11), "SYMBOL", "a");
        symbol.data.set("name", Val::Str("Nav <tab>".into()));
        symbol.data.set(
            "size",
            object(&[("x", Val::Float(60.0)), ("y", Val::Float(40.0))]),
        );
        let color = if new {
            (0.0, 1.0, 0.0)
        } else {
            (1.0, 0.0, 0.0)
        };
        let inner = filled_rectangle_at(12, 10, color, 0.0, 0.0, 60.0, 40.0);
        let second = if master {
            vec![Node::from_data(symbol.data).unwrap(), inner]
        } else {
            Vec::new()
        };
        page.sort_by_key(|n| n.id);
        two_page_document(page, second)
    };
    (side(false, old_has_master), side(true, true))
}

fn rendered(old_has_master: bool) -> (DiffReport, Document, Document) {
    let (old, new) = pair(old_has_master);
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    (report, old, new)
}

#[test]
fn one_component_edit_is_one_shared_change_with_the_component_picture() {
    let (report, old, new) = rendered(true);
    for p in &report.pages {
        eprintln!(
            "PAGE {:?} {:?} screens={} changed={} unchanged={}",
            p.id,
            p.status,
            p.screens.len(),
            p.changed.len(),
            p.screens_unchanged
        );
        for s in &p.screens {
            eprintln!(
                "  {} {:?} px={:?} regions={:?}",
                s.name,
                s.status,
                s.pixels_changed,
                s.regions
                    .iter()
                    .map(|r| (r.x, r.y, r.width, r.height, r.shared))
                    .collect::<Vec<_>>()
            );
        }
    }
    assert_eq!(
        report.shared_changes.len(),
        1,
        "{:#?}",
        report.shared_changes
    );
    let group = &report.shared_changes[0];
    let component = group.component.as_ref().unwrap();
    assert_eq!(
        (component.id, component.name.as_str()),
        (Guid(0, 10), "Nav <tab>")
    );
    assert!(component.set.is_none() && component.variants.is_empty());
    assert_eq!(group.count, 2);
    assert!(group.component_view);
    let regions: Vec<_> = report.pages[0]
        .screens
        .iter()
        .flat_map(|s| s.regions.iter())
        .collect();
    assert!(regions.len() >= 2 && regions.iter().all(|r| r.shared == Some(0)));
    // The two areas really differ, yet they are one group.
    assert_ne!(
        (regions[0].width, regions[0].height),
        (regions[1].width, regions[1].height)
    );

    let dir =
        std::env::temp_dir().join(format!("figstudio-shared-component-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("render")).unwrap();
    let mut report = diff_documents(&old, &new);
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
    assert_eq!(written.last().unwrap().0, SHARED_IMAGES_ID);
    for (id, links) in written {
        links.attach(&mut report, id);
    }
    let group = &report.shared_changes[0];
    assert_eq!(group.old_png.as_deref(), Some("render/shared-0-before.png"));
    assert_eq!(group.new_png.as_deref(), Some("render/shared-0-after.png"));
    let picture = image::open(dir.join("render/shared-0-after.png")).unwrap();
    assert_eq!(
        (Some(picture.width()), Some(picture.height())),
        (group.image_width, group.image_height)
    );
    let html = render_html(&report);
    assert!(html.contains("<h3>Component Nav &lt;tab&gt; changed</h3>"));
    assert!(html.contains("Appears in 2 screens on 1 pages"));
    // The one global switch drives shared cards too; they carry no switch of their own.
    assert_eq!(html.matches("name=\"view\"").count(), 2);
    assert!(!html.contains("id=\"m-") && html.matches("class=\"seg\"").count() == 1);
    assert_eq!(
        report,
        serde_json::from_str::<DiffReport>(&report.to_json().unwrap()).unwrap()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_old_master_uses_the_largest_region_as_the_picture() {
    let (report, ..) = rendered(false);
    let group = &report.shared_changes[0];
    assert!(group.component.is_some() && !group.component_view);
    let lead = group.occurrences[0];
    let area = |o: &figstudio_diff::SharedOccurrence| {
        let r = &report.pages[o.page_index].screens[o.screen_index].regions[o.region_index];
        u64::from(r.width) * u64::from(r.height)
    };
    assert!(group.occurrences.iter().all(|o| area(o) <= area(&lead)));
    assert!(group.occurrences.iter().any(|o| area(o) < area(&lead)));
    assert_eq!(
        group.image_width,
        report.pages[0].screens[lead.screen_index].regions[lead.region_index].image_width
    );
    assert!(render_html(&report).contains("<h3>Shared change #1</h3>"));
}

#[test]
fn legacy_shared_json_without_component_fields_parses() {
    let (report, ..) = rendered(true);
    let mut json = serde_json::to_value(&report).unwrap();
    for group in json["shared_changes"].as_array_mut().unwrap() {
        for key in ["component", "component_view", "image_width", "image_height"] {
            group.as_object_mut().unwrap().remove(key);
        }
    }
    let legacy: DiffReport = serde_json::from_value(json).unwrap();
    assert!(legacy.shared_changes[0].component.is_none());
    assert!(!legacy.shared_changes[0].component_view);
}
