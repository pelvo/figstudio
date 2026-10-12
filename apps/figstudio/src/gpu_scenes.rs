//! The browser's GPU canvas, the parts without a device (docs/GPU.md, L4 notes): wgpu objects
//! stay on the main thread in the browser, so the render worker only builds each tile's
//! `vello_gpu::Scene` from its recording, and the main thread renders a bounded number of
//! them per frame (`gpu_tiles_web.rs`). Compiled natively too, for the tests.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use figstudio_render::{GPU_LAYER_TEXTURE, Recording, Renderer, Viewport};
use web_time::Duration;

use crate::gpu_common::{CpuWhy, Fallback, guarded, scene};
use crate::recordings::{Drawn, Recordings};
use crate::tiles::Key;
use crate::worker::RenderJob;

/// A GPU tile on its way to the main thread: its scene, rendered there.
pub type GpuTexture = vello_gpu::Scene;

/// Most GPU tiles the main thread renders in one frame.
pub const TILES_PER_FRAME: usize = 4;
/// After this much GPU work in a frame, the rest waits for the next frame.
pub const TIME_PER_FRAME: Duration = Duration::from_millis(8);

/// The render worker's side of the browser GPU: builds scenes, or says why a tile goes to
/// the CPU.
#[derive(Debug)]
pub struct TileGpu {
    /// The largest intermediate texture side, or why the canvas is on the CPU.
    gpu: Result<u16, CpuWhy>,
    /// Set by the main thread when the device is lost or a render failed: the rest of the
    /// session draws on the CPU.
    lost: Arc<AtomicBool>,
}

impl Default for TileGpu {
    /// The CPU, as asked (tests and `?renderer=cpu`).
    fn default() -> Self {
        TileGpu::new(Err(CpuWhy::Asked), Arc::default())
    }
}

impl TileGpu {
    /// Scenes for a device whose largest texture is `max_texture` (or the CPU, `why`);
    /// `lost` is shared with the main thread's renderer.
    pub fn new(gpu: Result<u32, CpuWhy>, lost: Arc<AtomicBool>) -> Self {
        let side = |s: u32| u16::try_from(s).unwrap_or(u16::MAX).min(GPU_LAYER_TEXTURE);
        TileGpu {
            gpu: gpu.map(side),
            lost,
        }
    }

    /// Draws tile `key` of `job`: its scene when its recording allows, else on the CPU with
    /// `Stats::gpu_fallback_tiles` set when the GPU was wanted.
    pub fn draw(
        &mut self,
        recordings: &mut Recordings,
        renderer: &mut Renderer,
        job: &RenderJob,
        key: &Key,
    ) -> Drawn<GpuTexture> {
        crate::gpu_common::draw(recordings, renderer, job, key, |rec, viewport| {
            self.scene(rec, viewport)
        })
    }

    fn scene(
        &self,
        rec: Option<&Recording>,
        viewport: &Viewport,
    ) -> Result<vello_gpu::Scene, Fallback> {
        let max_texture = self.gpu.map_err(Fallback::Cpu)?;
        if self.lost.load(Ordering::Acquire) {
            return Err(Fallback::Lost);
        }
        guarded(&self.lost, || scene(rec, viewport, max_texture))
    }
}

/// The `renderer` query parameter of the page's `location.search` (`?renderer=cpu|gpu`),
/// for [`crate::gpu_common::choose`].
pub fn asked_renderer(search: &str) -> Option<String> {
    let query = search.strip_prefix('?').unwrap_or(search);
    form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "renderer")
        .map(|(_, v)| v.into_owned())
}

/// Scenes waiting for the main thread, oldest first, rendered a few per frame.
#[derive(Debug)]
pub struct Pending<T> {
    queue: VecDeque<T>,
}

impl<T> Default for Pending<T> {
    fn default() -> Self {
        Pending {
            queue: VecDeque::new(),
        }
    }
}

impl<T> Pending<T> {
    pub fn push(&mut self, item: T) {
        self.queue.push_back(item);
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Drops the items `keep` refuses (a closed document).
    pub fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.queue.retain(keep);
    }

    /// Hands items to `render`, which returns the time it took, until [`TILES_PER_FRAME`]
    /// were rendered or [`TIME_PER_FRAME`] was spent. At least one goes per frame, so a slow
    /// tile (the first one compiles the shaders) cannot stall the queue.
    pub fn pump(&mut self, mut render: impl FnMut(T) -> Duration) {
        let mut spent = Duration::ZERO;
        for _ in 0..TILES_PER_FRAME {
            if spent >= TIME_PER_FRAME {
                break;
            }
            let Some(item) = self.queue.pop_front() else {
                break;
            };
            spent += render(item);
        }
    }
}

#[cfg(test)]
#[path = "gpu_scenes_tests.rs"]
mod tests;
