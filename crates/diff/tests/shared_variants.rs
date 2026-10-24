mod support;
use figstudio_diff::{
    DiffReport, RenderOpts, SHARED_IMAGES_ID, attach_render_diffs_with_sink, diff_documents,
    render_html, write_render_images,
};
use figstudio_fig::{Document, Guid, Node, Val};
use support::{filled_rectangle_at, node, object, two_page_document};

fn instance(id: u32, parent: u32, symbol: u32, x: f64, new: bool) -> Node {
    let mut n = node(id, Some(parent), "INSTANCE", "a");
    n.data.set(
        "size",
        object(&[("x", Val::Float(60.0)), ("y", Val::Float(40.0))]),
    );
    n.data.set(
        "transform",
        object(&[("m02", Val::Float(x)), ("m12", Val::Float(20.0))]),
    );
    n.data.set(
        "symbolData",
        object(&[("symbolID", Guid(0, symbol).to_val())]),
    );
    n.data.set(
        "derivedSymbolData",
        Val::Arr(vec![object(&[(
            "fillGeometry",
            Val::Arr(vec![object(&[("styleID", Val::Int(i64::from(new)))])]),
        )])]),
    );
    Node::from_data(n.data).unwrap()
}

fn symbol(id: u32, parent: u32, name: &str, new: bool, blue: bool) -> Vec<Node> {
    let mut s = node(id, Some(parent), "SYMBOL", "a");
    s.data.set("name", Val::Str(name.into()));
    s.data.set(
        "size",
        object(&[("x", Val::Float(60.0)), ("y", Val::Float(40.0))]),
    );
    let color = match (new, blue) {
        (false, _) => (1.0, 0.0, 0.0),
        (true, false) => (0.0, 1.0, 0.0),
        (true, true) => (0.0, 0.0, 1.0),
    };
    vec![
        Node::from_data(s.data).unwrap(),
        filled_rectangle_at(id + 20, id, color, 0.0, 0.0, 60.0, 40.0),
    ]
}

/// Component set 20 with variants A (21), B (22), C (23); A and B change alike, C differently.
/// Each variant is used on two of six screens. `in_set: false` puts the symbols on the page.
fn pair(in_set: bool) -> (Document, Document) {
    let side = |new: bool| {
        let mut page = Vec::new();
        for i in 0..6_u32 {
            let mut f = filled_rectangle_at(
                2 + i,
                1,
                (1.0, 1.0, 1.0),
                400.0 * f64::from(i),
                0.0,
                300.0,
                100.0,
            );
            f.data.set("type", Val::Str("FRAME".into()));
            page.push(Node::from_data(f.data).unwrap());
            page.push(instance(30 + i, 2 + i, 21 + i / 2, 20.0, new));
        }
        let mut second = Vec::new();
        let mut set = node(20, Some(11), "FRAME", "a");
        set.data.set("name", Val::Str("Header".into()));
        set.data.set("isStateGroup", Val::Bool(true));
        let parent = if in_set { 20 } else { 11 };
        if in_set {
            second.push(Node::from_data(set.data).unwrap());
        }
        for (k, name) in ["Kind=A", "Kind=B", "Kind=C"].into_iter().enumerate() {
            second.extend(symbol(21 + k as u32, parent, name, new, k == 2));
        }
        two_page_document(page, second)
    };
    (side(false), side(true))
}

fn rendered(in_set: bool) -> (DiffReport, std::path::PathBuf) {
    let (old, new) = pair(in_set);
    let mut report = diff_documents(&old, &new);
    let dir = std::env::temp_dir().join(format!(
        "figstudio-variants-{}-{in_set}",
        std::process::id()
    ));
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
    assert_eq!(written.last().unwrap().0, SHARED_IMAGES_ID);
    for (id, links) in written {
        links.attach(&mut report, id);
    }
    (report, dir)
}

#[test]
fn variants_of_one_component_set_are_one_shared_change() {
    let (report, dir) = rendered(true);
    assert_eq!(
        report.shared_changes.len(),
        1,
        "{:#?}",
        report.shared_changes
    );
    let group = &report.shared_changes[0];
    assert_eq!(group.count, 6);
    let component = group.component.as_ref().unwrap();
    let set = component.set.as_ref().unwrap();
    assert_eq!((set.id, set.name.as_str()), (Guid(0, 20), "Header"));
    // all variants have 2 screens: the tie goes to the first in document order
    assert_eq!(component.id, Guid(0, 21));
    let names: Vec<_> = component
        .variants
        .iter()
        .map(|v| (v.id.1, v.count))
        .collect();
    assert_eq!(names, [(21, 2), (22, 2), (23, 2)]);
    // B looks like A, so only C is repeated under "Other variants"
    assert!(component.variants[1].old_png.is_none());
    let c = &component.variants[2];
    assert_eq!(c.old_png.as_deref(), Some("render/shared-0-v2-before.png"));
    assert!(dir.join("render/shared-0-v2-after.png").exists());
    assert!(!dir.join("render/shared-0-v1-after.png").exists());
    assert!(c.image_width.is_some());
    let html = render_html(&report);
    assert!(
        html.contains("<h3>Component Header changed</h3><span class=\"none\">3 variants</span>")
    );
    assert_eq!(html.matches("<summary>Other variants</summary>").count(), 1);
    assert!(html.contains("render/shared-0-v2-before.png") && !html.contains("shared-0-v1-"));
    assert_eq!(
        report,
        serde_json::from_str::<DiffReport>(&report.to_json().unwrap()).unwrap()
    );
    // determinism
    assert_eq!(html, render_html(&rendered(true).0));
    // old JSON without the new fields parses
    let mut json = serde_json::to_value(&report).unwrap();
    let c = json["shared_changes"][0]["component"]
        .as_object_mut()
        .unwrap();
    c.remove("set");
    c.remove("variants");
    let legacy: DiffReport = serde_json::from_value(json).unwrap();
    assert!(
        legacy.shared_changes[0]
            .component
            .as_ref()
            .unwrap()
            .set
            .is_none()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn masters_outside_a_set_stay_one_group_each() {
    let (report, dir) = rendered(false);
    // two screens per master: three groups, none with a set
    // (the two identical-looking symbols themselves also form one pixel-signature group)
    let groups: Vec<_> = report
        .shared_changes
        .iter()
        .filter(|g| g.component.is_some())
        .collect();
    assert_eq!(groups.len(), 3);
    for g in groups {
        let c = g.component.as_ref().unwrap();
        assert!(c.set.is_none() && c.variants.is_empty());
        assert_eq!(g.count, 2);
    }
    assert!(!render_html(&report).contains("Other variants"));
    std::fs::remove_dir_all(dir).unwrap();
}
