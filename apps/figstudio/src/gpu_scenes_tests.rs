//! The browser GPU canvas without a device: the `?renderer=` query, the renderer choice, the
//! fallback counts of scenes built on the worker. The main thread's queue: `gpu_scenes_queue_tests.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::gpu_common::{Choice, MIN_TEXTURE, choose, label};
use crate::recordings::Pixels;
use eframe::wgpu::Backend;

#[test]
fn the_query_names_the_renderer() {
    assert_eq!(asked_renderer("?renderer=cpu").as_deref(), Some("cpu"));
    assert_eq!(asked_renderer("?a=1&renderer=GPU").as_deref(), Some("GPU"));
    assert_eq!(asked_renderer("?renderer=%20cpu").as_deref(), Some(" cpu"));
    assert_eq!(asked_renderer(""), None);
    assert_eq!(asked_renderer("?renderers=cpu"), None);
    let asked = asked_renderer("?renderer=Cpu");
    assert_eq!(
        choose(asked.as_deref(), Some(8192)),
        Choice::Cpu(CpuWhy::Asked)
    );
}

#[test]
fn the_browser_backend_and_its_texture_limit_choose_the_renderer() {
    // WebGPU and WebGL2 alike: a device with 4096 px textures or more draws on the GPU.
    for backend in [Backend::BrowserWebGpu, Backend::Gl] {
        let choice = choose(None, Some(MIN_TEXTURE));
        assert_eq!(choice, Choice::Gpu);
        let name = if backend == Backend::Gl {
            "WebGL2"
        } else {
            "WebGPU"
        };
        assert_eq!(
            crate::gpu_common::backend_name(backend, true),
            name,
            "{backend:?}"
        );
    }
    // The Known limit: below 4096 px the canvas stays on the CPU.
    let small = choose(None, Some(2048));
    assert_eq!(small, Choice::Cpu(CpuWhy::SmallTextures));
    assert_eq!(label(small, Some(Backend::Gl)), "CPU");
}

fn open(name: &str) -> Arc<figstudio_fig::Document> {
    let path = format!(
        "{}/../../tests/fixtures/{name}.fig",
        env!("CARGO_MANIFEST_DIR")
    );
    Arc::new(figstudio_fig::Document::open(path).unwrap())
}

/// Draws the content tiles of `name` with `gpu`: each one's fallback count, or `None` for a
/// scene.
fn draw_all(gpu: &mut TileGpu, name: &str) -> Vec<Option<usize>> {
    let doc = open(name);
    let b = figstudio_render::content_bounds(&doc, doc.pages[0]).unwrap();
    let keys = crate::tiles::Range::covering(0, b).by_distance(doc.instance, doc.pages[0]);
    let mut recordings = Recordings::default();
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    keys.iter()
        .map(|key| draw_one(gpu, &mut recordings, &mut renderer, &doc, key))
        .collect()
}

/// The first content tile of `name`.
fn draw(gpu: &mut TileGpu, name: &str) -> Option<usize> {
    draw_all(gpu, name)[0]
}

fn draw_one(
    gpu: &mut TileGpu,
    recordings: &mut Recordings,
    renderer: &mut Renderer,
    doc: &Arc<figstudio_fig::Document>,
    key: &Key,
) -> Option<usize> {
    let key = *key;
    let job = RenderJob {
        tab: 1,
        doc: doc.clone(),
        source: None,
        root: doc.pages[0],
        background: None,
        images: 0,
        tiles: [key].into_iter().collect(),
        received: Vec::new(),
    };
    let drawn = gpu.draw(recordings, renderer, &job, &key);
    match drawn.into_parts() {
        (Pixels::Cpu { .. }, stats, _) => Some(stats.gpu_fallback_tiles),
        (Pixels::Gpu(_), ..) => None,
    }
}

#[test]
fn the_worker_builds_scenes_and_counts_every_unwanted_cpu_tile() {
    let lost = Arc::new(AtomicBool::new(false));
    let mut gpu = TileGpu::new(Ok(8192), lost.clone());
    assert_eq!(
        draw(&mut gpu, "vector"),
        None,
        "a scene for the main thread"
    );
    // Images are drawn on the CPU (`GpuUnsupported::Image`), and counted.
    let structure = draw_all(&mut gpu, "structure");
    assert!(structure.contains(&Some(1)), "{structure:?}");
    assert!(structure.iter().all(|t| *t != Some(0)), "{structure:?}");
    // The main thread lost the device or saw a render fail: CPU for the rest, counted.
    lost.store(true, Ordering::Release);
    assert_eq!(draw(&mut gpu, "vector"), Some(1));
    // No device, or textures too small: counted. Asking for the CPU: not counted.
    let mut none = TileGpu::new(Err(CpuWhy::NoDevice), Arc::default());
    assert_eq!(draw(&mut none, "vector"), Some(1));
    let mut small = TileGpu::new(Err(CpuWhy::SmallTextures), Arc::default());
    assert_eq!(draw(&mut small, "vector"), Some(1));
    assert_eq!(draw(&mut TileGpu::default(), "vector"), Some(0));
}

/// A page of `n` overlapping `side`-px ellipses around the origin (a heavy vector page).
fn ellipses(n: u32, side: f64) -> Arc<figstudio_fig::Document> {
    use figstudio_fig::{Guid, Node, Obj, Val};
    let obj = |f: &[(&str, Val)]| {
        Val::Obj(f.iter().fold(Obj::new(), |mut o, (k, v)| {
            o.insert(k, v.clone());
            o
        }))
    };
    let num = Val::Float;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/vector.fig"
    );
    let mut doc = figstudio_fig::Document::open(path).unwrap();
    let page = Guid(99, 1);
    let mut nodes = vec![
        Node::from_data(obj(&[
            ("guid", page.to_val()),
            ("type", Val::Str("CANVAS".into())),
        ]))
        .unwrap(),
    ];
    for i in 0..n {
        let (x, y) = (f64::from(i * 37 % 500), f64::from(i * 53 % 500));
        let paint = obj(&[
            ("type", Val::Str("SOLID".into())),
            ("visible", Val::Bool(true)),
            ("opacity", num(0.5)),
            ("color", obj(&[("r", num(0.5)), ("a", num(1.0))])),
        ]);
        nodes.push(
            Node::from_data(obj(&[
                ("guid", Guid(99, i + 2).to_val()),
                ("type", Val::Str("ELLIPSE".into())),
                (
                    "parentIndex",
                    obj(&[
                        ("guid", page.to_val()),
                        ("position", Val::Str(format!("{i:05}"))),
                    ]),
                ),
                ("size", obj(&[("x", num(side)), ("y", num(side))])),
                (
                    "transform",
                    obj(&[
                        ("m00", num(1.0)),
                        ("m11", num(1.0)),
                        ("m02", num(x - side / 2.0)),
                        ("m12", num(y - side / 2.0)),
                    ]),
                ),
                ("fillPaints", Val::Arr(vec![paint])),
            ]))
            .unwrap(),
        );
    }
    doc.set_nodes(nodes).unwrap();
    Arc::new(doc)
}

/// The scene the worker sends for tile (0, 0) of `doc`, or the tile's fallback count.
fn scene_of(doc: &Arc<figstudio_fig::Document>) -> Result<usize, usize> {
    let key = Key {
        doc: doc.instance,
        page: figstudio_fig::Guid(99, 1),
        level: 0,
        x: 0,
        y: 0,
    };
    let job = RenderJob {
        tab: 1,
        doc: doc.clone(),
        source: None,
        root: key.page,
        background: None,
        images: 0,
        tiles: [key].into_iter().collect(),
        received: Vec::new(),
    };
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let mut gpu = TileGpu::new(Ok(8192), Arc::default());
    let drawn = gpu.draw(&mut Recordings::default(), &mut renderer, &job, &key);
    match drawn.into_parts() {
        (Pixels::Gpu(t), ..) => Ok(t.bytes),
        (Pixels::Cpu { .. }, stats, _) => Err(stats.gpu_fallback_tiles),
    }
}

#[test]
fn an_oversized_scene_falls_back_counted_and_sent_scenes_carry_their_size() {
    // 1,000 small ellipses: a scene of a few MiB, sent with its estimate.
    let small = scene_of(&ellipses(1000, 40.0));
    assert!(
        small.is_ok_and(|b| b > 1 << 20 && b <= MAX_SCENE_BYTES),
        "{small:?}"
    );
    // 1,000 large overlapping ellipses: over the per-scene limit, drawn on the CPU, counted.
    assert_eq!(scene_of(&ellipses(1000, 500.0)), Err(1));
}

#[path = "gpu_scenes_queue_tests.rs"]
mod queue;
