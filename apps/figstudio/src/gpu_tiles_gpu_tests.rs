//! GPU tiles drawn through the app's path, then by egui-wgpu's own renderer next to the CPU
//! tile uploaded as egui does: what reaches the window must match (colour space included).
//! Skips, with a printed reason, without a GPU adapter.

use eframe::egui_wgpu;
use egui::epaint::{ClippedPrimitive, Mesh, Primitive};

use super::*;

const SIDE: u32 = TEXTURE as u32;

struct Device {
    device: wgpu::Device,
    queue: wgpu::Queue,
    egui: egui_wgpu::Renderer,
}

fn device() -> Option<Device> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("SKIPPED: no GPU adapter");
        return None;
    };
    let Ok((device, queue)) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        eprintln!("SKIPPED: no GPU device");
        return None;
    };
    // The window's usual format (egui prefers a non-sRGB target), no dithering noise.
    let options = egui_wgpu::RendererOptions {
        dithering: false,
        ..Default::default()
    };
    let egui = egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, options);
    Some(Device {
        device,
        queue,
        egui,
    })
}

impl Device {
    /// Draws texture `id` pixel for pixel into a `SIDE` square target, as the canvas would at
    /// 1:1, and reads the target back.
    fn show(&mut self, id: egui::TextureId) -> Vec<u8> {
        let full = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(516.0, 516.0));
        let mut mesh = Mesh::with_texture(id);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        mesh.add_rect_with_uv(full, uv, egui::Color32::WHITE);
        let jobs = [ClippedPrimitive {
            clip_rect: full,
            primitive: Primitive::Mesh(mesh),
        }];
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [SIDE, SIDE],
            pixels_per_point: 1.0,
        };
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let extra =
            self.egui
                .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.egui
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        self.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
        self.read(&target)
    }

    fn read(&self, texture: &wgpu::Texture) -> Vec<u8> {
        let row = (SIDE * 4).next_multiple_of(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(SIDE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
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
            extent(),
        );
        self.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mapped = buffer.get_mapped_range(..).unwrap();
        mapped
            .chunks_exact(row as usize)
            .flat_map(|r| &r[..SIDE as usize * 4])
            .copied()
            .collect()
    }
}

fn extent() -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: SIDE,
        height: SIDE,
        depth_or_array_layers: 1,
    }
}

/// Largest channel difference and the 99.9th percentile.
fn diff(a: &[u8], b: &[u8]) -> (u8, u8) {
    let mut d: Vec<u8> = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).collect();
    d.sort_unstable();
    (d[d.len() - 1], d[d.len() * 999 / 1000])
}

/// Every content tile of the fixtures without images, at a few levels: the GPU tile drawn by
/// the app's path and shown by egui matches the CPU tile shown by egui: max 2/255 as in L2,
/// p99.9 2 (measured 2 on `sections` at level -1, scale √½, which L2 did not sample).
#[test]
fn gpu_tiles_reach_the_window_like_cpu_tiles() {
    let Some(mut dev) = device() else { return };
    let (mut gpu_tiles, mut compared) = (0, 0);
    for name in ["vector", "corners", "sections", "structure"] {
        let doc = open(name);
        for level in [-1, 0, 2] {
            let keys = tiles_of(&doc, level);
            let mut tile_gpu = TileGpu::on(&dev.device, &dev.queue);
            let mut recordings = Recordings::default();
            let job = job(&doc, &keys);
            for key in &keys {
                let drawn = tile_gpu.draw(&mut recordings, &mut renderer(), &job, key);
                let (pixels, stats, _) = drawn.into_parts();
                let Pixels::Gpu(texture) = pixels else {
                    // Only images refuse among these fixtures; counted as a fallback.
                    assert_eq!((name, stats.gpu_fallback_tiles), ("structure", 1));
                    continue;
                };
                gpu_tiles += 1;
                let view = texture.create_view(&Default::default());
                let gpu_id =
                    dev.egui
                        .register_native_texture(&dev.device, &view, wgpu::FilterMode::Linear);
                let cpu = cpu_tile(&doc, &keys, key);
                let image = egui::ColorImage::from_rgba_premultiplied([516, 516], &cpu);
                let cpu_id = egui::TextureId::User(u64::MAX);
                let delta = egui::epaint::ImageDelta::full(image, egui::TextureOptions::LINEAR);
                dev.egui
                    .update_texture(&dev.device, &dev.queue, cpu_id, &delta);
                let (on_gpu, on_cpu) = (dev.show(gpu_id), dev.show(cpu_id));
                dev.egui.free_texture(&gpu_id);
                dev.egui.free_texture(&cpu_id);
                let ink = cpu.as_chunks::<4>().0.iter().any(|p| p[..] != cpu[..4]);
                compared += usize::from(ink);
                // Colour space: egui shows each tile's bytes as they are (no sRGB decode of
                // the GPU texture, no different handling of the two kinds).
                let raw_gpu = dev.read(&texture);
                assert!(
                    diff(&on_gpu, &raw_gpu).0 <= 1,
                    "{name}: GPU tile shown altered"
                );
                assert!(diff(&on_cpu, &cpu).0 <= 1, "{name}: CPU tile shown altered");
                let (max, p999) = diff(&on_gpu, &on_cpu);
                assert!(
                    max <= 2 && p999 <= 2,
                    "{name} level {level} tile ({}, {}): max {max}, p99.9 {p999}",
                    key.x,
                    key.y
                );
            }
        }
    }
    eprintln!("{gpu_tiles} GPU tiles, {compared} with content");
    assert!(compared > 10, "tiles with content were compared");
}

/// A wgpu error outside any error scope goes to our handler (logged, GPU tiles off) instead of
/// wgpu's default handler, which panics (it would kill the app on a failed surface configure).
#[test]
fn an_uncaptured_wgpu_error_turns_the_gpu_off_instead_of_panicking() {
    let Some(d) = device() else { return };
    let lost = Arc::new(AtomicBool::new(false));
    crate::gpu_common::catch_uncaptured(&d.device, lost.clone());
    // A zero-sized texture is a validation error, raised outside any error scope.
    let _bad = d.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 0,
            height: 0,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let _ = d.device.poll(wgpu::PollType::wait_indefinitely());
    assert!(
        lost.load(Ordering::Acquire),
        "the error reached the handler"
    );
}
