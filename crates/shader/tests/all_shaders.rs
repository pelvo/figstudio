//! Every shader of `tests/fixtures/shaders.fig` (fills and effects) run from its source
//! (`tests/fixtures/shader-sources`), as the renderer draws it. Their WebGPU use covers
//! multisampling, depth, mip views, `writeTexture` and several passes. Skipped without a GPU.
//! The fixture's images were removed, so effects run on the renderer's image placeholder.

use std::path::PathBuf;
use std::sync::Arc;

use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::shaders::ShaderSource;
use figstudio_render::Renderer;
use figstudio_render::shader::ShaderFills;
use figstudio_shader::Fills;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn sources() -> Vec<ShaderSource> {
    let dir = root().join("shader-sources");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("shader sources") {
        let path = entry.unwrap().path();
        let files = ["main.ts", "features.json"]
            .iter()
            .map(|f| {
                (
                    (*f).to_owned(),
                    std::fs::read_to_string(path.join(f)).unwrap(),
                )
            })
            .collect();
        let label = path.file_name().unwrap().to_string_lossy().into_owned();
        out.push(ShaderSource::new(label, files));
    }
    out
}

fn gpu() -> bool {
    let probe = figstudio_shader::render(
        "export function render() {}",
        &serde_json::json!({}),
        1,
        1,
        0.0,
    );
    if probe.as_ref().is_err_and(|e| e.is_no_gpu()) {
        eprintln!("SKIPPED: no GPU adapter");
        return false;
    }
    true
}

/// Fill layers (`fillPaints`) and effect layers (`effects`) of the fixture, with the shader.
const LAYERS: [(&str, &str); 11] = [
    ("1:14625", "mesh-gradient"),
    ("1:14627", "clouds"),
    ("157:1822", "moving-gradient"),
    ("1:14632", "pixelate"),
    ("1:14635", "bloom"),
    ("1:14638", "halftone"),
    ("1:14641", "pattern-refraction"),
    ("1:14644", "pixel-stretch"),
    ("156:1003", "crt-screen"),
    ("157:1803", "light-rays"),
    ("157:1819", "moving-blobs"),
];

#[test]
fn every_shader_in_the_file_is_drawn_instead_of_skipped() {
    worker();
    if !gpu() {
        return;
    }
    let doc = Document::open(root().join("shaders.fig")).unwrap();
    let fills: Arc<dyn ShaderFills> = Arc::new(Fills::new(sources()));
    for (id, name) in LAYERS {
        let guid: Guid = id.parse().unwrap();
        let draw = |host: Option<Arc<dyn ShaderFills>>| {
            let mut r = Renderer::new();
            r.shaders = host;
            r.render_export_complete(&doc, guid, 0.25, None, |_| None)
                .unwrap()
        };
        let without = draw(None);
        let with = draw(Some(fills.clone()));
        let skipped =
            |f: &figstudio_render::Frame| f.stats.unsupported_paints + f.stats.skipped_effects;
        assert_eq!(
            skipped(&with),
            0,
            "{name} ({id}) not drawn: {:?}",
            with.stats
        );
        assert!(skipped(&without) > 0, "{name} ({id})");
        // An effect on the flat image placeholder may leave it flat (a stretch of one colour).
        let fill = doc
            .node(guid)
            .unwrap()
            .data
            .get("effects")
            .is_none_or(|e| e.items().is_empty());
        if !fill {
            continue;
        }
        assert_ne!(
            with.pixmap.data_as_u8_slice(),
            without.pixmap.data_as_u8_slice(),
            "{name} ({id}) drew nothing"
        );
    }
}

#[test]
fn gradient_stops_reach_the_shader_as_plain_colours() {
    worker();
    let doc = Document::open(root().join("shaders.fig")).unwrap();
    let paint = &doc
        .node("1:14627".parse().unwrap())
        .unwrap()
        .data
        .get("fillPaints")
        .unwrap()
        .items()[0];
    let sources = sources();
    let (_, params) = figstudio_mcp_core::shaders::program(&doc, paint, &sources).unwrap();
    // Clouds reads stop.color.r…a (gradientParam); the file nests it as variable data.
    let stop = &params["skyColor"]["stops"][0];
    assert_eq!(stop["color"]["b"], 0.471, "{params}");
    assert_eq!(stop["position"], 0.0);
}

/// A shader whose output is its clock: red = frame.time / 1000.
fn clock_shader(animated: bool) -> ShaderSource {
    let main = "export default function Effect() {}
export function render(device, frame) {
  const e = device.createCommandEncoder();
  const p = e.beginRenderPass({ colorAttachments: [{ view: frame.output.createView(),
    loadOp: 'clear', clearValue: { r: (frame.time || 0) / 1000, g: 0, b: 0, a: 1 }, storeOp: 'store' }] });
  p.end();
  device.queue.submit([e.finish()]);
}";
    let features = format!("{{\"name\": \"clock\", \"version\": 2, \"isAnimated\": {animated}}}");
    ShaderSource::new(
        "clock".into(),
        vec![
            ("main.ts".into(), main.into()),
            ("features.json".into(), features),
        ],
    )
}

#[test]
fn animated_shaders_follow_the_playback_clock_and_still_ones_do_not() {
    worker();
    if !gpu() {
        return;
    }
    // A layer of the fixture whose fill is the shader, pointed at the clock shader by label.
    let doc = Document::open(root().join("shaders.fig")).unwrap();
    let paint = &doc
        .node("1:14622".parse().unwrap())
        .unwrap()
        .data
        .get("fillPaints")
        .unwrap()
        .items()[1];
    let key = "c2b302fbdfce8a1ec92a5a80b10a5916ab506ed7";
    let red = |animated: bool, ms: f64| {
        let mut src = clock_shader(animated);
        src.label = key.into();
        let fills = Fills::new(vec![src]);
        match fills.fill(&doc, paint, 4, 4, ms) {
            figstudio_render::shader::Shaded::Drawn(p) => p.sample(1, 1).r,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(red(true, 0.0), 0);
    assert!(
        (i32::from(red(true, 500.0)) - 128).abs() <= 1,
        "{}",
        red(true, 500.0)
    );
    assert_eq!(red(false, 500.0), 0, "a still shader runs at time 0");
}

/// The shader process for these tests: this crate's worker binary (a test binary cannot be one).
fn worker() {
    figstudio_shader::set_worker_program(env!("CARGO_BIN_EXE_figstudio-shader-worker").into());
}
