//! A headless wgpu device and `vello_gpu` renderer for `tests/gpu.rs`, after `vello_gpu`'s
//! `render_to_file` example: renders a scene into a texture and reads it back.
#![allow(
    dead_code,
    reason = "each GPU suite uses a different subset of these helpers"
)]
pub mod docs;

/// Tile side with its gutter, as the app draws tiles.
pub const SIDE: u16 = 516;

/// A wgpu device with a `vello_gpu` renderer for `SIDE`-pixel tiles, read back to memory.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
}

impl Gpu {
    /// Follows `vello_gpu`'s `render_to_file` example; `None` when no adapter or device.
    pub fn new() -> Option<Self> {
        // `WGPU_BACKEND` picks the backends (e.g. `WGPU_BACKEND=vulkan` on a Mac has none: skip).
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("SKIPPED: no GPU adapter ({e})");
                return None;
            }
        };
        eprintln!("GPU adapter: {:?}", adapter.get_info().backend);
        let (device, queue) =
            match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("SKIPPED: no GPU device ({e})");
                    return None;
                }
            };
        let config = vello_gpu::RenderTargetConfig {
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: SIDE,
            height: SIDE,
        };
        let (renderer, resources) = vello_gpu::Renderer::new(&device, &config);
        Some(Gpu {
            device,
            queue,
            renderer,
            resources,
        })
    }

    /// The largest intermediate texture side the renderer uses (its default, capped by the
    /// device), for `Recording::gpu_scene`.
    pub fn max_texture(&self) -> u16 {
        let device =
            u16::try_from(self.device.limits().max_texture_dimension_2d).unwrap_or(u16::MAX);
        device.min(figstudio_render::GPU_LAYER_TEXTURE)
    }

    /// Renders `scene` (`SIDE` square) and returns its RGBA8 pixels, or `vello_gpu`'s error.
    pub fn render(&mut self, scene: &vello_gpu::Scene) -> Result<Vec<u8>, vello_gpu::RenderError> {
        let side = u32::from(SIDE);
        let extent = wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let size = vello_gpu::RenderSize {
            width: SIDE,
            height: SIDE,
        };
        let depth = vello_gpu::Renderer::create_depth_texture_view(&self.device, &size);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.render(
            scene,
            &mut self.resources,
            &self.device,
            &self.queue,
            &mut encoder,
            &size,
            &view,
            Some(&depth),
            &vello_gpu::TextureBindings::new(),
            vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
        )?;
        let row = (side * 4).next_multiple_of(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(side),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            extent,
        );
        self.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let mapped = buffer.get_mapped_range(..).expect("mapped");
        Ok(mapped
            .chunks_exact(row as usize)
            .flat_map(|r| &r[..usize::from(SIDE) * 4])
            .copied()
            .collect())
    }
}
