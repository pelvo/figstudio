//! `get_motion_context` on the public Figma Motion playground fixture
//! (`tests/fixtures/motion.fig`, a Figma Community file, images removed).
use figstudio_fig::Document;
use figstudio_mcp_core::motion::motion_context;
use serde_json::{Value, json};
use std::path::PathBuf;

fn doc() -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/motion.fig"),
    )
    .unwrap()
}

fn motion(doc: &Document, id: &str, recursive: bool) -> Value {
    let out = motion_context(doc, id, recursive, 100_000);
    assert!(!out.is_error, "{:?}", out.content);
    out.structured.unwrap()
}

fn node<'a>(out: &'a Value, id: &str) -> &'a Value {
    out["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["nodeId"] == id)
        .unwrap_or_else(|| panic!("{id} not in {out}"))
}

fn binding<'a>(node: &'a Value, field: &str) -> &'a Value {
    node["keyframeBindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["field"] == field)
        .unwrap_or_else(|| panic!("{field} not in {node}"))
}

fn times(binding: &Value) -> Vec<f64> {
    binding["keyframes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["timeMs"].as_f64().unwrap())
        .collect()
}

#[test]
fn preset_tracks_scale_the_base_value_after_the_preset_delay() {
    let out = motion(&doc(), "1:748", true);
    assert_eq!(out["total"], 3);
    assert_eq!(
        out["timelineCohorts"],
        json!([{"rootNodeId": "1:748", "durationMs": 1500.0, "loopMode": "once",
                "memberNodeIds": ["1:749", "1:750", "1:751"]}])
    );
    let shape = node(&out, "1:749");
    let opacity = binding(shape, "OPACITY");
    assert_eq!(opacity["operation"], "SCALE");
    assert_eq!(opacity["preset"], "opacity");
    // Base opacity 100 times the preset's 0 → 1, starting 100 ms into the timeline.
    assert_eq!(times(opacity), [100.0, 700.0]);
    assert_eq!(opacity["keyframes"][0]["value"], 0.0);
    assert_eq!(opacity["keyframes"][1]["value"], 100.0);
    assert_eq!(
        opacity["keyframes"][1]["easing"]["cubicBezier"],
        json!([0.0, 1.078555, 1.0, 1.0])
    );
    let css = shape["codeSnippets"]["css"].as_str().unwrap();
    assert!(
        css.contains(
            "6.6667% { opacity: 0; animation-timing-function: cubic-bezier(0, 1.0786, 1, 1); }"
        ),
        "{css}"
    );
    assert!(css.contains("46.6667% { opacity: 1; }"), "{css}");
    assert!(css.contains("0% { scale: 0.3 0.3;"), "{css}");
    assert!(
        css.contains("animation: n1-749-opacity 1500ms both,"),
        "{css}"
    );
    assert_eq!(css.matches('{').count(), css.matches('}').count(), "{css}");
    let js = shape["codeSnippets"]["motion"].as_str().unwrap();
    assert!(js.contains("opacity: [0, 0, 1, 1],"), "{js}");
    assert!(
        js.contains("opacity: { times: [0, 0.0667, 0.4667, 1]"),
        "{js}"
    );
}

#[test]
fn a_node_without_keyframes_points_to_recursive() {
    let out = motion(&doc(), "1:748", false);
    assert_eq!(out["nodes"], json!([]));
    let notes = out["notes"].to_string();
    assert!(notes.contains("recursive: true"), "{notes}");
}

#[test]
fn instance_content_plays_on_the_instance_with_its_offset() {
    let out = motion(&doc(), "1:754", true);
    assert_eq!(out["timelineCohorts"][0]["rootNodeId"], "1:754");
    let shape = node(&out, "I1:754;1:749");
    // The preset's 100 ms plus the instance's 143.186 ms timeline offset.
    assert_eq!(times(binding(shape, "OPACITY")), [243.186, 843.186]);
    // An instance path works as the node ID too.
    let direct = motion(&doc(), "I1:754;1:749", false);
    assert_eq!(
        node(&direct, "I1:754;1:749")["keyframeBindings"],
        shape["keyframeBindings"]
    );
}

#[test]
fn keyframes_are_in_time_order_and_manual_tracks_set_values() {
    // Stored out of order (0.39 s, 0 s, 3.2 s, 2.7 s).
    let out = motion(&doc(), "1:1159", false);
    let width = binding(node(&out, "1:1159"), "WIDTH");
    assert_eq!(width["operation"], "SET");
    assert_eq!(times(width), [0.0, 391.148, 2719.271, 3226.538]);
    assert_eq!(width["keyframes"][0]["value"], 160.5);
    // Without a stored timeline the top-level frame plays it, as long as its last keyframe.
    assert_eq!(out["timelineCohorts"][0]["durationMs"], 3226.538);
}

#[test]
fn a_stored_spring_becomes_a_sampled_css_curve() {
    let out = motion(&doc(), "1:1041", false);
    let n = node(&out, "1:1041");
    let scale = binding(n, "MOTION_SCALE_XY");
    assert_eq!(
        scale["keyframes"][1]["easing"]["spring"],
        json!({"stiffness": 100.0, "damping": 6.519115, "mass": 1.0})
    );
    let css = n["codeSnippets"]["css"].as_str().unwrap();
    let curve = css.split("linear(").nth(1).expect("a linear() easing");
    // An underdamped spring overshoots 1 before it settles.
    let peak = curve
        .split(')')
        .next()
        .unwrap()
        .split(", ")
        .map(|v| v.parse::<f64>().unwrap())
        .fold(0.0, f64::max);
    assert!(peak > 1.2, "{curve}");
    assert_eq!(n["timelineRootId"], "1:1027");
    assert!(
        out["notes"]
            .to_string()
            .contains("sampled into CSS linear()")
    );
}

#[test]
fn rotation_is_flipped_to_clockwise_degrees() {
    let out = motion(&doc(), "1:1106", false);
    let css = node(&out, "1:1106")["codeSnippets"]["css"]
        .as_str()
        .unwrap();
    // OFFSET track from -2π to 0 radians, counter-clockwise positive.
    assert!(css.contains("rotate: 360deg;"), "{css}");
    assert!(out["notes"].to_string().contains("counter-clockwise"));
}

#[test]
fn a_page_lists_every_animated_layer_and_stops_at_the_byte_budget() {
    let doc = doc();
    let page = doc.pages[0].to_string();
    let all = motion(&doc, &page, true);
    assert_eq!(all["total"], 30);
    assert_eq!(all["truncated"], false);
    let small = motion_context(&doc, &page, true, 8_000).structured.unwrap();
    assert_eq!(small["truncated"], true);
    let kept = small["nodes"].as_array().unwrap().len();
    assert!((1..30).contains(&kept), "{kept}");
}

#[test]
fn bad_targets_are_tool_errors() {
    let doc = doc();
    assert!(motion_context(&doc, "9:9999", false, 100_000).is_error);
    assert!(motion_context(&doc, "0:0", false, 100_000).is_error);
}
