//! The "Concentric patterns" shader of `tests/fixtures/shaders.fig`, run from its source
//! (`tests/fixtures/shader-sources/concentric`) on the GPU. Skipped without a GPU adapter.

use std::path::PathBuf;
use std::sync::Arc;

use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::shaders::{self, ShaderSource};
use figstudio_render::Renderer;
use figstudio_shader::{Fills, render};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn doc() -> Document {
    Document::open(root().join("shaders.fig")).expect("fixture opens")
}

fn source() -> ShaderSource {
    let dir = root().join("shader-sources/concentric");
    let files = ["main.ts", "features.json"]
        .iter()
        .map(|f| {
            let text = std::fs::read_to_string(dir.join(f)).expect("source file");
            ((*f).to_owned(), text)
        })
        .collect();
    ShaderSource::new("concentric".into(), files)
}

/// The layer "Concentric Pattern" (its second fill is the shader).
const LAYER: Guid = Guid(1, 14622);

#[test]
fn the_layer_params_reach_the_shader_as_its_code_reads_them() {
    worker();
    let doc = doc();
    let sources = [source()];
    let paint = &doc
        .node(LAYER)
        .unwrap()
        .data
        .get("fillPaints")
        .unwrap()
        .items()[1];
    let (found, params) = shaders::program(&doc, paint, &sources).expect("shader and source");
    assert_eq!(found.label, "concentric");
    // A circle point reads as {x, y, radius, angle} (pointAngleRadiusParam).
    assert_eq!(params["transform"]["x"], 50.0);
    assert_eq!(params["transform"]["radius"], 144.0);
    assert_eq!(params["color"]["a"], 1.0);
    assert!(params.get("ringCount").is_some(), "{params}");
}

#[test]
fn concentric_rings_are_drawn() {
    worker();
    let doc = doc();
    let sources = [source()];
    let paint = &doc
        .node(LAYER)
        .unwrap()
        .data
        .get("fillPaints")
        .unwrap()
        .items()[1];
    let (found, params) = shaders::program(&doc, paint, &sources).unwrap();
    let main = &found.files.iter().find(|(f, _)| f == "main.ts").unwrap().1;
    let pixmap = match render(main, &params, 200, 200, 0.0) {
        Ok(p) => p,
        Err(e) if e.is_no_gpu() => return eprintln!("SKIPPED: {e}"),
        Err(e) => panic!("{e}"),
    };
    assert_eq!((pixmap.width(), pixmap.height()), (200, 200));
    // Along the middle row the black rings alternate with gaps: count the bands.
    let alpha: Vec<u8> = (0..200).map(|x| pixmap.sample(x, 100).a).collect();
    let bands = alpha
        .windows(2)
        .filter(|w| w[0] < 128 && w[1] >= 128)
        .count();
    assert!(bands >= 5, "bands {bands}: {alpha:?}");
    // Straight-alpha black: colour stays black wherever it is drawn.
    assert!((0..200).all(|x| pixmap.sample(x, 100).r == 0));
}

#[test]
fn the_renderer_draws_the_shader_fill_instead_of_skipping_it() {
    worker();
    let doc = doc();
    let draw = |shaders: Option<Arc<Fills>>| {
        let mut r = Renderer::new();
        r.shaders = shaders.map(|f| f as Arc<dyn figstudio_render::shader::ShaderFills>);
        r.render_export_complete(&doc, LAYER, 1.0, None, |_| None)
            .expect("renders")
    };
    if figstudio_shader::render(
        "export function render() {}",
        &serde_json::json!({}),
        1,
        1,
        0.0,
    )
    .is_err_and(|e| e.is_no_gpu())
    {
        return eprintln!("SKIPPED: no GPU adapter");
    }
    let without = draw(None);
    let with = draw(Some(Arc::new(Fills::new(vec![source()]))));
    assert_eq!(
        with.stats.unsupported_paints + 1,
        without.stats.unsupported_paints
    );
    assert_ne!(with.pixmap.data(), without.pixmap.data());
}

#[test]
fn bad_shader_code_is_an_error_not_a_panic() {
    worker();
    let params = serde_json::json!({});
    let cases = [
        // Throws.
        "export function render() { throw new Error('x'); }",
        // Never ends: stopped by the loop limit.
        "export function render() { while (true) {} }",
        // WGSL that does not compile, used in a pipeline.
        "export function render(device, frame) {
            const m = device.createShaderModule({ code: 'fn nope(' });
            device.createRenderPipeline({ layout: 'auto',
                vertex: { module: m, entryPoint: 'vs_main', buffers: [] },
                fragment: { module: m, entryPoint: 'fs_main', targets: [{ format: frame.output.format }] } });
        }",
        // A WebGPU call this runtime does not have.
        "export function render(device) { device.createComputePipeline({}); }",
        // Not a module at all.
        "export function (",
    ];
    for code in cases {
        match render(code, &params, 8, 8, 0.0) {
            Ok(_) => panic!("drew: {code}"),
            Err(e) if e.is_no_gpu() => return eprintln!("SKIPPED: {e}"),
            Err(_) => {}
        }
    }
}

/// The shader process for these tests: this crate's worker binary (a test binary cannot be one).
fn worker() {
    figstudio_shader::set_worker_program(env!("CARGO_BIN_EXE_figstudio-shader-worker").into());
}
