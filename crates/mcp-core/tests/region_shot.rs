//! Region links in `get_screenshot` and `get_nodes_in_region`, on `structure.fig`.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp_core::{
    link::format_region_link,
    output::{Block, Output},
    region_shot::{region_shot, viewport},
    region_tool::nodes_in_region,
    rendering::Rendering,
    screenshot::screenshot,
};
use serde_json::Value;
use std::path::PathBuf;

const PAGE: Guid = Guid(0, 1);

fn doc() -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap()
}

fn text(out: &Output) -> String {
    out.content
        .iter()
        .filter_map(Block::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn png(out: &Output) -> Vec<u8> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    assert!(!out.is_error, "{}", text(out));
    out.content
        .iter()
        .find_map(|b| match b {
            Block::Image { data_b64, .. } => Some(STANDARD.decode(data_b64).unwrap()),
            _ => None,
        })
        .unwrap()
}

fn size(out: &Output) -> (u32, u32) {
    let img = image::load_from_memory(&png(out)).unwrap();
    (img.width(), img.height())
}

fn shot(link: &str, scale: Option<f64>) -> Output {
    screenshot(&doc(), link, scale, &Rendering::default())
}

fn link(rect: [f64; 4]) -> String {
    format_region_link("structure", PAGE, rect).unwrap()
}

#[test]
fn a_region_link_renders_the_rect_at_the_scale() {
    assert_eq!(size(&shot(&link([0.0, 0.0, 100.0, 50.0]), None)), (100, 50));
    assert_eq!(
        size(&shot(&link([0.0, 0.0, 100.0, 50.0]), Some(2.0))),
        (200, 100)
    );
}

/// What the app's Copy region as PNG draws and what the link renders are the same pixels.
#[test]
fn a_link_from_copy_region_renders_the_same_area_as_the_app_crop() {
    let doc = doc();
    let rect = [-20.5, 10.25, 300.0, 200.5];
    let (vp, _) = viewport(&doc, PAGE, rect, 1.0).unwrap();
    let app =
        Rendering::default().draw(&doc, figstudio_mcp_core::screenshot::Root::Node(PAGE), &vp);
    let from_link = png(&shot(&link(rect), None));
    let decoded = image::load_from_memory(&from_link).unwrap().to_rgba8();
    assert_eq!(decoded.dimensions(), (300, 201));
    // The PNG holds straight alpha; the page background is opaque, so the bytes agree.
    assert_eq!(decoded.as_raw().as_slice(), app.pixmap.data_as_u8_slice());
}

#[test]
fn a_region_over_the_content_is_not_blank() {
    let out = shot(&link([-200.0, -200.0, 1200.0, 1200.0]), None);
    let img = image::load_from_memory(&png(&out)).unwrap().to_rgba8();
    let first = img.get_pixel(0, 0);
    assert!(img.pixels().any(|p| p != first), "a flat image");
}

#[test]
fn huge_regions_are_capped() {
    let big = link([0.0, 0.0, 1e6, 5e5]);
    let (w, h) = size(&shot(&big, None));
    assert_eq!(w.max(h), 2048);
    let (w, h) = size(&shot(&big, Some(4.0)));
    assert!(
        w.max(h) <= 4096 && f64::from(w) * f64::from(h) <= 16_000_000.0,
        "{w}x{h}"
    );
    assert!(text(&shot(&big, None)).contains(" px, scale "));
}

#[test]
fn bad_rects_scales_and_pages_are_errors_not_panics() {
    let doc = doc();
    let r = Rendering::default();
    for rect in [
        [0.0, 0.0, 0.0, 10.0],
        [0.0, 0.0, -5.0, 10.0],
        [f64::NAN, 0.0, 10.0, 10.0],
        [0.0, 0.0, f64::INFINITY, 10.0],
        [0.0, 0.0, 2e7, 10.0],
    ] {
        let err = region_shot(&doc, PAGE, rect, None, &r).err().unwrap();
        assert!(err.contains("rect must be"), "{rect:?}: {err}");
    }
    assert!(region_shot(&doc, PAGE, [0.0, 0.0, 9.0, 9.0], Some(9.0), &r).is_err());
    assert!(region_shot(&doc, Guid(99, 99), [0.0, 0.0, 9.0, 9.0], None, &r).is_err());
    // A node that is not a page.
    assert!(region_shot(&doc, Guid(1, 34), [0.0, 0.0, 9.0, 9.0], None, &r).is_err());
    let bad_link = shot("figstudio://structure?page=0-1&rect=1,2,0,4", None);
    assert!(bad_link.is_error);
    let unknown = shot("figstudio://structure?page=77-7&rect=0,0,10,10", None);
    assert!(unknown.is_error && text(&unknown).contains("does not exist"));
}

fn json(out: &Output) -> Value {
    assert!(!out.is_error, "{}", text(out));
    out.structured.clone().unwrap()
}

fn ids(v: &Value) -> Vec<String> {
    v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["nodeId"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_nodes_come_top_level_first_with_a_total_and_a_limit() {
    let doc = doc();
    let all = json(&nodes_in_region(
        &doc,
        None,
        Some("0:1"),
        Some([-500.0, -500.0, 5000.0, 5000.0]),
        Some(500),
    ));
    let all_ids = ids(&all);
    assert!(all_ids.len() > 3, "{all_ids:?}");
    assert_eq!(all["total"], all_ids.len());
    assert_eq!(all["truncated"], false);
    assert_eq!(all["page"], "0:1");
    assert!(all["nodes"][0]["name"].is_string() && all["nodes"][0]["kind"].is_string());
    // The first rows are the page's own children.
    let top = doc
        .children(PAGE)
        .iter()
        .filter(|c| all_ids.contains(&c.to_string()))
        .count();
    assert!(
        all_ids[..top]
            .iter()
            .all(|id| doc.children(PAGE).iter().any(|c| &c.to_string() == id))
    );

    let two = json(&nodes_in_region(
        &doc,
        None,
        Some("0-1"),
        Some([-500.0, -500.0, 5000.0, 5000.0]),
        Some(2),
    ));
    assert_eq!(ids(&two), all_ids[..2]);
    assert_eq!(two["total"], all["total"]);
    assert_eq!(two["truncated"], true);
}

#[test]
fn a_region_link_and_page_rect_name_the_same_nodes() {
    let doc = doc();
    let rect = [0.0, 0.0, 400.0, 300.0];
    let by_link = json(&nodes_in_region(&doc, Some(&link(rect)), None, None, None));
    let by_rect = json(&nodes_in_region(&doc, None, Some("0:1"), Some(rect), None));
    assert_eq!(by_link, by_rect);
    assert!(!ids(&by_link).is_empty());
    assert_eq!(by_link["rect"], serde_json::json!(rect));
}

#[test]
fn bad_region_inputs_are_tool_errors() {
    let doc = doc();
    let rect = Some([0.0, 0.0, 10.0, 10.0]);
    let cases = [
        nodes_in_region(&doc, None, None, None, None),
        nodes_in_region(
            &doc,
            Some(&link([0.0, 0.0, 9.0, 9.0])),
            Some("0:1"),
            rect,
            None,
        ),
        nodes_in_region(
            &doc,
            Some("figstudio://structure?node-id=1-34"),
            None,
            None,
            None,
        ),
        nodes_in_region(&doc, Some("nonsense"), None, None, None),
        nodes_in_region(&doc, None, Some("zz"), rect, None),
        nodes_in_region(&doc, None, Some("0:1"), Some([0.0, 0.0, 0.0, 5.0]), None),
        nodes_in_region(
            &doc,
            None,
            Some("0:1"),
            Some([f64::NAN, 0.0, 5.0, 5.0]),
            None,
        ),
        nodes_in_region(&doc, None, Some("55:5"), rect, None),
        nodes_in_region(&doc, None, Some("1:34"), rect, None),
    ];
    for (i, out) in cases.iter().enumerate() {
        assert!(out.is_error, "case {i}: {}", text(out));
    }
}

#[test]
fn a_huge_limit_is_capped() {
    let out = json(&nodes_in_region(
        &doc(),
        None,
        Some("0:1"),
        Some([-1e6, -1e6, 2e6, 2e6]),
        Some(usize::MAX),
    ));
    assert!(ids(&out).len() <= 500);
    let zero = json(&nodes_in_region(
        &doc(),
        None,
        Some("0:1"),
        Some([-1e6, -1e6, 2e6, 2e6]),
        Some(0),
    ));
    assert_eq!(ids(&zero).len(), 1);
}

/// A sub-rect shows the same pixels as that part of a larger render, so the link's offset is
/// the rect's origin.
#[test]
fn a_region_is_the_matching_crop_of_a_larger_region() {
    let rgba = |rect| {
        image::load_from_memory(&png(&shot(&link(rect), None)))
            .unwrap()
            .to_rgba8()
    };
    let whole = rgba([0.0, 0.0, 400.0, 300.0]);
    let part = rgba([120.0, 70.0, 100.0, 80.0]);
    let crop = image::imageops::crop_imm(&whole, 120, 70, 100, 80).to_image();
    let flat = part.pixels().all(|p| p == part.get_pixel(0, 0));
    assert!(!flat, "pick a rect with content");
    assert_eq!(part.as_raw(), crop.as_raw());
}

/// A page of `n` small frames, so the 500-row cap can be reached.
fn crowded(n: u32) -> Document {
    let obj = |fields: Vec<(&str, Val)>| {
        let mut o = Obj::new();
        for (k, v) in fields {
            o.insert(k, v);
        }
        Val::Obj(o)
    };
    let parent =
        |g: Guid, pos: String| obj(vec![("guid", g.to_val()), ("position", Val::Str(pos))]);
    let mut nodes = vec![
        Node::from_data(obj(vec![
            ("guid", Guid(9, 1).to_val()),
            ("type", Val::Str("CANVAS".into())),
            ("name", Val::Str("Crowd".into())),
            ("parentIndex", parent(Guid(9, 0), "0".into())),
        ]))
        .unwrap(),
    ];
    for i in 0..n {
        nodes.push(
            Node::from_data(obj(vec![
                ("guid", Guid(9, 10 + i).to_val()),
                ("type", Val::Str("FRAME".into())),
                ("name", Val::Str(format!("f{i}"))),
                ("parentIndex", parent(Guid(9, 1), format!("{i:05}"))),
                (
                    "size",
                    obj(vec![("x", Val::Float(10.0)), ("y", Val::Float(10.0))]),
                ),
            ]))
            .unwrap(),
        );
    }
    let mut doc = doc();
    doc.set_nodes(nodes).unwrap();
    doc.pages = vec![Guid(9, 1)];
    doc
}

#[test]
fn a_limit_above_the_cap_lists_at_most_500_rows() {
    let doc = crowded(520);
    let rect = Some([-10.0, -10.0, 100.0, 100.0]);
    let out = json(&nodes_in_region(
        &doc,
        None,
        Some("9:1"),
        rect,
        Some(usize::MAX),
    ));
    assert_eq!(ids(&out).len(), 500);
    assert_eq!(out["total"], 520);
    assert_eq!(out["truncated"], true);
}
