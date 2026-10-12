//! The browser's GPU canvas, main-thread side (docs/GPU.md, L4 notes). wgpu's device and
//! queue are not `Send` in the browser, so the render worker sends each GPU tile as a
//! `vello_gpu::Scene` (`gpu_scenes.rs`) and the main thread renders a few of them per frame
//! into textures egui draws, on eframe's device (WebGPU, or WebGL2 through wgpu's GL
//! backend).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui_wgpu::RenderState;
use eframe::wgpu;
use web_time::Instant;

use crate::gpu_common::{Choice, CpuWhy, Vello, choose, label};
use crate::gpu_registry::{NativeTexture, Registry};
use crate::gpu_scenes::{Pending, TileGpu, asked_renderer};
use crate::recordings::Pixels;
use crate::tiles::{Key, Range};
use crate::worker::RenderDone;
use figstudio_fig::Guid;

/// The render worker's scene builder, the main thread's renderer (`None`: CPU canvas) and the
/// status bar's name for them, from eframe's device and the page's `?renderer=` query.
pub fn start(state: Option<&RenderState>) -> (TileGpu, Option<WebGpu>, String) {
    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let side = state.map(|s| s.device.limits().max_texture_dimension_2d);
    let choice = choose(asked_renderer(&search).as_deref(), side);
    let name = label(choice, state.map(|s| s.adapter.get_info().backend));
    let lost = Arc::new(AtomicBool::new(false));
    let (Choice::Gpu, Some(state), Some(side)) = (choice, state, side) else {
        let why = match choice {
            Choice::Cpu(why) => why,
            Choice::Gpu => CpuWhy::NoDevice,
        };
        return (TileGpu::new(Err(why), lost), None, name);
    };
    let flag = lost.clone();
    state
        .device
        .set_device_lost_callback(move |_, _| flag.store(true, Ordering::Release));
    let gpu = WebGpu {
        device: state.device.clone(),
        queue: state.queue.clone(),
        registry: Registry::new(state),
        vello: Vello::default(),
        lost: lost.clone(),
        pending: Pending::default(),
        failed: Rc::default(),
    };
    (TileGpu::new(Ok(side), lost), Some(gpu), name)
}

/// Renders the worker's scenes on the main thread, a bounded number per frame.
pub struct WebGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    registry: Registry,
    vello: Vello,
    /// Shared with the worker's [`TileGpu`]: once set, every tile is drawn on the CPU.
    lost: Arc<AtomicBool>,
    pending: Pending,
    /// Tiles whose wgpu error scope fired after they were shown (errors resolve later in the
    /// browser): dropped from the cache, so they are asked for again and drawn on the CPU.
    failed: Rc<RefCell<Vec<Key>>>,
}

impl std::fmt::Debug for WebGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebGpu").finish_non_exhaustive()
    }
}

impl WebGpu {
    /// Queues a tile whose pixels are a scene.
    pub fn push(&mut self, done: RenderDone) {
        self.pending.push(done);
    }

    /// Scenes are waiting (the UI keeps repainting until they are drawn).
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Drops the waiting scenes of a closed document.
    pub fn forget(&mut self, doc: u64) {
        self.pending.forget(doc);
    }

    /// Tiles to drop from the cache because their GPU work failed.
    pub fn take_failed(&self) -> Vec<Key> {
        std::mem::take(&mut self.failed.borrow_mut())
    }

    /// Drops the scenes outside `plan` (the last frame's document, page and tiles) and renders
    /// this frame's share of the rest, nearest the view centre first. Hands each tile to
    /// `each` with its texture, or `None` when it was dropped or the GPU failed (the GPU is
    /// then off): such a tile is not stored, and asked for again while it is still wanted.
    pub fn pump(
        &mut self,
        plan: Option<(u64, Guid, Range)>,
        mut each: impl FnMut(RenderDone, Option<NativeTexture>),
    ) {
        let mut pending = std::mem::take(&mut self.pending);
        pending.prune(plan, |done| each(done, None));
        pending.pump(|mut done| {
            let start = Instant::now();
            let texture = match &done.pixels {
                Pixels::Gpu(scene) => self.render(scene, done.key),
                Pixels::Cpu { .. } => None,
            };
            let spent = start.elapsed();
            done.elapsed += spent;
            each(done, texture);
            spent
        });
        self.pending = pending;
    }

    fn render(&mut self, scene: &vello_gpu::Scene, key: Key) -> Option<NativeTexture> {
        if self.lost.load(Ordering::Acquire) {
            return None;
        }
        let (texture, failed) = self.vello.render(&self.device, &self.queue, scene);
        let Some(texture) = texture else {
            self.lost.store(true, Ordering::Release);
            return None;
        };
        let (lost, keys) = (self.lost.clone(), self.failed.clone());
        wasm_bindgen_futures::spawn_local(async move {
            if failed.await {
                lost.store(true, Ordering::Release);
                keys.borrow_mut().push(key);
            }
        });
        Some(self.registry.register(texture))
    }
}
