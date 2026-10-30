import { defineProperties } from "figma:shaders";
export default function Effect() { }
export function setup(device: any, frame: any) {
    var shader = /* wgsl */ `
diagnostic(off,derivative_uniformity);

const PI: f32 = 3.14159265358979323846;

struct Uniforms {
  centerPos: vec2f,
  angle: f32,
  size: f32,
  amount: f32,
  seamlessness: f32,
  frost: f32,
  iorDispersion: f32,
  patternType: u32,
  pixelWrapMode: u32,
  _pad0: u32,
  _pad1: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var inputSampler: sampler;
@group(0) @binding(2) var inputTexture: texture_2d<f32>;
@group(0) @binding(3) var inputSamplerClamp: sampler;

struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@location(0) pos: vec2f, @location(1) uv: vec2f) -> VsOut {
  var o: VsOut;
  o.position = vec4f(pos, 0.0, 1.0);
  o.uv = uv;
  return o;
}

fn rot(a: f32) -> mat2x2f {
  let c = cos(a);
  let s = sin(a);
  return mat2x2f(c, s, -s, c);
}

fn wave(t: f32, freq: f32, amp: f32) -> f32 {
  return sin(t * freq * 2.0 * PI) * amp;
}

fn zigzag(t: f32, freq: f32, amp: f32) -> f32 {
  let p = t * freq;
  return (abs(fract(p) * 2.0 - 1.0) * 2.0 - 1.0) * amp;
}

// Value noise helper
fn hash3(p: vec3f) -> f32 {
  var pp = p;
  pp = fract(pp * 0.3183099 + vec3f(0.1, 0.1, 0.1));
  pp *= 17.0;
  return fract(pp.x * pp.y * pp.z * (pp.x + pp.y + pp.z));
}

fn vnoise(p: vec3f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  return mix(
    mix(
      mix(hash3(i + vec3f(0.0, 0.0, 0.0)), hash3(i + vec3f(1.0, 0.0, 0.0)), u.x),
      mix(hash3(i + vec3f(0.0, 1.0, 0.0)), hash3(i + vec3f(1.0, 1.0, 0.0)), u.x),
      u.y
    ),
    mix(
      mix(hash3(i + vec3f(0.0, 0.0, 1.0)), hash3(i + vec3f(1.0, 0.0, 1.0)), u.x),
      mix(hash3(i + vec3f(0.0, 1.0, 1.0)), hash3(i + vec3f(1.0, 1.0, 1.0)), u.x),
      u.y
    ),
    u.z
  );
}

fn patternHeight(p: vec2f) -> f32 {
  var pos = p - u.centerPos;
  pos = rot(u.angle) * pos;
  pos /= u.size;

  let waveFreq_scaled = 0.15;
  let waveP = pos.y;

  if (u.patternType == 2u) {
    pos.x += wave(waveP, waveFreq_scaled, 0.6);
  } else if (u.patternType == 1u) {
    pos.x += zigzag(waveP, waveFreq_scaled, 0.6);
  }

  let gridPos = fract(pos) * 2.0 - 1.0;

  var height = 1.0;

  if (u.patternType == 0u || u.patternType == 1u || u.patternType == 2u) {
    height = pow(sin((gridPos.x * 0.5 + 0.5) * PI), 0.7);
  } else if (u.patternType == 3u) {
    height = 1.0 - dot(gridPos, gridPos);
  } else if (u.patternType == 4u) {
    let absUV = abs(gridPos);
    height = 1.0 - max(absUV.x, absUV.y) * dot(gridPos, gridPos) * 0.8;
  } else if (u.patternType == 5u) {
    let d = abs(gridPos * 1.125) - vec2f(0.5);
    height = 1.0 - saturate(length(max(d, vec2f(0.0))) + max(d.x, d.y));
  }

  height = saturate(height);
  height *= pow(height, u.seamlessness);

  if (u.frost > 0.0001) {
    height += (vnoise(vec3f(p * 0.5, 1.0)) - 0.5) * u.frost;
  }

  return height;
}

fn patternNormals(p: vec2f) -> vec3f {
  let height = patternHeight(p);
  let offset = vec2f(0.125, 0.0);
  let h1 = patternHeight(p + offset.xy);
  let v1 = patternHeight(p + offset.yx);
  return normalize(vec3f(height - vec2f(h1, v1), 0.0125));
}

fn sampleWithWrap(pixelPos: vec2f, dims: vec2f) -> vec4f {
  // wrapMode: 0=zero, 1=clamp, 2=repeat, 3=mirror
  var sampleUv = pixelPos / dims;
  if (u.pixelWrapMode == 0u) {
    if (sampleUv.x < 0.0 || sampleUv.x > 1.0 || sampleUv.y < 0.0 || sampleUv.y > 1.0) {
      return vec4f(0.0);
    }
    return textureSampleLevel(inputTexture, inputSamplerClamp, clamp(sampleUv, vec2f(0.0), vec2f(1.0)), 0.0);
  } else if (u.pixelWrapMode == 1u) {
    return textureSampleLevel(inputTexture, inputSamplerClamp, clamp(sampleUv, vec2f(0.0), vec2f(1.0)), 0.0);
  } else if (u.pixelWrapMode == 2u) {
    return textureSampleLevel(inputTexture, inputSampler, fract(sampleUv), 0.0);
  } else {
    // mirror
    let m = fract(sampleUv * 0.5) * 2.0;
    let mirrored = vec2f(
      select(m.x, 2.0 - m.x, m.x > 1.0),
      select(m.y, 2.0 - m.y, m.y > 1.0)
    );
    return textureSampleLevel(inputTexture, inputSamplerClamp, clamp(mirrored, vec2f(0.0), vec2f(1.0)), 0.0);
  }
}

@fragment
fn fs_main(@builtin(position) fragPos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
  let dims = vec2f(textureDimensions(inputTexture, 0));
  let localPos = uv * dims;

  let msaaSamples = 6;
  let msaaScale = 1.0;
  let ray = vec3f(0.0, 0.0, -1.0);

  var iorR = 1.333 + u.iorDispersion;
  var iorG = 1.333;
  var iorB = 1.333 - u.iorDispersion;

  var accum = vec4f(0.0);

  for (var i = 0; i < msaaSamples; i++) {
    for (var j = 0; j < msaaSamples; j++) {
      let offset = vec2f(f32(i), f32(j)) / f32(msaaSamples) - (f32(msaaSamples) - 1.0) / f32(msaaSamples) * 0.5;
      let samplePos = localPos + offset * msaaScale;

      let nor = patternNormals(samplePos);

      let rDirR = refract(ray, nor, iorR);
      let rDirG = refract(ray, nor, iorG);
      let rDirB = refract(ray, nor, iorB);

      let amountScaled = u.amount;

      let colR = sampleWithWrap(localPos + rDirR.xy * amountScaled, dims);
      let colG = sampleWithWrap(localPos + rDirG.xy * amountScaled, dims);
      let colB = sampleWithWrap(localPos + rDirB.xy * amountScaled, dims);

      accum.r += colR.r;
      accum.g += colG.g;
      accum.b += colB.b;
      accum.a += colG.a;
    }
  }

  let total = f32(msaaSamples * msaaSamples);
  return accum / total;
}
`;
    frame.state.shaderModule = device.createShaderModule({ code: shader });
    frame.state.uniformBuf = device.createBuffer({
        size: 48,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    frame.state.sampler = device.createSampler({
        addressModeU: 'repeat',
        addressModeV: 'repeat',
        magFilter: 'linear',
        minFilter: 'linear',
    });
    frame.state.samplerClamp = device.createSampler({
        addressModeU: 'clamp-to-edge',
        addressModeV: 'clamp-to-edge',
        magFilter: 'linear',
        minFilter: 'linear',
    });
    frame.state.quad = device.createBuffer({
        size: 6 * 4 * 4,
        usage: GPUBufferUsage.VERTEX,
        mappedAtCreation: true,
    });
    new Float32Array(frame.state.quad.getMappedRange()).set([
        -1, -1, 0, 1,
        1, -1, 1, 1,
        -1, 1, 0, 0,
        -1, 1, 0, 0,
        1, -1, 1, 1,
        1, 1, 1, 0,
    ]);
    frame.state.quad.unmap();
}
export function render(device: any, frame: any) {
    if (frame.input == null)
        return;
    var p = frame.params;
    // Single point-angle-radius param as source of truth for center, angle, size
    var w = frame.input.width;
    var h = frame.input.height;
    var centerX = p.centerHandle.x / 100.0 * w;
    var centerY = p.centerHandle.y / 100.0 * h;
    var angle = -p.centerHandle.angle * (Math.PI / 180.0);
    // radius 0–100% maps to size 20–1000px
    var size = 20.0 + (p.centerHandle.radius / 100.0) * 980.0;
    // amount slider is [-100,100] mapped to [-1000,1000] px
    var amount = p.amount * 10.0;
    var seamlessness = p.seamlessness / 100.0;
    // clamp to avoid pow(0)
    if (seamlessness < 0.001)
        seamlessness = 0.001;
    var frost = (p.frost / 100.0) * 0.1;
    var iorDispersion = (p.iorDispersion / 100.0) * 0.25;
    var patternType = p.patternType;
    var pixelWrapMode = p.pixelWrapMode;
    // Uniforms layout (48 bytes = 3 vec4f):
    // [0] centerPos.x, centerPos.y, angle, size
    // [1] amount, seamlessness, frost, iorDispersion
    // [2] patternType(u32), pixelWrapMode(u32), pad, pad
    var udata = new Float32Array(12);
    udata[0] = centerX;
    udata[1] = centerY;
    udata[2] = angle;
    udata[3] = size;
    udata[4] = amount;
    udata[5] = seamlessness;
    udata[6] = frost;
    udata[7] = iorDispersion;
    // pack u32 values using a DataView
    var buf = udata.buffer;
    var dv = new DataView(buf);
    dv.setUint32(8 * 4, patternType, true);
    dv.setUint32(9 * 4, pixelWrapMode, true);
    // udata[10..11] stay 0 (padding)
    // Note: total 12 floats = 48 bytes
    device.queue.writeBuffer(frame.state.uniformBuf, 0, udata);
    if (frame.state.pipelineFormat !== frame.output.format) {
        frame.state.pipeline = device.createRenderPipeline({
            layout: 'auto',
            vertex: {
                module: frame.state.shaderModule,
                entryPoint: 'vs_main',
                buffers: [{
                        arrayStride: 16,
                        attributes: [
                            { shaderLocation: 0, format: 'float32x2', offset: 0 },
                            { shaderLocation: 1, format: 'float32x2', offset: 8 },
                        ],
                    }],
            },
            fragment: {
                module: frame.state.shaderModule,
                entryPoint: 'fs_main',
                targets: [{ format: frame.output.format }],
            },
            primitive: { topology: 'triangle-list' },
        });
        frame.state.pipelineFormat = frame.output.format;
    }
    var bindGroup = device.createBindGroup({
        layout: frame.state.pipeline.getBindGroupLayout(0),
        entries: [
            { binding: 0, resource: { buffer: frame.state.uniformBuf } },
            { binding: 1, resource: frame.state.sampler },
            { binding: 2, resource: frame.input.createView() },
            { binding: 3, resource: frame.state.samplerClamp },
        ],
    });
    var encoder = device.createCommandEncoder();
    var pass = encoder.beginRenderPass({
        colorAttachments: [{
                view: frame.output.createView(),
                loadOp: 'clear',
                clearValue: { r: 0, g: 0, b: 0, a: 0 },
                storeOp: 'store',
            }],
    });
    pass.setPipeline(frame.state.pipeline);
    pass.setBindGroup(0, bindGroup);
    pass.setVertexBuffer(0, frame.state.quad);
    pass.draw(6);
    pass.end();
    device.queue.submit([encoder.finish()]);
}
defineProperties(Effect, {
    patternType: {
        type: "number",
        label: "Pattern",
        defaultValue: 0,
        control: "select",
        options: [
            { value: 0, label: "Lenticular" },
            { value: 1, label: "Zigzag" },
            { value: 2, label: "Waves" },
            { value: 3, label: "Circular" },
            { value: 4, label: "Curved square" },
            { value: 5, label: "Flat square" },
        ],
    },
    amount: {
        type: "number",
        label: "Strength",
        defaultValue: 50,
        control: "slider",
        min: -100,
        max: 100,
        step: 1,
    },
    seamlessness: {
        type: "number",
        label: "Smoothness",
        defaultValue: 0,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
        unit: "%",
    },
    frost: {
        type: "number",
        label: "Frost",
        defaultValue: 0,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
        unit: "%",
    },
    iorDispersion: {
        type: "number",
        label: "Dispersion",
        defaultValue: 4,
        control: "slider",
        min: -100,
        max: 100,
        step: 1,
        unit: "%",
    },
    pixelWrapMode: {
        type: "number",
        label: "Edge wrap",
        defaultValue: 0,
        control: "select",
        options: [
            { value: 0, label: "Zero" },
            { value: 1, label: "Clamp" },
            { value: 2, label: "Repeat" },
            { value: 3, label: "Mirror" },
        ],
    },
    centerHandle: {
        type: "point-angle-radius",
        label: "Transform",
        defaultValue: { x: 50, y: 50, radius: 8.16, angle: 45 },
        control: "point-angle-radius",
        mode: "canvas_and_ui",
        positionUnit: "%",
        radiusUnit: "%",
    },
});
