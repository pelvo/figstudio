//! The GPU canvas (desktop): tiles replayed from page recordings into `vello_gpu` scenes and
//! rendered on eframe's wgpu device into textures egui draws, with the CPU tiles as fallback.
//! Threading and colour handling are explained in `docs/GPU.md` (L3 notes).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui_wgpu::RenderState;
use eframe::wgpu;
use figstudio_render::{GPU_LAYER_TEXTURE, Recording, Renderer, Viewport};

pub use crate::gpu_common::{Choice, CpuWhy, Fallback, choose, label};
use crate::gpu_common::{Vello, guarded};
pub use crate::gpu_registry::Registry;
use crate::recordings::{Drawn, Recordings};
use crate::tiles::Key;
use crate::worker::RenderJob;

/// A finished GPU tile: its texture, rendered and submitted.
pub type GpuTexture = wgpu::Texture;

/// The render worker's GPU: draws tiles into textures, or says why not.
#[derive(Debug)]
pub struct TileGpu {
    gpu: Result<Gpu, CpuWhy>,
}

impl Default for TileGpu {
    /// The CPU, as asked (tests and `FIGSTUDIO_RENDERER=cpu`).
    fn default() -> Self {
        TileGpu {
            gpu: Err(CpuWhy::Asked),
        }
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    lost: Arc<AtomicBool>,
    max_texture: u16,
    /// Created on the worker thread at the first tile (shader compiles off the UI thread).
    vello: Vello,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu")
            .field("max_texture", &self.max_texture)
            .finish_non_exhaustive()
    }
}

impl TileGpu {
    /// eframe's GPU (`state`) when `choice` is [`Choice::Gpu`].
    pub fn new(choice: Choice, state: Option<&RenderState>) -> Self {
        let state = match (choice, state) {
            (Choice::Cpu(why), _) => return TileGpu { gpu: Err(why) },
            (Choice::Gpu, None) => {
                return TileGpu {
                    gpu: Err(CpuWhy::NoDevice),
                };
            }
            (Choice::Gpu, Some(s)) => s,
        };
        Self::on(&state.device, &state.queue)
    }

    /// Tiles on the GPU of `device`.
    pub(crate) fn on(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |_, _| flag.store(true, Ordering::Release));
        let side = u16::try_from(device.limits().max_texture_dimension_2d).unwrap_or(u16::MAX);
        TileGpu {
            gpu: Ok(Gpu {
                device: device.clone(),
                queue: queue.clone(),
                lost,
                max_texture: side.min(GPU_LAYER_TEXTURE),
                vello: Vello::default(),
            }),
        }
    }

    /// Draws tile `key` of `job`: on the GPU when its recording allows (the texture, already
    /// submitted), else on the CPU with `Stats::gpu_fallback_tiles` set when the GPU was
    /// wanted. Never panics on a GPU failure: a panic inside `vello_gpu` or a lost device
    /// turns the GPU off for good.
    pub fn draw(
        &mut self,
        recordings: &mut Recordings,
        renderer: &mut Renderer,
        job: &RenderJob,
        key: &Key,
    ) -> Drawn<GpuTexture> {
        crate::gpu_common::draw(recordings, renderer, job, key, |rec, viewport| {
            self.tile(rec, viewport)
        })
    }

    fn tile(
        &mut self,
        rec: Option<&Recording>,
        viewport: &Viewport,
    ) -> Result<wgpu::Texture, Fallback> {
        let gpu = self.gpu.as_mut().map_err(|why| Fallback::Cpu(*why))?;
        if gpu.lost.load(Ordering::Acquire) {
            return Err(Fallback::Lost);
        }
        let max_texture = gpu.max_texture;
        // `vello_gpu` 0.3 panics on some inputs the scan does not know about yet, while the
        // scene is built as well as while it renders: both stay inside one unwind boundary.
        let lost = Arc::clone(&gpu.lost);
        guarded(&lost, || gpu.render(&scene(rec, viewport, max_texture)?))
    }
}

/// The tile's `vello_gpu` scene from its recording, or why the tile goes to the CPU.
pub fn scene(
    rec: Option<&Recording>,
    viewport: &Viewport,
    max_texture: u16,
) -> Result<vello_gpu::Scene, Fallback> {
    match rec.map(|r| r.gpu_scene(viewport, max_texture)) {
        None | Some(None) => Err(Fallback::NoRecording),
        Some(Some(Err(why))) => Err(Fallback::Unsupported(why)),
        Some(Some(Ok((scene, _)))) => Ok(scene),
    }
}

impl Gpu {
    /// Renders `scene` into a new tile texture and submits it; wgpu validation and memory
    /// errors are caught in error scopes (on native they resolve at once).
    fn render(&mut self, scene: &vello_gpu::Scene) -> Result<wgpu::Texture, Fallback> {
        let (texture, failed) = self.vello.render(&self.device, &self.queue, scene);
        match (texture, pollster::block_on(failed)) {
            (Some(texture), false) => Ok(texture),
            _ => Err(Fallback::RenderFailed),
        }
    }
}

#[cfg(test)]
#[path = "gpu_tiles_tests.rs"]
mod tests;
