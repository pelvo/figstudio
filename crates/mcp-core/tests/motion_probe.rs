//! `tests/fixtures/motion-probe.fig`: a file made for this check in a scratch Figma draft
//! (one rectangle per easing, moving 1000 px over a 1 s looping timeline). Figma's own
//! `get_motion_context` on it gave the samples and snippets below.

use figstudio_fig::Document;
use figstudio_mcp_core::motion::motion_context;
use serde_json::Value;

fn probe() -> Value {
    let doc = Document::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/motion-probe.fig"
    ))
    .unwrap();
    let out = motion_context(&doc, "2:2", true, 1_000_000);
    assert!(!out.is_error, "{:?}", out.content);
    out.structured.unwrap()
}

/// Layer name, Figma's motion.dev transition, and the first CSS `linear()` samples (of 51).
const FIGMA: [(&str, &str, &[f64]); 13] = [
    (
        "GENTLE",
        "duration: 1, type: 'spring', bounce: 0.25, repeat: Infinity",
        &[
            0.0, 0.0188, 0.0679, 0.1374, 0.2195, 0.308, 0.3978, 0.4856, 0.5686, 0.6452, 0.7142,
        ],
    ),
    (
        "QUICK",
        "duration: 1, type: 'spring', bounce: 0.4226, repeat: Infinity",
        &[
            0.0, 0.03, 0.1077, 0.2165, 0.342, 0.4728, 0.6001, 0.7175, 0.8208, 0.9077, 0.9776,
        ],
    ),
    (
        "BOUNCY",
        "duration: 1, type: 'spring', bounce: 0.6938, repeat: Infinity",
        &[
            0.0, 0.0985, 0.342, 0.6483, 0.9442, 1.1767, 1.3179, 1.364, 1.3302, 1.2427, 1.1314,
        ],
    ),
    (
        "SLOW",
        "duration: 1, type: 'spring', bounce: 0, repeat: Infinity",
        &[
            0.0, 0.0216, 0.0747, 0.1458, 0.2255, 0.3076, 0.3879, 0.4638, 0.5339, 0.5974, 0.6542,
        ],
    ),
    (
        "CUSTOM_SPRING_0.3",
        "duration: 1, type: 'spring', bounce: 0.3, repeat: Infinity",
        &[
            0.0, 0.0212, 0.0764, 0.1545, 0.2463, 0.3445, 0.4435, 0.5391, 0.6282, 0.7091, 0.7805,
        ],
    ),
    (
        "CUSTOM_SPRING_0.8",
        "duration: 1, type: 'spring', bounce: 0.8, repeat: Infinity",
        &[
            0.0, 0.2201, 0.7134, 1.2053, 1.4894, 1.4977, 1.295, 1.0184, 0.8024, 0.7228, 0.7793,
        ],
    ),
    (
        "EASE_IN",
        "duration: 1, times: [0, 1], ease: \"easeIn\", repeat: Infinity",
        &[],
    ),
    (
        "EASE_OUT",
        "duration: 1, times: [0, 1], ease: \"easeOut\", repeat: Infinity",
        &[],
    ),
    (
        "EASE_IN_AND_OUT",
        "duration: 1, times: [0, 1], ease: \"easeInOut\", repeat: Infinity",
        &[],
    ),
    (
        "EASE_IN_BACK",
        "duration: 1, times: [0, 1], ease: [0.3, -0.05, 0.7, -0.5], repeat: Infinity",
        &[],
    ),
    (
        "EASE_OUT_BACK",
        "duration: 1, times: [0, 1], ease: [0.45, 1.45, 0.8, 1], repeat: Infinity",
        &[],
    ),
    (
        "EASE_IN_AND_OUT_BACK",
        "duration: 1, times: [0, 1], ease: [0.7, -0.4, 0.4, 1.4], repeat: Infinity",
        &[],
    ),
    (
        "LINEAR",
        "duration: 1, times: [0, 1], ease: \"linear\", repeat: Infinity",
        &[],
    ),
];

#[test]
fn every_easing_matches_figmas_get_motion_context() {
    let out = probe();
    assert_eq!(out["timelineCohorts"][0]["loopMode"], "loop");
    for (name, figma_js, figma_css) in FIGMA {
        let node = out["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == name)
            .unwrap_or_else(|| panic!("no {name}"));
        let js = node["codeSnippets"]["motion"].as_str().unwrap();
        assert!(js.contains(figma_js), "{name}: {js}");
        if figma_css.is_empty() {
            continue;
        }
        let css = node["codeSnippets"]["css"].as_str().unwrap();
        let curve = css.split("linear(").nth(1).expect("a linear() easing");
        let ours: Vec<f64> = curve
            .split(')')
            .next()
            .unwrap()
            .split(", ")
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(ours.len(), 51, "{name}");
        for (i, (o, f)) in ours.iter().zip(figma_css).enumerate() {
            assert!(
                (o - f).abs() < 0.004,
                "{name} sample {i}: ours {o}, Figma {f}"
            );
        }
    }
}

#[test]
fn presets_read_back_under_figmas_public_names() {
    let out = probe();
    let easing = |name: &str| {
        out["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == name)
            .unwrap()["keyframeBindings"][0]["keyframes"][1]["easing"]
            .clone()
    };
    assert_eq!(easing("GENTLE")["type"], "GENTLE");
    assert_eq!(easing("QUICK")["type"], "QUICK");
    assert_eq!(easing("BOUNCY")["type"], "BOUNCY");
    assert_eq!(easing("SLOW")["type"], "SLOW");
    assert_eq!(easing("EASE_IN_BACK")["type"], "EASE_IN_BACK");
}
