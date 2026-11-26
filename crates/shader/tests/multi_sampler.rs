//! A single WGSL texture sampled with multiple samplers works on every backend, including
//! OpenGL/GLES where Naga requires one texture binding per sampler and `wgpu-hal::gles` maps each
//! texture slot to at most one sampler across the pipeline. Skipped without a GPU.

use std::path::PathBuf;
use std::sync::Arc;

use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::shaders::ShaderSource;
use figstudio_render::Renderer;
use figstudio_render::shader::ShaderFills;
use figstudio_shader::{Fills, pixmap, render, render_effect};
use serde_json::json;
use vello_cpu::Pixmap;

/// 4x4 premultiplied RGBA input: left half red, right half blue.
fn split_input() -> Pixmap {
    let mut rgba = Vec::with_capacity(4 * 4 * 4);
    for _y in 0..4 {
        for x in 0..4 {
            if x < 2 {
                rgba.extend_from_slice(&[255, 0, 0, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 255, 255]);
            }
        }
    }
    pixmap(rgba, 4, 4, true).expect("valid 4x4 pixmap")
}

fn shader_js(wgsl: &str, tex_binding: u32) -> String {
    format!(
        r#"
export function render(device: any, frame: any) {{
  const module = device.createShaderModule({{ code: `{wgsl}` }});
  const repeat = device.createSampler({{
    addressModeU: 'repeat',
    addressModeV: 'repeat',
    magFilter: 'nearest',
    minFilter: 'nearest',
  }});
  const clamp = device.createSampler({{
    addressModeU: 'clamp-to-edge',
    addressModeV: 'clamp-to-edge',
    magFilter: 'nearest',
    minFilter: 'nearest',
  }});
  const pipeline = device.createRenderPipeline({{
    layout: 'auto',
    vertex: {{ module, entryPoint: 'vs' }},
    fragment: {{ module, entryPoint: 'fs', targets: [{{ format: frame.output.format }}] }},
    primitive: {{ topology: 'triangle-list' }},
  }});
  const bind = device.createBindGroup({{
    layout: pipeline.getBindGroupLayout(0),
    entries: [
      {{ binding: {tex_binding}, resource: frame.input.createView() }},
      {{ binding: 1, resource: repeat }},
      {{ binding: 2, resource: clamp }},
    ],
  }});
  const encoder = device.createCommandEncoder();
  const pass = encoder.beginRenderPass({{
    colorAttachments: [{{
      view: frame.output.createView(),
      loadOp: 'clear',
      clearValue: {{ r: 0, g: 0, b: 0, a: 0 }},
      storeOp: 'store',
    }}],
  }});
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, bind);
  pass.draw(3);
  pass.end();
  device.queue.submit([encoder.finish()]);
}}
"#
    )
}

fn assert_split_samples(wgsl: &str, tex_binding: u32) {
    worker();
    let input = split_input();
    let main = shader_js(wgsl, tex_binding);
    let out = match render_effect(&main, &json!({}), &input, 0.0) {
        Ok(p) => p,
        Err(e) if e.skips_test() => return,
        Err(e) => panic!("shader failed: {e}"),
    };
    let left = out.sample(0, 2);
    let right = out.sample(3, 2);
    assert_eq!(
        [left.r, left.g, left.b, left.a],
        [255, 0, 0, 255],
        "repeat sampler at u=1.25 should wrap into the red left half"
    );
    assert_eq!(
        [right.r, right.g, right.b, right.a],
        [0, 0, 255, 255],
        "clamp-to-edge sampler at u=1.25 should clamp into the blue right half"
    );
}

#[test]
fn one_texture_sampled_with_two_samplers_honors_both_address_modes() {
    let wgsl = r"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var repeat_samp: sampler;
@group(0) @binding(2) var clamp_samp: sampler;

struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4f(p[i], 0.0, 1.0);
  o.uv = p[i] * 0.5 + vec2f(0.5);
  return o;
}

@fragment fn fs(in: VsOut) -> @location(0) vec4f {
  let dims = textureDimensions(tex, 0);
  if (dims.x == 0u) {
    return vec4f(0.0);
  }
  if (in.uv.x < 0.5) {
    return textureSampleLevel(tex, repeat_samp, vec2f(1.25, 0.5), 0.0);
  }
  return textureSampleLevel(tex, clamp_samp, vec2f(1.25, 0.5), 0.0);
}
";
    assert_split_samples(wgsl, 0);
}

#[test]
fn one_texture_sampled_with_two_samplers_through_helpers_and_type_aliases_works() {
    // Covers (texture, sampler) helpers, global-texture + param-sampler helpers, nested helpers,
    // and type-aliased samplers (`alias S = sampler`).
    let cases = [
        r"
alias S = sampler;
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var repeat_samp: S;
@group(0) @binding(2) var clamp_samp: S;

struct VsOut { @builtin(position) pos: vec4f, @location(0) uv: vec2f };
@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4f(p[i], 0.0, 1.0);
  o.uv = p[i] * 0.5 + vec2f(0.5);
  return o;
}
fn sample_at(t: texture_2d<f32>, s: S, uv: vec2f) -> vec4f {
  let _d = textureDimensions(t, 0);
  return textureSampleLevel(t, s, uv, 0.0);
}
@fragment fn fs(in: VsOut) -> @location(0) vec4f {
  if (in.uv.x < 0.5) { return sample_at(tex, repeat_samp, vec2f(1.25, 0.5)); }
  return sample_at(tex, clamp_samp, vec2f(1.25, 0.5));
}
",
        r"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var repeat_samp: sampler;
@group(0) @binding(2) var clamp_samp: sampler;

struct VsOut { @builtin(position) pos: vec4f, @location(0) uv: vec2f };
@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4f(p[i], 0.0, 1.0);
  o.uv = p[i] * 0.5 + vec2f(0.5);
  return o;
}
fn leaf(t: texture_2d<f32>, s: sampler, uv: vec2f) -> vec4f {
  return textureSampleLevel(t, s, uv, 0.0);
}
fn middle(s: sampler, uv: vec2f) -> vec4f {
  return leaf(tex, s, uv);
}
@fragment fn fs(in: VsOut) -> @location(0) vec4f {
  if (in.uv.x < 0.5) { return middle(repeat_samp, vec2f(1.25, 0.5)); }
  return middle(clamp_samp, vec2f(1.25, 0.5));
}
",
    ];
    for wgsl in cases {
        assert_split_samples(wgsl, 0);
    }
}

#[test]
fn multipass_single_module_reusing_bindings_across_active_entry_points_renders_both_passes() {
    worker();
    let main = r#"
export function render(device: any, frame: any) {
  const module = device.createShaderModule({ code: `
@group(0) @binding(0) var pass1_tex: texture_2d<f32>;
@group(0) @binding(1) var repeat_samp: sampler;
@group(0) @binding(2) var clamp_samp: sampler;
@group(0) @binding(0) var<uniform> pass2_tint: vec4f;
@group(0) @binding(1) var pass2_tex: texture_2d<f32>;
@group(0) @binding(2) var pass2_samp: sampler;

struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4f(p[i], 0.0, 1.0);
  o.uv = vec2f(p[i].x * 0.5 + 0.5, 0.5 - p[i].y * 0.5);
  return o;
}

@fragment fn fs_pass1(in: VsOut) -> @location(0) vec4f {
  if (in.uv.x < 0.5) {
    return textureSampleLevel(pass1_tex, repeat_samp, vec2f(1.25, 0.5), 0.0);
  }
  return textureSampleLevel(pass1_tex, clamp_samp, vec2f(1.25, 0.5), 0.0);
}

@fragment fn fs_pass2(in: VsOut) -> @location(0) vec4f {
  return textureSampleLevel(pass2_tex, pass2_samp, in.uv, 0.0) * pass2_tint;
}
` });
  const repeat = device.createSampler({
    addressModeU: 'repeat',
    addressModeV: 'repeat',
    magFilter: 'nearest',
    minFilter: 'nearest',
  });
  const clamp = device.createSampler({
    addressModeU: 'clamp-to-edge',
    addressModeV: 'clamp-to-edge',
    magFilter: 'nearest',
    minFilter: 'nearest',
  });
  const mid = device.createTexture({
    size: [4, 4],
    format: frame.output.format,
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
  });
  const tint = device.createBuffer({
    size: 16,
    usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
  });
  device.queue.writeBuffer(tint, 0, new Float32Array([1, 1, 1, 1]));

  const p1 = device.createRenderPipeline({
    layout: 'auto',
    vertex: { module, entryPoint: 'vs' },
    fragment: { module, entryPoint: 'fs_pass1', targets: [{ format: frame.output.format }] },
    primitive: { topology: 'triangle-list' },
  });
  const p2 = device.createRenderPipeline({
    layout: 'auto',
    vertex: { module, entryPoint: 'vs' },
    fragment: { module, entryPoint: 'fs_pass2', targets: [{ format: frame.output.format }] },
    primitive: { topology: 'triangle-list' },
  });
  const bg1 = device.createBindGroup({
    layout: p1.getBindGroupLayout(0),
    entries: [
      { binding: 0, resource: frame.input.createView() },
      { binding: 1, resource: repeat },
      { binding: 2, resource: clamp },
    ],
  });
  const bg2 = device.createBindGroup({
    layout: p2.getBindGroupLayout(0),
    entries: [
      { binding: 0, resource: { buffer: tint } },
      { binding: 1, resource: mid.createView() },
      { binding: 2, resource: clamp },
    ],
  });
  const enc = device.createCommandEncoder();
  const r1 = enc.beginRenderPass({
    colorAttachments: [{
      view: mid.createView(),
      loadOp: 'clear',
      clearValue: { r: 0, g: 0, b: 0, a: 0 },
      storeOp: 'store',
    }],
  });
  r1.setPipeline(p1);
  r1.setBindGroup(0, bg1);
  r1.draw(3);
  r1.end();
  const r2 = enc.beginRenderPass({
    colorAttachments: [{
      view: frame.output.createView(),
      loadOp: 'clear',
      clearValue: { r: 0, g: 0, b: 0, a: 0 },
      storeOp: 'store',
    }],
  });
  r2.setPipeline(p2);
  r2.setBindGroup(0, bg2);
  r2.draw(3);
  r2.end();
  device.queue.submit([enc.finish()]);
}
"#;
    let input = split_input();
    let out = match render_effect(main, &json!({}), &input, 0.0) {
        Ok(p) => p,
        Err(e) if e.skips_test() => return,
        Err(e) => panic!("multi-pass reused-binding shader failed: {e}"),
    };
    let left = out.sample(0, 2);
    let right = out.sample(3, 2);
    assert_eq!(
        [left.r, left.g, left.b, left.a],
        [255, 0, 0, 255],
        "pass 1 repeat sample propagated through pass 2 must stay red"
    );
    assert_eq!(
        [right.r, right.g, right.b, right.a],
        [0, 0, 255, 255],
        "pass 1 clamp sample propagated through pass 2 must stay blue"
    );
}

#[test]
fn cross_stage_two_module_pipeline_avoids_binding_collision_and_preserves_both_samplers() {
    worker();
    let main = r#"
export function render(device: any, frame: any) {
  const vsMod = device.createShaderModule({ code: `
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var repeat_samp: sampler;

struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
  @location(1) vs_col: vec4f,
};

@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4f(p[i], 0.0, 1.0);
  o.uv = p[i] * 0.5 + vec2f(0.5);
  o.vs_col = textureSampleLevel(tex, repeat_samp, vec2f(1.25, 0.5), 0.0);
  return o;
}
` });
  const fsMod = device.createShaderModule({ code: `
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(2) var clamp_samp: sampler;
@group(0) @binding(3) var<uniform> tint: vec4f;

struct FsIn {
  @location(0) uv: vec2f,
  @location(1) vs_col: vec4f,
};

@fragment fn fs(in: FsIn) -> @location(0) vec4f {
  if (in.uv.x < 0.5) {
    return in.vs_col * tint;
  }
  return textureSampleLevel(tex, clamp_samp, vec2f(1.25, 0.5), 0.0) * tint;
}
` });
  const repeat = device.createSampler({
    addressModeU: 'repeat',
    addressModeV: 'repeat',
    magFilter: 'nearest',
    minFilter: 'nearest',
  });
  const clamp = device.createSampler({
    addressModeU: 'clamp-to-edge',
    addressModeV: 'clamp-to-edge',
    magFilter: 'nearest',
    minFilter: 'nearest',
  });
  const tint = device.createBuffer({
    size: 16,
    usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
  });
  device.queue.writeBuffer(tint, 0, new Float32Array([1, 1, 1, 1]));
  const pipeline = device.createRenderPipeline({
    layout: 'auto',
    vertex: { module: vsMod, entryPoint: 'vs' },
    fragment: { module: fsMod, entryPoint: 'fs', targets: [{ format: frame.output.format }] },
    primitive: { topology: 'triangle-list' },
  });
  const bind = device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [
      { binding: 0, resource: frame.input.createView() },
      { binding: 1, resource: repeat },
      { binding: 2, resource: clamp },
      { binding: 3, resource: { buffer: tint } },
    ],
  });
  const encoder = device.createCommandEncoder();
  const pass = encoder.beginRenderPass({
    colorAttachments: [{
      view: frame.output.createView(),
      loadOp: 'clear',
      clearValue: { r: 0, g: 0, b: 0, a: 0 },
      storeOp: 'store',
    }],
  });
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, bind);
  pass.draw(3);
  pass.end();
  device.queue.submit([encoder.finish()]);
}
"#;
    let input = split_input();
    let out = match render_effect(main, &json!({}), &input, 0.0) {
        Ok(p) => p,
        Err(e) if e.skips_test() => return,
        Err(e) => panic!("cross-stage shader failed: {e}"),
    };
    let left = out.sample(0, 2);
    let right = out.sample(3, 2);
    assert_eq!(
        [left.r, left.g, left.b, left.a],
        [255, 0, 0, 255],
        "vertex stage must sample with repeat (red), not be overwritten by fragment's clamp sampler"
    );
    assert_eq!(
        [right.r, right.g, right.b, right.a],
        [0, 0, 255, 255],
        "fragment stage must sample with clamp-to-edge (blue) without colliding with binding 3"
    );
}

#[test]
fn high_binding_number_reuses_free_gap_within_device_limit() {
    let wgsl = r"
@group(0) @binding(999) var tex: texture_2d<f32>;
@group(0) @binding(1) var repeat_samp: sampler;
@group(0) @binding(2) var clamp_samp: sampler;

struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var o: VsOut;
  o.pos = vec4f(p[i], 0.0, 1.0);
  o.uv = p[i] * 0.5 + vec2f(0.5);
  return o;
}

@fragment fn fs(in: VsOut) -> @location(0) vec4f {
  if (in.uv.x < 0.5) {
    return textureSampleLevel(tex, repeat_samp, vec2f(1.25, 0.5), 0.0);
  }
  return textureSampleLevel(tex, clamp_samp, vec2f(1.25, 0.5), 0.0);
}
";
    assert_split_samples(wgsl, 999);
}

#[test]
fn pattern_refraction_draws_exact_patterned_pixels_on_fixture_and_synthetic_input() {
    worker();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let main_ts =
        std::fs::read_to_string(root.join("shader-sources/pattern-refraction/main.ts")).unwrap();
    let features =
        std::fs::read_to_string(root.join("shader-sources/pattern-refraction/features.json"))
            .unwrap();

    let doc = Document::open(root.join("shaders.fig")).unwrap();
    let guid: Guid = "1:14641".parse().unwrap();
    let probe = render("export function render() {}", &json!({}), 1, 1, 0.0);
    if probe.as_ref().is_err_and(|e| e.skips_test()) {
        return;
    }
    let fills: Arc<dyn ShaderFills> = Arc::new(Fills::new(vec![ShaderSource::new(
        "pattern-refraction".into(),
        vec![
            ("main.ts".into(), main_ts.clone()),
            ("features.json".into(), features),
        ],
    )]));
    let draw = |host: Option<Arc<dyn ShaderFills>>| {
        let mut r = Renderer::new();
        r.shaders = host;
        r.render_export_complete(&doc, guid, 0.25, None, |_| None)
            .unwrap()
    };
    let without = draw(None);
    let with = draw(Some(fills));
    assert_eq!(with.stats.skipped_effects, 0, "{:?}", with.stats);
    assert_eq!(with.stats.approximated_effects, 0, "{:?}", with.stats);
    assert_eq!(with.stats.unsupported_paints, 0, "{:?}", with.stats);
    assert_eq!(with.stats.approximated_paints, 0, "{:?}", with.stats);
    assert!(without.stats.skipped_effects > 0, "{:?}", without.stats);
    assert_ne!(
        with.pixmap.data_as_u8_slice(),
        without.pixmap.data_as_u8_slice(),
        "pattern-refraction must alter the rendered pixels"
    );
    let dispersed = with
        .pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] > 0 && px[0].abs_diff(px[2]) >= 8)
        .count();
    assert!(
        dispersed > 100,
        "expected visible RGB dispersion from pattern-refraction, found only {dispersed} pixels"
    );

    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for _y in 0..32 {
        for x in 0..32 {
            if x < 16 {
                rgba.extend_from_slice(&[255, 0, 0, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 255, 255]);
            }
        }
    }
    let input = pixmap(rgba, 32, 32, true).expect("valid 32x32 pixmap");
    let params = |wrap: u32| {
        json!({
            "amount": 50.0,
            "centerHandle": { "angle": 0.0, "radius": 8.0, "x": 50.0, "y": 50.0 },
            "frost": 0.0,
            "iorDispersion": 20.0,
            "patternType": 0.0,
            "pixelWrapMode": wrap,
            "seamlessness": 0.0,
        })
    };
    let repeat_out = render_effect(&main_ts, &params(2), &input, 0.0).expect("repeat wrap renders");
    let clamp_out = render_effect(&main_ts, &params(1), &input, 0.0).expect("clamp wrap renders");
    assert_ne!(
        repeat_out.data_as_u8_slice(),
        clamp_out.data_as_u8_slice(),
        "repeat and clamp wrap modes must produce distinct refracted outputs"
    );
}

#[test]
fn unsupported_shared_handle_helper_never_corrupts_and_js_try_catch_fallback_succeeds() {
    worker();
    let shared_wgsl = r"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var repeat_samp: sampler;
@group(0) @binding(2) var clamp_samp: sampler;
struct VsOut { @builtin(position) pos: vec4f, @location(0) uv: vec2f };
@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var o: VsOut; o.pos = vec4f(p[i], 0.0, 1.0); o.uv = p[i] * 0.5 + vec2f(0.5); return o;
}
fn sample_both(t: texture_2d<f32>, use_repeat: bool, uv: vec2f) -> vec4f {
  if (use_repeat) { return textureSampleLevel(t, repeat_samp, uv, 0.0); }
  return textureSampleLevel(t, clamp_samp, uv, 0.0);
}
@fragment fn fs(in: VsOut) -> @location(0) vec4f {
  return sample_both(tex, in.uv.x < 0.5, vec2f(1.25, 0.5));
}
";
    let input = split_input();
    match render_effect(&shader_js(shared_wgsl, 0), &json!({}), &input, 0.0) {
        Err(e) if e.skips_test() => return,
        // On GL, in-place lowering rejects shared-handle splits explicitly as a GPU error.
        Err(e) => assert!(!e.is_script(), "must be a GPU error, not script: {e}"),
        // On backends with native separate texture/sampler support (Vulkan/Metal/DX12), pixels
        // must be exact.
        Ok(out) => {
            let left = out.sample(0, 2);
            let right = out.sample(3, 2);
            assert_eq!([left.r, left.g, left.b, left.a], [255, 0, 0, 255]);
            assert_eq!([right.r, right.g, right.b, right.a], [0, 0, 255, 255]);
        }
    }

    // If JS catches a rejected pipeline creation and renders a fallback pipeline, valid pixels
    // must be returned (no sticky session.gpu_error).
    let fallback_js = format!(
        r#"
export function render(device: any, frame: any) {{
  const primary = device.createShaderModule({{ code: `{shared_wgsl}` }});
  try {{
    device.createRenderPipeline({{
      layout: 'auto',
      vertex: {{ module: primary, entryPoint: 'vs' }},
      fragment: {{ module: primary, entryPoint: 'fs', targets: [{{ format: frame.output.format }}] }},
      primitive: {{ topology: 'triangle-list' }},
    }});
  }} catch (_e) {{
  }}
  const fb = device.createShaderModule({{ code: `
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {{
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(p[i], 0.0, 1.0);
}}
@fragment fn fs() -> @location(0) vec4f {{ return vec4f(0.0, 1.0, 0.0, 1.0); }}
` }});
  const pipeline = device.createRenderPipeline({{
    layout: 'auto',
    vertex: {{ module: fb, entryPoint: 'vs' }},
    fragment: {{ module: fb, entryPoint: 'fs', targets: [{{ format: frame.output.format }}] }},
    primitive: {{ topology: 'triangle-list' }},
  }});
  const enc = device.createCommandEncoder();
  const pass = enc.beginRenderPass({{
    colorAttachments: [{{
      view: frame.output.createView(),
      loadOp: 'clear',
      clearValue: {{ r: 0, g: 0, b: 0, a: 0 }},
      storeOp: 'store',
    }}],
  }});
  pass.setPipeline(pipeline);
  pass.draw(3);
  pass.end();
  device.queue.submit([enc.finish()]);
}}
"#
    );
    let out = render_effect(&fallback_js, &json!({}), &input, 0.0)
        .expect("caught pipeline error must allow fallback pipeline to succeed");
    let px = out.sample(1, 1);
    assert_eq!([px.r, px.g, px.b, px.a], [0, 255, 0, 255]);
}

#[test]
fn internal_pipeline_error_and_wgsl_capability_diagnostics_are_preserved() {
    worker();
    // 1. Unenabled WGSL capability (`enable f16;`) must fail with stock WGSL source diagnostics
    //    instead of blank `:1:1` Naga IR errors.
    let f16_js = r#"
export function render(device: any, frame: any) {
  const module = device.createShaderModule({ code: `
enable f16;
@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
@fragment fn fs() -> @location(0) vec4f { let x: f16 = 1.0h; return vec4f(f32(x)); }
` });
  device.createRenderPipeline({
    layout: 'auto',
    vertex: { module, entryPoint: 'vs' },
    fragment: { module, entryPoint: 'fs', targets: [{ format: frame.output.format }] },
    primitive: { topology: 'triangle-list' },
  });
}
"#;
    let err = match render(f16_js, &json!({}), 4, 4, 0.0) {
        Err(e) if e.skips_test() => return,
        Err(e) => e,
        Ok(_) => panic!("enable f16 must fail on default device without SHADER_F16"),
    };
    assert!(!err.is_script(), "must report a GPU error: {err}");
    let msg = err.to_string();
    assert!(
        msg.contains("enable f16;"),
        "expected stock WGSL source snippet in capability diagnostic, got: {msg}"
    );

    // 2. `@interpolate(flat, first)` triggers `ErrorType::Internal` (`FirstSamplingNotSupported`)
    //    in `naga::back::glsl` on OpenGL; ErrorFilter::Internal must surface that root error
    //    instead of the cascading `RenderPipeline::get_bind_group_layout` Validation error.
    let first_sampling_js = r#"
export function render(device: any, frame: any) {
  const module = device.createShaderModule({ code: `
struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat, first) tag: u32,
};
@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return VsOut(vec4f(p[i], 0.0, 1.0), i);
}
@fragment fn fs(in: VsOut) -> @location(0) vec4f {
  return vec4f(f32(in.tag), 0.0, 0.0, 1.0);
}
` });
  const pipeline = device.createRenderPipeline({
    layout: 'auto',
    vertex: { module, entryPoint: 'vs' },
    fragment: { module, entryPoint: 'fs', targets: [{ format: frame.output.format }] },
    primitive: { topology: 'triangle-list' },
  });
  pipeline.getBindGroupLayout(0);
}
"#;
    if let Err(err) = render(first_sampling_js, &json!({}), 4, 4, 0.0) {
        assert!(!err.is_script(), "must report a GPU error: {err}");
        let msg = err.to_string();
        assert!(
            msg.contains("First") || msg.contains("create_render_pipeline"),
            "root Internal pipeline error must not be masked by get_bind_group_layout: {msg}"
        );
    }
}

/// The shader process for these tests: this crate's worker binary (a test binary cannot be one).
fn worker() {
    figstudio_shader::set_worker_program(env!("CARGO_BIN_EXE_figstudio-shader-worker").into());
}
