import { defineProperties } from "figma:shaders";
export default function Effect() { }
export function setup(device: any, frame: any) {
    var extractWGSL = `
diagnostic(off,derivative_uniformity);

struct ExtractUniforms {
  threshold: f32,
  knee: f32,
  originX: f32,
  originY: f32,
  originRadius: f32,
  aspectRatio: f32,
  pad0: f32,
  pad1: f32,
};

@group(0) @binding(0) var<uniform> u: ExtractUniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var inputTex: texture_2d<f32>;

struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@location(0) pos: vec2f, @location(1) uv: vec2f) -> VsOut {
  var out: VsOut;
  out.position = vec4f(pos, 0.0, 1.0);
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let color = textureSampleLevel(inputTex, samp, in.uv, 0.0);
  let lum = dot(color.rgb, vec3f(0.2126, 0.7152, 0.0722));
  let bright = smoothstep(u.threshold, u.threshold + u.knee, lum);
  let dx = (in.uv.x - u.originX) * u.aspectRatio;
  let dy = in.uv.y - u.originY;
  let dist = sqrt(dx * dx + dy * dy);
  let mask = 1.0 - smoothstep(u.originRadius * 0.9, u.originRadius, dist);
  return vec4f(color.rgb * bright * mask, color.a);
}
`;
    var blurWGSL = `
diagnostic(off,derivative_uniformity);

struct BlurUniforms {
  radius: f32,
  dirX: f32,
  dirY: f32,
  pad: f32,
};

@group(0) @binding(0) var<uniform> u: BlurUniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var inputTex: texture_2d<f32>;

struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@location(0) pos: vec2f, @location(1) uv: vec2f) -> VsOut {
  var out: VsOut;
  out.position = vec4f(pos, 0.0, 1.0);
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let dims = vec2f(textureDimensions(inputTex, 0));
  let sigma = max(u.radius * 0.5, 0.001);
  let step = vec2f(u.dirX, u.dirY) / dims;
  var colorSum = vec4f(0.0);
  var weightSum = 0.0;
  for (var i = -16; i <= 16; i++) {
    let fi = f32(i);
    let w = exp(-fi * fi / (2.0 * sigma * sigma));
    let sampleUv = clamp(in.uv + step * fi, vec2f(0.0), vec2f(1.0));
    colorSum += textureSampleLevel(inputTex, samp, sampleUv, 0.0) * w;
    weightSum += w;
  }
  return colorSum / weightSum;
}
`;
    var compositeWGSL = `
diagnostic(off,derivative_uniformity);

struct CompositeUniforms {
  intensity: f32,
  tintR: f32,
  tintG: f32,
  tintB: f32,
  vignetteStrength: f32,
  vignetteSoftness: f32,
  tintA: f32,
  pad1: f32,
};

@group(0) @binding(0) var<uniform> u: CompositeUniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var originalTex: texture_2d<f32>;
@group(0) @binding(3) var halfGlowTex: texture_2d<f32>;
@group(0) @binding(4) var quarterGlowTex: texture_2d<f32>;

struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@location(0) pos: vec2f, @location(1) uv: vec2f) -> VsOut {
  var out: VsOut;
  out.position = vec4f(pos, 0.0, 1.0);
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let original = textureSampleLevel(originalTex, samp, in.uv, 0.0);
  let halfGlow = textureSampleLevel(halfGlowTex, samp, in.uv, 0.0);
  let quarterGlow = textureSampleLevel(quarterGlowTex, samp, in.uv, 0.0);

  let glow = halfGlow * 0.6 + quarterGlow * 0.4;
  let tint = vec3f(u.tintR, u.tintG, u.tintB);
  let tintedGlow = mix(glow.rgb, glow.rgb * tint, u.tintA) * u.intensity;
  let composited = clamp(original.rgb + tintedGlow, vec3f(0.0), vec3f(1.0));

  let power = 1.0 + (1.0 - u.vignetteSoftness) * 4.0;
  let d = pow(length(in.uv - vec2f(0.5)) * 1.41421356, power);
  let vignette = 1.0 - clamp(d * u.vignetteStrength, 0.0, 1.0);

  return vec4f(composited * vignette, original.a);
}
`;
    frame.state.extractModule = device.createShaderModule({ code: extractWGSL });
    frame.state.blurModule = device.createShaderModule({ code: blurWGSL });
    frame.state.compositeModule = device.createShaderModule({ code: compositeWGSL });
    frame.state.sampler = device.createSampler({
        magFilter: 'linear',
        minFilter: 'linear',
        addressModeU: 'clamp-to-edge',
        addressModeV: 'clamp-to-edge',
    });
    // Extract uniform: 32 bytes
    frame.state.extractUniformBuf = device.createBuffer({
        size: 32,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    // Blur uniforms: 4 passes, each 16 bytes
    frame.state.blurUniformBufs = [];
    for (var i = 0; i < 6; i++) {
        frame.state.blurUniformBufs.push(device.createBuffer({
            size: 16,
            usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
        }));
    }
    // Composite uniform: 32 bytes
    frame.state.compositeUniformBuf = device.createBuffer({
        size: 32,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    // Fullscreen quad
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
    frame.state.lastWidth = 0;
    frame.state.lastHeight = 0;
    frame.state.pipelineFormat = null;
}
function makeTexture(device: any, width: any, height: any) {
    return device.createTexture({
        size: [width, height, 1],
        format: 'rgba8unorm',
        usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
}
export function render(device: any, frame: any) {
    if (frame.input == null)
        return;
    var p = frame.params;
    var threshold = 0.85 - (p.threshold / 100.0) * 0.75;
    var intensity = (p.intensity / 100.0) * 3.0;
    var radius = 5.0 + (p.softness / 100.0) * 55.0;
    var originX = p.origin.x / 100.0;
    var originY = p.origin.y / 100.0;
    var originRadius = p.origin.radius / 100.0;
    var inW = frame.input.width;
    var inH = frame.input.height;
    var halfW = Math.max(1, Math.floor(inW / 2));
    var halfH = Math.max(1, Math.floor(inH / 2));
    var quarterW = Math.max(1, Math.floor(inW / 4));
    var quarterH = Math.max(1, Math.floor(inH / 4));
    // Recreate intermediate textures if dimensions changed
    if (frame.state.lastWidth !== inW || frame.state.lastHeight !== inH) {
        if (frame.state.extractTex)
            frame.state.extractTex.destroy();
        if (frame.state.half0Tex)
            frame.state.half0Tex.destroy();
        if (frame.state.half1Tex)
            frame.state.half1Tex.destroy();
        if (frame.state.quarter0Tex)
            frame.state.quarter0Tex.destroy();
        if (frame.state.quarter1Tex)
            frame.state.quarter1Tex.destroy();
        frame.state.extractTex = makeTexture(device, inW, inH);
        frame.state.half0Tex = makeTexture(device, halfW, halfH);
        frame.state.half1Tex = makeTexture(device, halfW, halfH);
        frame.state.quarter0Tex = makeTexture(device, quarterW, quarterH);
        frame.state.quarter1Tex = makeTexture(device, quarterW, quarterH);
        frame.state.lastWidth = inW;
        frame.state.lastHeight = inH;
    }
    // Rebuild pipelines if output format changed
    if (frame.state.pipelineFormat !== frame.output.format) {
        var vertexBuffers = [{
                arrayStride: 16,
                attributes: [
                    { shaderLocation: 0, format: 'float32x2', offset: 0 },
                    { shaderLocation: 1, format: 'float32x2', offset: 8 },
                ],
            }];
        frame.state.extractPipeline = device.createRenderPipeline({
            layout: 'auto',
            vertex: { module: frame.state.extractModule, entryPoint: 'vs_main', buffers: vertexBuffers },
            fragment: { module: frame.state.extractModule, entryPoint: 'fs_main', targets: [{ format: 'rgba8unorm' }] },
            primitive: { topology: 'triangle-list' },
        });
        frame.state.blurPipeline = device.createRenderPipeline({
            layout: 'auto',
            vertex: { module: frame.state.blurModule, entryPoint: 'vs_main', buffers: vertexBuffers },
            fragment: { module: frame.state.blurModule, entryPoint: 'fs_main', targets: [{ format: 'rgba8unorm' }] },
            primitive: { topology: 'triangle-list' },
        });
        frame.state.compositePipeline = device.createRenderPipeline({
            layout: 'auto',
            vertex: { module: frame.state.compositeModule, entryPoint: 'vs_main', buffers: vertexBuffers },
            fragment: { module: frame.state.compositeModule, entryPoint: 'fs_main', targets: [{ format: frame.output.format }] },
            primitive: { topology: 'triangle-list' },
        });
        frame.state.pipelineFormat = frame.output.format;
    }
    var aspectRatio = inW / inH;
    // Write extract uniforms
    device.queue.writeBuffer(frame.state.extractUniformBuf, 0, new Float32Array([threshold, 0.05, originX, originY, originRadius, aspectRatio, 0, 0]));
    // Blur uniform data: [radius, dirX, dirY, pad]
    // Pass 2: downsample extract→half0 (blur pass with small radius, H direction)
    // Pass 3: downsample half0→quarter0 (blur pass with small radius, H direction)
    // Pass 4: blur H quarter0→quarter1
    // Pass 5: blur V quarter1→quarter0
    // Pass 6: blur H half0→half1
    // Pass 7: blur V half1→half0
    var blurData = [
        [radius / 2.0, 1.0, 0.0, 0.0], // buf0: downsample H (extract→half)
        [radius / 4.0, 1.0, 0.0, 0.0], // buf1: downsample H (half→quarter)
        [radius, 1.0, 0.0, 0.0], // buf2: blur H quarter
        [radius, 0.0, 1.0, 0.0], // buf3: blur V quarter
        [radius, 1.0, 0.0, 0.0], // buf4: blur H half
        [radius, 0.0, 1.0, 0.0], // buf5: blur V half
    ];
    for (var i = 0; i < 6; i++) {
        device.queue.writeBuffer(frame.state.blurUniformBufs[i], 0, new Float32Array(blurData[i]));
    }
    // Write composite uniforms
    var vigStr = p.vignetteStrength / 100.0;
    var vigSoft = p.vignetteSoftness / 100.0;
    device.queue.writeBuffer(frame.state.compositeUniformBuf, 0, new Float32Array([intensity, p.tint.r, p.tint.g, p.tint.b, vigStr, vigSoft, p.tint.a, 0]));
    var encoder = device.createCommandEncoder();
    // Helper: run a single-texture render pass
    function runPass(pipeline: any, uniformBuf: any, sourceTex: any, targetTex: any) {
        var bindGroup = device.createBindGroup({
            layout: pipeline.getBindGroupLayout(0),
            entries: [
                { binding: 0, resource: { buffer: uniformBuf } },
                { binding: 1, resource: frame.state.sampler },
                { binding: 2, resource: sourceTex.createView() },
            ],
        });
        var pass = encoder.beginRenderPass({
            colorAttachments: [{
                    view: targetTex.createView(),
                    loadOp: 'clear',
                    clearValue: { r: 0, g: 0, b: 0, a: 0 },
                    storeOp: 'store',
                }],
        });
        pass.setPipeline(pipeline);
        pass.setBindGroup(0, bindGroup);
        pass.setVertexBuffer(0, frame.state.quad);
        pass.draw(6);
        pass.end();
    }
    // Pass 1: extract → extractTex
    runPass(frame.state.extractPipeline, frame.state.extractUniformBuf, frame.input, frame.state.extractTex);
    // Pass 2: downsample extract → half0Tex
    runPass(frame.state.blurPipeline, frame.state.blurUniformBufs[0], frame.state.extractTex, frame.state.half0Tex);
    // Pass 3: downsample half0 → quarter0Tex
    runPass(frame.state.blurPipeline, frame.state.blurUniformBufs[1], frame.state.half0Tex, frame.state.quarter0Tex);
    // Pass 4: blur H quarter0 → quarter1Tex
    runPass(frame.state.blurPipeline, frame.state.blurUniformBufs[2], frame.state.quarter0Tex, frame.state.quarter1Tex);
    // Pass 5: blur V quarter1 → quarter0Tex
    runPass(frame.state.blurPipeline, frame.state.blurUniformBufs[3], frame.state.quarter1Tex, frame.state.quarter0Tex);
    // Pass 6: blur H half0 → half1Tex
    runPass(frame.state.blurPipeline, frame.state.blurUniformBufs[4], frame.state.half0Tex, frame.state.half1Tex);
    // Pass 7: blur V half1 → half0Tex
    runPass(frame.state.blurPipeline, frame.state.blurUniformBufs[5], frame.state.half1Tex, frame.state.half0Tex);
    // Pass 8: composite → frame.output
    var compositeBindGroup = device.createBindGroup({
        layout: frame.state.compositePipeline.getBindGroupLayout(0),
        entries: [
            { binding: 0, resource: { buffer: frame.state.compositeUniformBuf } },
            { binding: 1, resource: frame.state.sampler },
            { binding: 2, resource: frame.input.createView() },
            { binding: 3, resource: frame.state.half0Tex.createView() },
            { binding: 4, resource: frame.state.quarter0Tex.createView() },
        ],
    });
    var compositePass = encoder.beginRenderPass({
        colorAttachments: [{
                view: frame.output.createView(),
                loadOp: 'clear',
                clearValue: { r: 0, g: 0, b: 0, a: 0 },
                storeOp: 'store',
            }],
    });
    compositePass.setPipeline(frame.state.compositePipeline);
    compositePass.setBindGroup(0, compositeBindGroup);
    compositePass.setVertexBuffer(0, frame.state.quad);
    compositePass.draw(6);
    compositePass.end();
    device.queue.submit([encoder.finish()]);
}
defineProperties(Effect, {
    threshold: {
        type: "number",
        label: "Threshold",
        defaultValue: 50,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
    },
    intensity: {
        type: "number",
        label: "Intensity",
        defaultValue: 10,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
    },
    softness: {
        type: "number",
        label: "Softness",
        defaultValue: 50,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
    },
    tint: {
        type: "color",
        label: "Tint",
        defaultValue: { r: 1, g: 1, b: 1, a: 1 },
    },
    vignetteStrength: {
        type: "number",
        label: "Vignette",
        defaultValue: 50,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
    },
    vignetteSoftness: {
        type: "number",
        label: "Vignette Softness",
        defaultValue: 50,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
    },
    origin: {
        type: "point-radius",
        label: "Spotlight",
        defaultValue: { x: 50, y: 50, radius: 70 },
        control: "point-radius",
        mode: "canvas_and_ui",
        positionUnit: "%",
        radiusUnit: "%",
    },
});
