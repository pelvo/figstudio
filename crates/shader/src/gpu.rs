//! The GPU side of the WebGPU calls a shader makes: a headless wgpu device, the objects the
//! script created (by index), and render passes recorded until `queue.submit` replays them.

use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::ShaderError;

/// The shared headless device, `None` when this machine has no GPU adapter.
pub(crate) fn device() -> Option<&'static (wgpu::Device, wgpu::Queue)> {
    static DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle().with_env(),
            );
            let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
                .map_err(|e| tracing::warn!("shader fills: no GPU adapter ({e})"))
                .ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .map_err(|e| tracing::warn!("shader fills: no GPU device ({e})"))
                    .ok()?;
            // Errors outside a run's error scope are logged, never a panic.
            device.on_uncaptured_error(Arc::new(|e| tracing::warn!("shader GPU error: {e}")));
            Some((device, queue))
        })
        .as_ref()
}

/// Something the script created.
pub(crate) enum Object {
    Module(wgpu::ShaderModule),
    Buffer(wgpu::Buffer),
    Texture(wgpu::Texture),
    View(wgpu::TextureView),
    Sampler(wgpu::Sampler),
    Pipeline(wgpu::RenderPipeline),
    Layout(wgpu::BindGroupLayout),
    BindGroup(wgpu::BindGroup),
    /// A command encoder: its render passes, replayed on submit.
    Encoder(Vec<Pass>),
}

/// A recorded render pass.
pub(crate) struct Pass {
    pub targets: Vec<Target>,
    /// Depth attachment: view, clear depth (`None` loads), store.
    pub depth: Option<(usize, Option<f32>, bool)>,
    pub ops: Vec<Op>,
}

/// A colour attachment: view, clear colour (`None` loads), store, resolve view.
pub(crate) struct Target {
    pub view: usize,
    pub clear: Option<wgpu::Color>,
    pub store: bool,
    pub resolve: Option<usize>,
}

/// A recorded render-pass command.
pub(crate) enum Op {
    Pipeline(usize),
    BindGroup(u32, usize),
    Vertex(u32, usize, u64),
    Index(usize, wgpu::IndexFormat, u64),
    Draw([u32; 4]),
    DrawIndexed([u32; 3], i32, u32),
}

/// Most GPU memory (buffers and textures) one shader run may ask for.
pub(crate) const MAX_GPU_BYTES: u64 = 512 * 1024 * 1024;

/// One shader run: the device and the objects by index.
pub(crate) struct Session {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub objects: Vec<Object>,
    /// GPU memory asked for so far (see [`MAX_GPU_BYTES`]).
    pub gpu_bytes: u64,
}

fn bad(message: impl Into<String>) -> ShaderError {
    ShaderError::script(message)
}

impl Session {
    /// Counts `bytes` of GPU memory against the run's cap; the script's sizes are its own.
    pub fn reserve(&mut self, bytes: u64) -> Result<(), ShaderError> {
        self.gpu_bytes = self.gpu_bytes.saturating_add(bytes);
        if self.gpu_bytes > MAX_GPU_BYTES {
            return Err(bad("the shader asks for more than 512 MiB of GPU memory"));
        }
        Ok(())
    }

    pub fn add(&mut self, object: Object) -> usize {
        self.objects.push(object);
        self.objects.len() - 1
    }

    fn get(&self, id: usize) -> Result<&Object, ShaderError> {
        self.objects
            .get(id)
            .ok_or_else(|| bad("unknown GPU object"))
    }

    pub fn buffer(&self, id: usize) -> Result<&wgpu::Buffer, ShaderError> {
        match self.get(id)? {
            Object::Buffer(b) => Ok(b),
            _ => Err(bad("not a GPUBuffer")),
        }
    }

    fn module(&self, id: usize) -> Result<&wgpu::ShaderModule, ShaderError> {
        match self.get(id)? {
            Object::Module(m) => Ok(m),
            _ => Err(bad("not a GPUShaderModule")),
        }
    }

    pub fn texture(&self, id: usize) -> Result<&wgpu::Texture, ShaderError> {
        match self.get(id)? {
            Object::Texture(t) => Ok(t),
            _ => Err(bad("not a GPUTexture")),
        }
    }

    pub fn pipeline(&self, id: usize) -> Result<&wgpu::RenderPipeline, ShaderError> {
        match self.get(id)? {
            Object::Pipeline(p) => Ok(p),
            _ => Err(bad("not a GPURenderPipeline")),
        }
    }

    pub fn create_texture(&mut self, desc: &Value) -> Result<usize, ShaderError> {
        let size = &desc["size"];
        let (w, h) = match size {
            Value::Array(v) => (num(v.first()), num(v.get(1)).max(1.0)),
            _ => (num(size.get("width")), num(size.get("height")).max(1.0)),
        };
        let limit = self.device.limits().max_texture_dimension_2d;
        if w > f64::from(limit) || h > f64::from(limit) {
            return Err(bad("texture larger than the device allows"));
        }
        let mips = (num(desc.get("mipLevelCount")) as u32).max(1);
        let format = texture_format(desc["format"].as_str())?;
        let samples = (num(desc.get("sampleCount")) as u32).max(1);
        // Mip levels add at most a third more.
        let texel = u64::from(format.block_copy_size(None).unwrap_or(16));
        self.reserve((w.max(1.0) * h.max(1.0)) as u64 * texel * u64::from(samples) * 4 / 3)?;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: (w as u32).max(1),
                height: (h as u32).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: mips.min(16),
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::from_bits_truncate(num(desc.get("usage")) as u32),
            view_formats: &[],
        });
        Ok(self.add(Object::Texture(texture)))
    }

    pub fn create_sampler(&mut self, desc: &Value) -> usize {
        let filter = |k: &str| match desc[k].as_str() {
            Some("linear") => wgpu::FilterMode::Linear,
            _ => wgpu::FilterMode::Nearest,
        };
        let mipmap = match desc["mipmapFilter"].as_str() {
            Some("linear") => wgpu::MipmapFilterMode::Linear,
            _ => wgpu::MipmapFilterMode::Nearest,
        };
        let address = |k: &str| match desc[k].as_str() {
            Some("repeat") => wgpu::AddressMode::Repeat,
            Some("mirror-repeat") => wgpu::AddressMode::MirrorRepeat,
            _ => wgpu::AddressMode::ClampToEdge,
        };
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: address("addressModeU"),
            address_mode_v: address("addressModeV"),
            address_mode_w: address("addressModeW"),
            mag_filter: filter("magFilter"),
            min_filter: filter("minFilter"),
            mipmap_filter: mipmap,
            ..Default::default()
        });
        self.add(Object::Sampler(sampler))
    }

    pub fn create_pipeline(&mut self, desc: &Value) -> Result<usize, ShaderError> {
        if desc["layout"].as_str() != Some("auto") {
            // shortcut: explicit pipeline layouts are not built; add createPipelineLayout when a
            // shader with source uses one.
            return Err(bad("only layout: 'auto' pipelines are supported"));
        }
        let vertex = &desc["vertex"];
        let buffers = vertex["buffers"].as_array().map_or(&[][..], Vec::as_slice);
        let mut attributes = Vec::new();
        for b in buffers {
            let mut list = Vec::new();
            for a in b["attributes"].as_array().map_or(&[][..], Vec::as_slice) {
                list.push(wgpu::VertexAttribute {
                    format: vertex_format(a["format"].as_str())?,
                    offset: num(a.get("offset")) as u64,
                    shader_location: num(a.get("shaderLocation")) as u32,
                });
            }
            attributes.push(list);
        }
        let layouts: Vec<_> = buffers
            .iter()
            .zip(&attributes)
            .map(|(b, list)| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: num(b.get("arrayStride")) as u64,
                    step_mode: if b["stepMode"].as_str() == Some("instance") {
                        wgpu::VertexStepMode::Instance
                    } else {
                        wgpu::VertexStepMode::Vertex
                    },
                    attributes: list,
                })
            })
            .collect();
        let fragment = &desc["fragment"];
        let mut targets = Vec::new();
        for t in fragment["targets"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
        {
            targets.push(Some(wgpu::ColorTargetState {
                format: texture_format(t["format"].as_str())?,
                blend: blend(&t["blend"])?,
                write_mask: wgpu::ColorWrites::ALL,
            }));
        }
        let depth_stencil = if desc["depthStencil"].is_object() {
            let d = &desc["depthStencil"];
            Some(wgpu::DepthStencilState {
                format: texture_format(d["format"].as_str())?,
                depth_write_enabled: Some(d["depthWriteEnabled"].as_bool() == Some(true)),
                depth_compare: Some(compare(d["depthCompare"].as_str())),
                stencil: Default::default(),
                bias: Default::default(),
            })
        } else {
            None
        };
        let multisample = wgpu::MultisampleState {
            count: (num(desc["multisample"].get("count")) as u32).max(1),
            ..Default::default()
        };
        let topology = match desc["primitive"]["topology"].as_str() {
            Some("triangle-strip") => wgpu::PrimitiveTopology::TriangleStrip,
            Some("line-list") => wgpu::PrimitiveTopology::LineList,
            Some("line-strip") => wgpu::PrimitiveTopology::LineStrip,
            Some("point-list") => wgpu::PrimitiveTopology::PointList,
            _ => wgpu::PrimitiveTopology::TriangleList,
        };
        let vertex_module = self.module(handle(&vertex["module"])?)?;
        let fragment_module = if fragment.is_object() {
            Some(self.module(handle(&fragment["module"])?)?)
        } else {
            None
        };
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState {
                    module: vertex_module,
                    entry_point: vertex["entryPoint"].as_str(),
                    compilation_options: Default::default(),
                    buffers: &layouts,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil,
                multisample,
                fragment: fragment_module.map(|module| wgpu::FragmentState {
                    module,
                    entry_point: fragment["entryPoint"].as_str(),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            });
        Ok(self.add(Object::Pipeline(pipeline)))
    }

    pub fn create_bind_group(&mut self, desc: &Value) -> Result<usize, ShaderError> {
        let layout = match self.get(handle(&desc["layout"])?)? {
            Object::Layout(l) => l,
            _ => return Err(bad("not a GPUBindGroupLayout")),
        };
        let mut entries = Vec::new();
        for e in desc["entries"].as_array().map_or(&[][..], Vec::as_slice) {
            let resource = &e["resource"];
            let binding = num(e.get("binding")) as u32;
            let resource = if resource.get("buffer").is_some() {
                let buffer = self.buffer(handle(&resource["buffer"])?)?;
                let size = resource["size"]
                    .as_f64()
                    .and_then(|s| wgpu::BufferSize::new(s as u64));
                wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: num(resource.get("offset")) as u64,
                    size,
                })
            } else {
                match self.get(handle(resource)?)? {
                    Object::Sampler(s) => wgpu::BindingResource::Sampler(s),
                    Object::View(v) => wgpu::BindingResource::TextureView(v),
                    _ => return Err(bad("unsupported bind group resource")),
                }
            };
            entries.push(wgpu::BindGroupEntry { binding, resource });
        }
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &entries,
        });
        Ok(self.add(Object::BindGroup(group)))
    }

    /// Replays the recorded passes of the command buffers `ids` and submits them.
    pub fn submit(&mut self, ids: &[usize]) -> Result<(), ShaderError> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for &id in ids {
            let Object::Encoder(passes) = self.get(id)? else {
                return Err(bad("not a GPUCommandBuffer"));
            };
            for pass in passes {
                self.replay(&mut encoder, pass)?;
            }
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn replay(&self, encoder: &mut wgpu::CommandEncoder, pass: &Pass) -> Result<(), ShaderError> {
        let store = |s: bool| {
            if s {
                wgpu::StoreOp::Store
            } else {
                wgpu::StoreOp::Discard
            }
        };
        let mut attachments = Vec::new();
        for t in &pass.targets {
            attachments.push(Some(wgpu::RenderPassColorAttachment {
                view: self.view(t.view)?,
                depth_slice: None,
                resolve_target: t.resolve.map(|r| self.view(r)).transpose()?,
                ops: wgpu::Operations {
                    load: t.clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                    store: store(t.store),
                },
            }));
        }
        let depth = match pass.depth {
            Some((view, clear, keep)) => Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.view(view)?,
                depth_ops: Some(wgpu::Operations {
                    load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                    store: store(keep),
                }),
                stencil_ops: None,
            }),
            None => None,
        };
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &attachments,
            depth_stencil_attachment: depth,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for op in &pass.ops {
            match *op {
                Op::Pipeline(id) => rp.set_pipeline(self.pipeline(id)?),
                Op::BindGroup(index, id) => {
                    let Object::BindGroup(group) = self.get(id)? else {
                        return Err(bad("not a GPUBindGroup"));
                    };
                    rp.set_bind_group(index, group, &[]);
                }
                Op::Vertex(slot, id, offset) => {
                    rp.set_vertex_buffer(slot, self.slice(id, offset)?);
                }
                Op::Index(id, format, offset) => {
                    rp.set_index_buffer(self.slice(id, offset)?, format);
                }
                Op::Draw([vertices, instances, first_vertex, first_instance]) => rp.draw(
                    first_vertex..first_vertex + vertices,
                    first_instance..first_instance + instances,
                ),
                Op::DrawIndexed([indices, instances, first_index], base, first_instance) => rp
                    .draw_indexed(
                        first_index..first_index + indices,
                        base,
                        first_instance..first_instance + instances,
                    ),
            }
        }
        Ok(())
    }

    fn view(&self, id: usize) -> Result<&wgpu::TextureView, ShaderError> {
        match self.get(id)? {
            Object::View(v) => Ok(v),
            _ => Err(bad("not a GPUTextureView")),
        }
    }

    /// `queue.writeTexture`: `data` laid out with `bytes_per_row` into `size` texels of
    /// texture `id` at mip `level` and `origin`.
    pub fn write_texture(
        &self,
        id: usize,
        level: u32,
        origin: (u32, u32),
        data: &[u8],
        layout: (u64, Option<u32>, Option<u32>),
        size: (u32, u32),
    ) -> Result<(), ShaderError> {
        let texture = self.texture(id)?;
        if level >= texture.mip_level_count() {
            return Err(bad("writeTexture mip level past the texture's"));
        }
        let (tw, th) = (
            (texture.width() >> level).max(1),
            (texture.height() >> level).max(1),
        );
        if origin.0 + size.0 > tw || origin.1 + size.1 > th {
            return Err(bad("writeTexture past the texture's edge"));
        }
        let block = texture.format().block_copy_size(None).unwrap_or(4);
        let row = layout.1.unwrap_or(size.0 * block);
        let rows = layout.2.unwrap_or(size.1);
        let needed = layout.0
            + u64::from(row) * u64::from(rows.saturating_sub(1))
            + u64::from(size.0 * block);
        if size.0 * block > row || (data.len() as u64) < needed {
            return Err(bad("writeTexture data smaller than its layout"));
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d {
                    x: origin.0,
                    y: origin.1,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: layout.0,
                bytes_per_row: Some(row),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// A buffer from `offset` on; wgpu panics on an offset past the end, so it is checked.
    fn slice(&self, id: usize, offset: u64) -> Result<wgpu::BufferSlice<'_>, ShaderError> {
        let buffer = self.buffer(id)?;
        if offset >= buffer.size() {
            return Err(bad("buffer offset past its end"));
        }
        Ok(buffer.slice(offset..))
    }

    /// The straight-alpha RGBA8 pixels of texture `id` (an `rgba8unorm` texture).
    pub fn read(&self, id: usize) -> Result<Vec<u8>, ShaderError> {
        let texture = self.texture(id)?;
        let (w, h) = (texture.width(), texture.height());
        let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(h),
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
                    rows_per_image: Some(h),
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| ShaderError::gpu(format!("waiting for the GPU: {e}")))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| ShaderError::gpu(format!("reading the output: {e}")))?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let start = y * row as usize;
            out.extend_from_slice(&mapped[start..start + w as usize * 4]);
        }
        Ok(out)
    }
}

/// The object index a handle (`{__id: n}`) stands for.
pub(crate) fn handle(value: &Value) -> Result<usize, ShaderError> {
    value["__id"]
        .as_u64()
        .map(|id| id as usize)
        .ok_or_else(|| bad("expected a GPU object"))
}

pub(crate) fn num(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0).max(0.0)
}

pub(crate) fn texture_format(name: Option<&str>) -> Result<wgpu::TextureFormat, ShaderError> {
    use wgpu::TextureFormat as F;
    Ok(match name {
        Some("rgba8unorm") => F::Rgba8Unorm,
        Some("rgba8unorm-srgb") => F::Rgba8UnormSrgb,
        Some("bgra8unorm") => F::Bgra8Unorm,
        Some("bgra8unorm-srgb") => F::Bgra8UnormSrgb,
        Some("rgba16float") => F::Rgba16Float,
        Some("rgba32float") => F::Rgba32Float,
        Some("r8unorm") => F::R8Unorm,
        Some("r16float") => F::R16Float,
        Some("r32float") => F::R32Float,
        Some("rg16float") => F::Rg16Float,
        Some("rg32float") => F::Rg32Float,
        Some("depth24plus") => F::Depth24Plus,
        Some("depth32float") => F::Depth32Float,
        Some("depth24plus-stencil8") => F::Depth24PlusStencil8,
        Some("r8uint") => F::R8Uint,
        Some("rgba8snorm") => F::Rgba8Snorm,
        other => return Err(bad(format!("texture format {other:?} not supported"))),
    })
}

fn vertex_format(name: Option<&str>) -> Result<wgpu::VertexFormat, ShaderError> {
    use wgpu::VertexFormat as F;
    Ok(match name {
        Some("float32") => F::Float32,
        Some("float32x2") => F::Float32x2,
        Some("float32x3") => F::Float32x3,
        Some("float32x4") => F::Float32x4,
        Some("uint32") => F::Uint32,
        Some("uint32x2") => F::Uint32x2,
        Some("uint32x3") => F::Uint32x3,
        Some("uint32x4") => F::Uint32x4,
        Some("sint32") => F::Sint32,
        Some("sint32x2") => F::Sint32x2,
        Some("unorm8x4") => F::Unorm8x4,
        other => return Err(bad(format!("vertex format {other:?} not supported"))),
    })
}

fn compare(name: Option<&str>) -> wgpu::CompareFunction {
    use wgpu::CompareFunction as C;
    match name {
        Some("never") => C::Never,
        Some("less") => C::Less,
        Some("equal") => C::Equal,
        Some("less-equal") => C::LessEqual,
        Some("greater") => C::Greater,
        Some("not-equal") => C::NotEqual,
        Some("greater-equal") => C::GreaterEqual,
        _ => C::Always,
    }
}

fn blend(value: &Value) -> Result<Option<wgpu::BlendState>, ShaderError> {
    if !value.is_object() {
        return Ok(None);
    }
    let component = |c: &Value| -> Result<wgpu::BlendComponent, ShaderError> {
        let factor = |k: &str, default| -> Result<wgpu::BlendFactor, ShaderError> {
            use wgpu::BlendFactor as F;
            Ok(match c[k].as_str() {
                None => default,
                Some("zero") => F::Zero,
                Some("one") => F::One,
                Some("src") => F::Src,
                Some("one-minus-src") => F::OneMinusSrc,
                Some("src-alpha") => F::SrcAlpha,
                Some("one-minus-src-alpha") => F::OneMinusSrcAlpha,
                Some("dst") => F::Dst,
                Some("one-minus-dst") => F::OneMinusDst,
                Some("dst-alpha") => F::DstAlpha,
                Some("one-minus-dst-alpha") => F::OneMinusDstAlpha,
                Some(other) => return Err(bad(format!("blend factor {other} not supported"))),
            })
        };
        use wgpu::BlendOperation as O;
        Ok(wgpu::BlendComponent {
            src_factor: factor("srcFactor", wgpu::BlendFactor::One)?,
            dst_factor: factor("dstFactor", wgpu::BlendFactor::Zero)?,
            operation: match c["operation"].as_str() {
                Some("subtract") => O::Subtract,
                Some("reverse-subtract") => O::ReverseSubtract,
                Some("min") => O::Min,
                Some("max") => O::Max,
                _ => O::Add,
            },
        })
    };
    Ok(Some(wgpu::BlendState {
        color: component(&value["color"])?,
        alpha: component(&value["alpha"])?,
    }))
}
