//! `get_motion_context` on the public Figma Motion playground fixture
//! (`tests/fixtures/motion.fig`, a Figma Community file, images removed).
use figstudio_fig::Document;
use figstudio_mcp_core::motion::{has_motion, motion_context, node_css};
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
    // Figma's public easing object, not the file's internal CUSTOM_CUBIC.
    assert_eq!(
        opacity["keyframes"][1]["easing"],
        json!({"type": "CUSTOM_CUBIC_BEZIER",
               "easingFunctionCubicBezier": {"x1": 0.0, "y1": 1.078555, "x2": 1.0, "y2": 1.0}})
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
    // Figma's snippet shape: a motion element with initial, animate and transition.
    let js = shape["codeSnippets"]["motion"].as_str().unwrap();
    assert!(js.starts_with("<motion.svg\n"), "{js}");
    assert!(
        js.contains("initial={{ opacity: 0, scaleX: 0.3, scaleY: 0.3 }}"),
        "{js}"
    );
    assert!(js.contains("animate={{ opacity: [0, 0, 1, 1], "), "{js}");
    assert!(
        js.contains("opacity: { duration: 1.5, times: [0, 0.0667, 0.4667, 1], ease: [\"linear\", [0, 1.0786, 1, 1], \"linear\"] },"),
        "{js}"
    );
    assert!(js.ends_with("/>\n"), "{js}");
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
    // Read back as Figma does: bounce only (one minus the damping ratio 0.326).
    assert_eq!(
        scale["keyframes"][1]["easing"],
        json!({"type": "CUSTOM_SPRING", "easingFunctionSpring": {"bounce": 0.674044}})
    );
    // The spring as Figma's own get_motion_context writes it for this layer:
    // 1 - exp(-t·7.204)·(cos(t·20.8941) + 0.3448·sin(t·20.8941)).
    let js = n["codeSnippets"]["motion"].as_str().unwrap();
    assert!(
        js.contains("scaleX: { duration: 3.601, times: [0, 0.2777, 1], ease: [(t) => "),
        "{js}"
    );
    assert_spring(js, "scaleX", (7.204, 20.8941, 0.3448));
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
fn rotation_turns_clockwise_as_figma_shows_it() {
    let out = motion(&doc(), "1:1106", false);
    let snippets = &node(&out, "1:1106")["codeSnippets"];
    let css = snippets["css"].as_str().unwrap();
    // OFFSET track from -2π to 0 radians. Figma's Motion panel calls this preset
    // "Clockwise 360°"; CSS degrees are clockwise positive too, so the sign is kept.
    assert!(css.contains("rotate: -360deg;"), "{css}");
    assert!(css.contains("rotate: 0deg;"), "{css}");
    let js = snippets["motion"].as_str().unwrap();
    assert!(js.contains("rotate: [-360, -360, 0, 0] }}"), "{js}");
    // Figma's named curve by motion.dev's name.
    assert!(js.contains("ease: [\"easeOut\", \"linear\"]"), "{js}");
    assert_eq!(
        binding(node(&out, "1:1106"), "MOTION_TRANSLATION_X")["keyframes"][1]["easing"],
        json!({"type": "EASE_OUT"})
    );
    assert!(out["notes"].to_string().contains("clockwise positive"));
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

#[test]
fn the_copy_menu_helpers_see_only_the_node_itself() {
    let doc = doc();
    let data = |id: &str| &doc.node(id.parse().unwrap()).unwrap().data;
    assert!(has_motion(data("1:749")));
    // The symbol holds the timeline, but its own fields are not animated.
    assert!(!has_motion(data("1:748")));
    let css = node_css(&doc, "1:749").expect("CSS");
    assert!(css.starts_with("/* 1:749"), "{css}");
    assert!(css.contains("@keyframes n1-749-opacity"), "{css}");
    assert_eq!(node_css(&doc, "1:748"), None);
}

/// The motion fixture with `id`'s data changed by `patch`.
fn patched(id: &str, patch: impl Fn(&mut figstudio_fig::Val)) -> Document {
    let mut doc = doc();
    let id: figstudio_fig::Guid = id.parse().unwrap();
    let mut nodes = doc.nodes.clone();
    for node in &mut nodes {
        if node.id == id {
            patch(&mut node.data);
        }
    }
    doc.set_nodes(nodes).unwrap();
    doc
}

fn val(v: Value) -> figstudio_fig::Val {
    serde_json::from_value(v).unwrap()
}

#[test]
fn pivot_text_stagger_and_static_rotation_reach_the_snippets() {
    // The fixture has none of these; the field shapes come from the file's schema.
    let doc = patched("1:1106", |data| {
        data.set(
            "transformOrigin",
            val(
                json!({"x": {"value": 1.0, "units": "RAW"}, "y": {"value": 3.098, "units": "RAW"}}),
            ),
        );
        data.set(
            "textElementDelayBehaviour",
            val(json!({"enabled": true, "grouping": "WORD", "delayMs": 40.0, "order": "CENTER_OUT"})),
        );
        // A static 45° rotation (m10 = sin 45°): Figma's rotate is absolute.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        data.set(
            "transform",
            val(json!({"m00": s, "m01": -s, "m02": 100.0, "m10": s, "m11": s, "m12": 150.0})),
        );
    });
    let out = motion(&doc, "1:1106", false);
    let n = node(&out, "1:1106");
    assert_eq!(n["transformOrigin"], "100% 309.8%");
    assert_eq!(n["textAnimation"]["grouping"], "WORD");
    assert_eq!(n["textAnimation"]["order"], "CENTER_OUT");
    let js = n["codeSnippets"]["motion"].as_str().unwrap();
    assert!(
        js.contains("style={{ transformOrigin: \"100% 309.8%\" }}"),
        "{js}"
    );
    assert!(js.contains("rotate: [-315, -315, 45, 45]"), "{js}");
    let css = n["codeSnippets"]["css"].as_str().unwrap();
    assert!(css.contains("transform-origin: 100% 309.8%;"), "{css}");
}

#[test]
fn a_path_trim_track_draws_the_stroke() {
    // Re-point the rotation track at PATH_TRIM_END: 0 → 1 of the path (schema field).
    let doc = patched("1:1106", |data| {
        let mut text = serde_json::to_string(&*data).unwrap();
        text = text.replace("\"MOTION_ROTATION\"", "\"PATH_TRIM_END\"");
        *data = serde_json::from_str(&text).unwrap();
    });
    let js = node(&motion(&doc, "1:1106", false), "1:1106")["codeSnippets"]["motion"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(js.starts_with("<motion.path\n  pathLength={1}\n"), "{js}");
    assert!(js.contains("strokeDasharray: "), "{js}");
}

#[test]
fn a_recursive_reply_names_the_first_node_for_repeated_motion() {
    let out = motion(&doc(), "1:754", true);
    let snippets: Vec<_> = out["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["codeSnippets"]["motion"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        snippets.iter().all(|s| !s.starts_with("// Same motion as")),
        "different delays"
    );
    let page = motion(&doc(), &doc().pages[0].to_string(), true);
    let repeats = page["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| {
            n["codeSnippets"]["motion"]
                .as_str()
                .unwrap()
                .starts_with("// Same motion as ")
        })
        .count();
    assert!(repeats > 0);
    assert!(
        page["notes"]
            .to_string()
            .contains("repeat an earlier node's")
    );
}

#[test]
fn a_spring_preset_and_a_looping_timeline_match_figmas_output() {
    let out = motion(&doc(), "1:819", true);
    let n = node(&out, "1:820");
    // Figma's get_motion_context for this layer (preset SPRING_PRESET_TWO, BOUNCY in its
    // panel, 0.6 s segment): 1 - exp(-t·7.1866)·(cos(t·22.3441) + 0.3216·sin(t·22.3441)).
    let js = n["codeSnippets"]["motion"].as_str().unwrap();
    assert!(js.contains("ease: [\"linear\", (t) => "), "{js}");
    assert_spring(js, "scaleX", (7.1866, 22.3441, 0.3216));
    // Figma's CSS samples the same curve 50 times: 0, 0.0985, 0.342, 0.6483, …
    let css = n["codeSnippets"]["css"].as_str().unwrap();
    assert_samples(css, &[0.0, 0.0985, 0.342, 0.6483]);
    // A root with only the older duration field loops, as Figma reports it.
    assert_eq!(out["timelineCohorts"][0]["rootNodeId"], "1:800");
    assert_eq!(out["timelineCohorts"][0]["loopMode"], "loop");
}

/// The spring function of `value`'s ease in a motion.dev snippet has Figma's decay and
/// frequency within 0.5 % and its ratio to the hundredth.
fn assert_spring(js: &str, value: &str, (decay, freq, ratio): (f64, f64, f64)) {
    let line = js
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("{value}:")))
        .unwrap_or_else(|| panic!("no {value} in {js}"));
    let num = |after: &str| -> f64 {
        let rest = &line[line
            .find(after)
            .unwrap_or_else(|| panic!("{after} in {line}"))
            + after.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap();
        rest[..end].parse().unwrap()
    };
    let (d, f) = (num("Math.exp(-t * "), num("Math.cos(t * "));
    let r = num(") + ");
    assert!(
        (d / decay - 1.0).abs() < 0.005,
        "decay {d}, Figma {decay}: {line}"
    );
    assert!(
        (f / freq - 1.0).abs() < 0.005,
        "frequency {f}, Figma {freq}: {line}"
    );
    assert!((r - ratio).abs() < 0.01, "ratio {r}, Figma {ratio}: {line}");
}

/// The first samples of the CSS `linear()` spring are Figma's within 0.004.
fn assert_samples(css: &str, figma: &[f64]) {
    let curve = css.split("linear(").nth(1).expect("a linear() easing");
    let ours: Vec<f64> = curve
        .split(')')
        .next()
        .unwrap()
        .split(", ")
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(ours.len(), 51, "{curve}");
    for (o, f) in ours.iter().zip(figma) {
        assert!((o - f).abs() < 0.004, "ours {ours:?}, Figma {figma:?}");
    }
}
