//! The GPU canvas (desktop): tiles replayed from page recordings into `vello_gpu` scenes and
//! rendered on eframe's wgpu device into textures egui draws, with the CPU tiles as fallback.
//! Threading and colour handling are explained in `docs/GPU.md` (L3 notes).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui_wgpu::RenderState;
use eframe::wgpu;
use figstudio_render::{GPU_LAYER_TEXTURE, GpuUnsupported, Recording, Renderer, Viewport};

pub use crate::gpu_registry::{NativeTexture, Registry};
use crate::recordings::{Drawn, Recordings};
use crate::tiles::{Key, TEXTURE};
use crate::worker::RenderJob;

/// A finished GPU tile: its texture, rendered and submitted.
pub type GpuTexture = wgpu::Texture;

/// Smallest `max_texture_dimension_2d` the GPU canvas runs on: `vello_gpu`'s intermediate
/// textures. Below it (WebGL2-class drivers) tiles stay on the CPU (docs/GPU.md, Known limit).
pub const MIN_TEXTURE: u32 = GPU_LAYER_TEXTURE as u32;

/// Which renderer draws the canvas tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Gpu,
    Cpu(CpuWhy),
}

/// Why the canvas is drawn on the CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuWhy {
    /// `FIGSTUDIO_RENDERER=cpu`.
    Asked,
    /// eframe has no wgpu device.
    NoDevice,
    /// The device's largest texture is below [`MIN_TEXTURE`].
    SmallTextures,
}

/// The renderer for `FIGSTUDIO_RENDERER` (`cpu` or `gpu`, any case; unset or anything else
/// means `gpu`) and the device's largest texture side (`None`: no wgpu device).
pub fn choose(env: Option<&str>, max_texture: Option<u32>) -> Choice {
    if env.is_some_and(|v| v.trim().eq_ignore_ascii_case("cpu")) {
        return Choice::Cpu(CpuWhy::Asked);
    }
    match max_texture {
        None => Choice::Cpu(CpuWhy::NoDevice),
        Some(side) if side < MIN_TEXTURE => Choice::Cpu(CpuWhy::SmallTextures),
        Some(_) => Choice::Gpu,
    }
}

/// Why one tile was drawn on the CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// The canvas renderer is the CPU.
    Cpu(CpuWhy),
    /// The device was lost (or `vello_gpu` panicked): the GPU is not used again.
    Lost,
    /// No recording for the tile (over the size limit or work budget, or not covering it).
    NoRecording,
    /// The tile uses something `vello_gpu` cannot draw.
    Unsupported(GpuUnsupported),
    /// `vello_gpu` or wgpu reported an error while rendering.
    RenderFailed,
}

impl Fallback {
    /// Counted in `Stats::gpu_fallback_tiles`: every tile the GPU was wanted for.
    pub fn counted(self) -> bool {
        self != Fallback::Cpu(CpuWhy::Asked)
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
    vello: Option<(vello_gpu::Renderer, vello_gpu::Resources, wgpu::TextureView)>,
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
                vello: None,
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
        let mut why = None;
        let drawn = recordings.draw_with(renderer, job, key, |rec, viewport| {
            self.tile(rec, viewport).map_err(|w| why = Some(w)).ok()
        });
        match drawn {
            Drawn::Cpu(mut frame) => {
                frame.stats.gpu_fallback_tiles = usize::from(why.is_some_and(Fallback::counted));
                Drawn::Cpu(frame)
            }
            gpu => gpu,
        }
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

/// Runs `step`; a panic in it turns the GPU off for the session (`lost`) and becomes a
/// counted CPU fallback instead of taking the render worker down.
fn guarded<T>(
    lost: &AtomicBool,
    step: impl FnOnce() -> Result<T, Fallback>,
) -> Result<T, Fallback> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).unwrap_or_else(|_| {
        lost.store(true, Ordering::Release);
        Err(Fallback::Lost)
    })
}

impl Gpu {
    /// Renders `scene` into a new tile texture and submits it; wgpu validation and memory
    /// errors are caught in error scopes (on native they resolve at once).
    fn render(&mut self, scene: &vello_gpu::Scene) -> Result<wgpu::Texture, Fallback> {
        let side = u32::from(TEXTURE);
        let size = vello_gpu::RenderSize {
            width: TEXTURE,
            height: TEXTURE,
        };
        let device = &self.device;
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let valid = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (vello, resources, depth) = self.vello.get_or_insert_with(|| {
            let config = vello_gpu::RenderTargetConfig {
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: TEXTURE,
                height: TEXTURE,
            };
            let (r, res) = vello_gpu::Renderer::new(device, &config);
            (
                r,
                res,
                vello_gpu::Renderer::create_depth_texture_view(device, &size),
            )
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("figstudio gpu tile"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // egui's own textures: premultiplied sRGB bytes, sampled without conversion.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        let rendered = vello.render(
            scene,
            resources,
            device,
            &self.queue,
            &mut encoder,
            &size,
            &view,
            Some(depth),
            &vello_gpu::TextureBindings::new(),
            vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
        );
        if rendered.is_ok() {
            self.queue.submit([encoder.finish()]);
        }
        let errors = [valid.pop(), memory.pop()].map(pollster::block_on);
        if rendered.is_err() || errors.iter().any(Option::is_some) {
            return Err(Fallback::RenderFailed);
        }
        Ok(texture)
    }
}

/// The GPU's name for the status bar ("GPU (Metal)"); no device or driver names.
pub fn label(choice: Choice, backend: Option<wgpu::Backend>) -> String {
    match (choice, backend) {
        (Choice::Gpu, Some(b)) => format!("GPU ({})", backend_name(b)),
        (Choice::Gpu, None) => "GPU".into(),
        (Choice::Cpu(_), _) => "CPU".into(),
    }
}

fn backend_name(b: wgpu::Backend) -> &'static str {
    match b {
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Dx12 => "DX12",
        wgpu::Backend::Gl => "GL",
        wgpu::Backend::BrowserWebGpu => "WebGPU",
        _ => "other",
    }
}

#[cfg(test)]
#[path = "gpu_tiles_tests.rs"]
mod tests;
