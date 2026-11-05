//! The two stages of `mesh-gradient`, each alone, so a driver that leaves it blank shows which
//! one: an indexed mesh from vertex, index and uniform buffers, and a 4× multisampled target
//! resolved by hand with `textureLoad`. Each draws opaque red over the whole output. Skipped
//! without a GPU.

use figstudio_shader::render;
use serde_json::json;

/// The centre pixel `main` draws on 8 × 8; `None` without a GPU.
fn centre(main: &str) -> Option<[u8; 4]> {
    let out = match render(main, &json!({}), 8, 8, 0.0) {
        Err(e) if e.skips_test() => return None,
        other => other.unwrap(),
    };
    let i = (4 * 8 + 4) * 4;
    Some(out.data_as_u8_slice()[i..i + 4].try_into().unwrap())
}

const RED: [u8; 4] = [255, 0, 0, 255];

#[test]
fn an_indexed_mesh_from_vertex_index_and_uniform_buffers_is_drawn() {
    worker();
    let main = r"
export function render(device: any, frame: any) {
  const code = `
struct U { color: vec4f };
@group(0) @binding(0) var<uniform> u: U;
@vertex fn vs(@location(0) st: vec2f) -> @builtin(position) vec4f {
  return vec4f(st.x * 2.0 - 1.0, 1.0 - st.y * 2.0, 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4f { return u.color; }`;
  const module = device.createShaderModule({ code });
  const vbuf = device.createBuffer({ size: 32, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
  device.queue.writeBuffer(vbuf, 0, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]));
  const ibuf = device.createBuffer({ size: 24, usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST });
  device.queue.writeBuffer(ibuf, 0, new Uint32Array([0, 2, 1, 1, 2, 3]));
  const ubuf = device.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  device.queue.writeBuffer(ubuf, 0, new Float32Array([1, 0, 0, 1]));
  const pipeline = device.createRenderPipeline({ layout: 'auto',
    vertex: { module, entryPoint: 'vs',
      buffers: [{ arrayStride: 8, attributes: [{ shaderLocation: 0, format: 'float32x2', offset: 0 }] }] },
    fragment: { module, entryPoint: 'fs', targets: [{ format: frame.output.format }] },
    primitive: { topology: 'triangle-list' } });
  const bind = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0),
    entries: [{ binding: 0, resource: { buffer: ubuf } }] });
  const e = device.createCommandEncoder();
  const p = e.beginRenderPass({ colorAttachments: [{ view: frame.output.createView(),
    loadOp: 'clear', clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: 'store' }] });
  p.setPipeline(pipeline);
  p.setBindGroup(0, bind);
  p.setVertexBuffer(0, vbuf);
  p.setIndexBuffer(ibuf, 'uint32');
  p.drawIndexed(6);
  p.end();
  device.queue.submit([e.finish()]);
}";
    if let Some(px) = centre(main) {
        assert_eq!(px, RED);
    }
}

#[test]
fn a_multisampled_target_resolved_with_texture_load_is_drawn() {
    worker();
    let main = r"
export function render(device: any, frame: any) {
  const draw = device.createShaderModule({ code: `
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(p[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4f { return vec4f(1.0, 0.0, 0.0, 1.0); }` });
  const resolve = device.createShaderModule({ code: `
@group(0) @binding(0) var ms: texture_multisampled_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(p[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4f) -> @location(0) vec4f {
  var sum = vec4f(0.0);
  for (var s = 0; s < 4; s = s + 1) { sum = sum + textureLoad(ms, vec2i(at.xy), s); }
  return sum / 4.0;
}` });
  const format = frame.output.format;
  const ms = device.createTexture({ size: { width: 8, height: 8 }, format, sampleCount: 4,
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING });
  const meshPipeline = device.createRenderPipeline({ layout: 'auto',
    vertex: { module: draw, entryPoint: 'vs' },
    fragment: { module: draw, entryPoint: 'fs', targets: [{ format }] },
    primitive: { topology: 'triangle-list' }, multisample: { count: 4 } });
  const resolvePipeline = device.createRenderPipeline({ layout: 'auto',
    vertex: { module: resolve, entryPoint: 'vs' },
    fragment: { module: resolve, entryPoint: 'fs', targets: [{ format }] },
    primitive: { topology: 'triangle-list' } });
  const bind = device.createBindGroup({ layout: resolvePipeline.getBindGroupLayout(0),
    entries: [{ binding: 0, resource: ms.createView() }] });
  const e = device.createCommandEncoder();
  const p1 = e.beginRenderPass({ colorAttachments: [{ view: ms.createView(),
    loadOp: 'clear', clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: 'store' }] });
  p1.setPipeline(meshPipeline);
  p1.draw(3);
  p1.end();
  const p2 = e.beginRenderPass({ colorAttachments: [{ view: frame.output.createView(),
    loadOp: 'clear', clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: 'store' }] });
  p2.setPipeline(resolvePipeline);
  p2.setBindGroup(0, bind);
  p2.draw(3);
  p2.end();
  device.queue.submit([e.finish()]);
}";
    if let Some(px) = centre(main) {
        assert_eq!(px, RED);
    }
}

/// The shader process for these tests: this crate's worker binary (a test binary cannot be one).
fn worker() {
    figstudio_shader::set_worker_program(env!("CARGO_BIN_EXE_figstudio-shader-worker").into());
}
