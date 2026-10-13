//! The GPU canvas parts the desktop (`gpu_tiles.rs`) and the browser (`gpu_scenes.rs`,
//! `gpu_tiles_web.rs`) share: the renderer choice, the fallback table, the tile's scene and
//! rendering a scene into a tile texture. See `docs/GPU.md` (L3 and L4 notes).

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::wgpu;
use figstudio_render::{GPU_LAYER_TEXTURE, GpuUnsupported, Recording, Renderer, Viewport};

use crate::recordings::{Drawn, Recordings};
use crate::tiles::{Key, TEXTURE};
use crate::worker::RenderJob;

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
    /// `FIGSTUDIO_RENDERER=cpu` (desktop) or `?renderer=cpu` (browser).
    Asked,
    /// eframe has no wgpu device.
    NoDevice,
    /// The device's largest texture is below [`MIN_TEXTURE`].
    SmallTextures,
    /// A software adapter (llvmpipe, SwiftShader, WARP): `vello_gpu` on it is slower than the
    /// CPU tiles. An explicit `gpu` still uses it, for debugging.
    Software,
}

/// The renderer for the asked one (`cpu` or `gpu`, any case; unset or anything else means
/// the default), the device's largest texture side (`None`: no wgpu device) and its type.
pub fn choose(
    asked: Option<&str>,
    max_texture: Option<u32>,
    device: Option<wgpu::DeviceType>,
) -> Choice {
    let is = |want: &str| asked.is_some_and(|v| v.trim().eq_ignore_ascii_case(want));
    if is("cpu") {
        return Choice::Cpu(CpuWhy::Asked);
    }
    match max_texture {
        None => Choice::Cpu(CpuWhy::NoDevice),
        Some(side) if side < MIN_TEXTURE => Choice::Cpu(CpuWhy::SmallTextures),
        // Only `Cpu` is software: wgpu's GL backend on a real GPU reports `Other`.
        Some(_) if device == Some(wgpu::DeviceType::Cpu) && !is("gpu") => {
            Choice::Cpu(CpuWhy::Software)
        }
        Some(_) => Choice::Gpu,
    }
}

/// Routes wgpu errors raised outside an error scope (wgpu's default handler panics, and
/// egui-wgpu sets none) to a log line and turns the GPU tiles off (`lost`): a failed
/// `Surface::configure` or a stray validation error skips a frame instead of killing the app.
pub fn catch_uncaptured(device: &wgpu::Device, lost: std::sync::Arc<AtomicBool>) {
    device.on_uncaptured_error(std::sync::Arc::new(move |e: wgpu::Error| {
        tracing::warn!("figstudio: wgpu error (frame skipped, canvas tiles on the CPU): {e}");
        lost.store(true, Ordering::Release);
    }));
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
    /// `vello_gpu` or wgpu reported an error while rendering (desktop; the browser renders on
    /// the main thread and turns the GPU off instead, `gpu_tiles_web.rs`).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the browser's render errors turn the GPU off")
    )]
    RenderFailed,
}

impl Fallback {
    /// Counted in `Stats::gpu_fallback_tiles`: every tile the GPU was wanted for. Not when the
    /// CPU was asked for, or chosen for a software adapter: the whole canvas is on the CPU then.
    pub fn counted(self) -> bool {
        !matches!(self, Fallback::Cpu(CpuWhy::Asked | CpuWhy::Software))
    }
}

/// Draws tile `key` of `job` with `gpu` when its recording allows, else on the CPU with
/// `Stats::gpu_fallback_tiles` set when the GPU was wanted.
pub fn draw<G>(
    recordings: &mut Recordings,
    renderer: &mut Renderer,
    job: &RenderJob,
    key: &Key,
    mut gpu: impl FnMut(Option<&Recording>, &Viewport) -> Result<G, Fallback>,
) -> Drawn<G> {
    let mut why = None;
    let drawn = recordings.draw_with(renderer, job, key, |rec, viewport| {
        gpu(rec, viewport).map_err(|w| why = Some(w)).ok()
    });
    match drawn {
        Drawn::Cpu(mut frame) => {
            frame.stats.gpu_fallback_tiles = usize::from(why.is_some_and(Fallback::counted));
            Drawn::Cpu(frame)
        }
        gpu => gpu,
    }
}

/// Runs `step`; a panic in it turns the GPU off for the session (`lost`) and becomes a
/// counted CPU fallback instead of taking the render worker down. The browser build aborts
/// on panic, so there this catches nothing (docs/GPU.md, L4 notes).
pub fn guarded<T>(
    lost: &AtomicBool,
    step: impl FnOnce() -> Result<T, Fallback>,
) -> Result<T, Fallback> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).unwrap_or_else(|_| {
        lost.store(true, Ordering::Release);
        Err(Fallback::Lost)
    })
}

/// One `vello_gpu` renderer, created at the first tile (it compiles its shaders then).
#[derive(Default)]
pub struct Vello {
    inner: Option<(vello_gpu::Renderer, vello_gpu::Resources, wgpu::TextureView)>,
}

impl std::fmt::Debug for Vello {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vello")
            .field("created", &self.inner.is_some())
            .finish_non_exhaustive()
    }
}

impl Vello {
    /// Renders `scene` into a new tile texture and submits it. `None` when `vello_gpu`
    /// returns an error; the future resolves to true when a wgpu validation or out-of-memory
    /// error scope fired (at once on native, later in the browser).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &vello_gpu::Scene,
    ) -> (
        Option<wgpu::Texture>,
        impl std::future::Future<Output = bool> + use<>,
    ) {
        let side = u32::from(TEXTURE);
        let size = vello_gpu::RenderSize {
            width: TEXTURE,
            height: TEXTURE,
        };
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let valid = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (vello, resources, depth) = self.inner.get_or_insert_with(|| {
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
            queue,
            &mut encoder,
            &size,
            &view,
            Some(depth),
            &vello_gpu::TextureBindings::new(),
            vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
        );
        if rendered.is_ok() {
            queue.submit([encoder.finish()]);
        }
        let errors = [valid.pop(), memory.pop()];
        let failed = async move {
            let [valid, memory] = errors;
            valid.await.is_some() | memory.await.is_some()
        };
        (rendered.is_ok().then_some(texture), failed)
    }
}

/// The GPU's name for the status bar ("GPU (Metal)", "GPU (WebGL2)"); no device or driver
/// names.
pub fn label(choice: Choice, backend: Option<wgpu::Backend>) -> String {
    match (choice, backend) {
        (Choice::Gpu, Some(b)) => {
            format!("GPU ({})", backend_name(b, cfg!(target_arch = "wasm32")))
        }
        (Choice::Gpu, None) => "GPU".into(),
        (Choice::Cpu(_), _) => "CPU".into(),
    }
}

/// The backend's name; wgpu's GL backend is WebGL2 in the browser (`web`).
pub fn backend_name(b: wgpu::Backend, web: bool) -> &'static str {
    match b {
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Dx12 => "DX12",
        wgpu::Backend::Gl if web => "WebGL2",
        wgpu::Backend::Gl => "GL",
        wgpu::Backend::BrowserWebGpu => "WebGPU",
        _ => "other",
    }
}
