//! Renderer choice, the fallback table and texture bookkeeping (no GPU needed), plus GPU
//! tiles drawn through the app's path against its CPU tiles (skips without an adapter).

use std::sync::Mutex;

use figstudio_render::Stats;

use super::*;
use crate::gpu_registry::Free;
use crate::recordings::Pixels;
use crate::tile_cache::Cache;

#[test]
fn the_env_var_and_the_device_choose_the_renderer() {
    let big = Some(8192);
    assert_eq!(choose(None, big), Choice::Gpu);
    assert_eq!(choose(Some("gpu"), big), Choice::Gpu);
    assert_eq!(choose(Some(" CPU "), big), Choice::Cpu(CpuWhy::Asked));
    assert_eq!(choose(Some("cpu"), None), Choice::Cpu(CpuWhy::Asked));
    // Anything else means the default: the GPU when there is one.
    assert_eq!(choose(Some("vulkan"), big), Choice::Gpu);
    assert_eq!(choose(Some("gpu"), None), Choice::Cpu(CpuWhy::NoDevice));
    assert_eq!(choose(None, Some(MIN_TEXTURE)), Choice::Gpu);
    assert_eq!(
        choose(None, Some(MIN_TEXTURE - 1)),
        Choice::Cpu(CpuWhy::SmallTextures)
    );
    assert_eq!(choose(None, Some(2048)), Choice::Cpu(CpuWhy::SmallTextures));
}

#[test]
fn the_status_bar_names_only_the_backend() {
    assert_eq!(
        label(Choice::Gpu, Some(wgpu::Backend::Metal)),
        "GPU (Metal)"
    );
    assert_eq!(
        label(Choice::Gpu, Some(wgpu::Backend::Vulkan)),
        "GPU (Vulkan)"
    );
    let cpu = Choice::Cpu(CpuWhy::SmallTextures);
    assert_eq!(label(cpu, Some(wgpu::Backend::Gl)), "CPU");
    assert_eq!(label(Choice::Cpu(CpuWhy::Asked), None), "CPU");
}

#[test]
fn every_fallback_but_asking_for_the_cpu_is_counted() {
    assert!(!Fallback::Cpu(CpuWhy::Asked).counted());
    for f in [
        Fallback::Cpu(CpuWhy::NoDevice),
        Fallback::Cpu(CpuWhy::SmallTextures),
        Fallback::Lost,
        Fallback::NoRecording,
        Fallback::Unsupported(GpuUnsupported::Image),
        Fallback::RenderFailed,
    ] {
        assert!(f.counted(), "{f:?}");
    }
}

fn open(name: &str) -> std::sync::Arc<figstudio_fig::Document> {
    let path = format!(
        "{}/../../tests/fixtures/{name}.fig",
        env!("CARGO_MANIFEST_DIR")
    );
    std::sync::Arc::new(figstudio_fig::Document::open(path).unwrap())
}

fn renderer() -> Renderer {
    let mut r = Renderer::new();
    r.system_fonts = false;
    r
}

fn job(doc: &std::sync::Arc<figstudio_fig::Document>, tiles: &[Key]) -> RenderJob {
    RenderJob {
        tab: 1,
        doc: doc.clone(),
        source: None,
        root: doc.pages[0],
        background: Some(figstudio_render::page_background(doc, doc.pages[0])),
        images: 0,
        tiles: tiles.iter().copied().collect(),
        received: Vec::new(),
    }
}

/// The tiles of `level` holding the fixture's content, plus a ring.
fn tiles_of(doc: &figstudio_fig::Document, level: i32) -> Vec<Key> {
    let b = figstudio_render::content_bounds(doc, doc.pages[0]).unwrap();
    let range = crate::tiles::Range::covering(level, b).grow(1);
    range.by_distance(doc.instance, doc.pages[0])
}

/// The CPU tile `key`, drawn as with `FIGSTUDIO_RENDERER=cpu`.
fn cpu_tile(doc: &std::sync::Arc<figstudio_fig::Document>, keys: &[Key], key: &Key) -> Vec<u8> {
    let drawn = TileGpu::default().draw(
        &mut Recordings::default(),
        &mut renderer(),
        &job(doc, keys),
        key,
    );
    match drawn.into_parts() {
        (Pixels::Cpu { rgba, .. }, stats, _) => {
            assert_eq!(
                stats.gpu_fallback_tiles, 0,
                "asking for the CPU is not a fallback"
            );
            rgba
        }
        (Pixels::Gpu(_), ..) => panic!("the CPU drawer drew on the GPU"),
    }
}

#[test]
fn without_a_device_tiles_are_cpu_tiles_and_counted() {
    let doc = open("vector");
    let keys = tiles_of(&doc, 0);
    let mut gpu = TileGpu::new(Choice::Gpu, None);
    let mut recordings = Recordings::default();
    let drawn = gpu.draw(
        &mut recordings,
        &mut renderer(),
        &job(&doc, &keys),
        &keys[0],
    );
    let (pixels, stats, _) = drawn.into_parts();
    let Pixels::Cpu { rgba, .. } = pixels else {
        panic!("no device, yet a GPU tile");
    };
    assert_eq!(stats.gpu_fallback_tiles, 1);
    assert_eq!(
        rgba,
        cpu_tile(&doc, &keys, &keys[0]),
        "the fallback is the CPU tile"
    );
}

#[test]
fn a_tile_without_a_usable_recording_falls_back() {
    let doc = open("vector");
    let key = tiles_of(&doc, 0)[0];
    let viewport = Viewport {
        width: TEXTURE,
        height: TEXTURE,
        transform: key.transform(),
        background: None,
    };
    assert_eq!(
        scene(None, &viewport, GPU_LAYER_TEXTURE).err(),
        Some(Fallback::NoRecording)
    );
}

#[test]
fn evicted_and_forgotten_gpu_tiles_free_their_egui_texture() {
    let freed = Arc::new(Mutex::new(Vec::new()));
    let log = freed.clone();
    let free: Free = Arc::new(move |id| log.lock().unwrap().push(id));
    let key = |x| Key {
        doc: 1,
        page: figstudio_fig::Guid(0, 1),
        level: 0,
        x,
        y: 0,
    };
    let mut cache = Cache::new(2);
    for x in 0..3 {
        cache.insert(
            key(x),
            NativeTexture::fake(x as u64, free.clone()),
            Stats::default(),
            None,
        );
    }
    assert_eq!(
        *freed.lock().unwrap(),
        [egui::TextureId::User(0)],
        "the oldest, once"
    );
    // Replacing a tile frees the old texture.
    cache.insert(
        key(2),
        NativeTexture::fake(9, free.clone()),
        Stats::default(),
        None,
    );
    assert_eq!(freed.lock().unwrap().len(), 2);
    cache.retain(|k| k.doc != 1);
    let mut ids = freed.lock().unwrap().clone();
    ids.sort_by_key(|id| format!("{id:?}"));
    let user = |n| egui::TextureId::User(n);
    assert_eq!(
        ids,
        [user(0), user(1), user(2), user(9)],
        "each freed exactly once"
    );
}

#[path = "gpu_tiles_gpu_tests.rs"]
mod on_gpu;

#[test]
fn a_panic_while_building_or_rendering_turns_the_gpu_off_and_falls_back() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let lost = AtomicBool::new(false);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r: Result<(), Fallback> = guarded(&lost, || panic!("scene build"));
    std::panic::set_hook(hook);
    assert_eq!(r, Err(Fallback::Lost));
    assert!(
        Fallback::Lost.counted(),
        "a panic is a counted CPU fallback"
    );
    assert!(lost.load(Ordering::Acquire), "the GPU stays off afterwards");
    // A step that does not panic passes its result through, and the worker carries on.
    let ok = AtomicBool::new(false);
    assert_eq!(guarded(&ok, || Ok::<_, Fallback>(7)), Ok(7));
    assert!(!ok.load(Ordering::Acquire));
}
