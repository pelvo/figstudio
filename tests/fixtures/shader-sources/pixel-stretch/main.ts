import { defineProperties } from "figma:shaders";
export default function Effect() { }
export function setup(device: any, frame: any) {
    var wgsl = `
diagnostic(off,derivative_uniformity);
struct Uniforms {
  frameData: vec4f,
  inputDimsData: vec4f,
  transform: vec4f,
  pars: vec4f,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var inputTex: texture_2d<f32>;

struct VsIn {
  @location(0) pos: vec2f,
  @location(1) uv: vec2f,
};
struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

fn sampleInput(p: vec2f) -> vec4f {
  return textureSampleLevel(inputTex, samp, clamp(p, vec2f(0.0), vec2f(1.0)), 0.0);
}

// Like sampleInput but returns transparent (premultiplied zero) outside the
// input rect instead of repeating the edge texel.
fn sampleInputOrZero(p: vec2f) -> vec4f {
  let inside = step(vec2f(0.0), p) * step(p, vec2f(1.0));
  return sampleInput(p) * (inside.x * inside.y);
}

fn rot(angle: f32) -> mat2x2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return mat2x2<f32>(c, s, -s, c);
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

/// Linear version of smoothstep
fn linearstep(a: f32, b: f32, t: f32) -> f32 {
    return clamp((t - a) / (b - a), 0.0, 1.0);
}

fn stretch(localPos: vec2f, centerUv: vec2f, dist: f32, rot_rad: f32, offset: f32, smoothK: f32) -> vec3f {

    let halfDist = dist * 0.5;
    var p = localPos - centerUv;
    p = rot(rot_rad) * p;

    p.x += offset * halfDist;

    var v = linearstep(-halfDist, halfDist, p.x);
    p.x = smin(halfDist - abs(p.x), 0.0, halfDist * smoothK + 0.001) * -sign(p.x);

    p = rot(-rot_rad) * p;
    p += centerUv;
    return vec3f(p.x, p.y, v);
}

@vertex fn vs_main(in: VsIn) -> VsOut {
  var out: VsOut;
  out.position = vec4f(in.pos, 0.0, 1.0);
  out.uv = in.uv;
  return out;
}

@fragment fn fs_main(@location(0) outputUv_in: vec2f) -> @location(0) vec4f {
  let time = u.frameData.x;
  let outputUv = outputUv_in;
  let dims = max(u.frameData.yz, vec2f(1.0));
  let inputDims = max(u.inputDimsData.xy, vec2f(1.0));
  let inputTexel = vec2f(1.0) / inputDims;
  let centerPos = u.transform.xy;
  let distance = u.transform.z;
  let rotation = -u.transform.w;
  let offset = u.pars.x;
  let smoothness = u.pars.y;
  let falloff = u.pars.z;
  let centerUv = u.transform.xy * 0.01;

  let localP = outputUv * inputDims;
  let localCenter = centerPos * 0.01 * inputDims;
  let s = stretch(localP, localCenter, distance, rotation, offset, smoothness);
  let p = s.xy;
  var color = sampleInputOrZero(p / inputDims);

  if (falloff != 0.0) {
    let start = 1.0 - abs(falloff);
    let end = 1.0;
    let t = select(1.0-s.z, s.z, falloff > 0.0);
    let alpha = smoothstep(end, start, t);
    color *= alpha;
  }
  return color;
}
`;
    frame.state.module = device.createShaderModule({
        code: wgsl,
    });
    frame.state.pipeline = null;
    frame.state.pipelineFormat = null;
    frame.state.quad = device.createBuffer({
        size: 6 * 4 * 4,
        usage: GPUBufferUsage.VERTEX,
        mappedAtCreation: true,
    });
    new Float32Array(frame.state.quad.getMappedRange()).set([
        -1, -1, 0, 1, 1, -1, 1, 1, -1, 1, 0, 0, -1, 1, 0, 0, 1, -1,
        1, 1, 1, 1, 1, 0,
    ]);
    frame.state.quad.unmap();
    frame.state.uniformBuf = device.createBuffer({
        size: 64,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    frame.state.sampler = device.createSampler({
        magFilter: "linear",
        minFilter: "linear",
        addressModeU: "clamp-to-edge",
        addressModeV: "clamp-to-edge",
    });
    frame.state.placeholder = device.createTexture({
        size: [1, 1, 1],
        format: "rgba8unorm",
        usage: GPUTextureUsage.TEXTURE_BINDING |
            GPUTextureUsage.COPY_DST,
    });
    device.queue.writeTexture({ texture: frame.state.placeholder }, new Uint8Array([0, 0, 0, 0]), { bytesPerRow: 4 }, { width: 1, height: 1, depthOrArrayLayers: 1 });
}
export function render(device: any, frame: any) {
    var params = frame.params || {};
    function finiteNumber(value: any, fallback: any) {
        var num = Number(value);
        return Number.isFinite(num) ? num : fallback;
    }
    function numberParam(name: any, fallback: any) {
        return finiteNumber(params[name], fallback);
    }
    function boolParam(name: any, fallback: any) {
        if (typeof params[name] === "boolean")
            return params[name] ? 1 : 0;
        return fallback ? 1 : 0;
    }
    function pointParam(name: any, fallback: any) {
        var value = params[name] || {};
        return [
            finiteNumber(value.x, fallback[0]),
            finiteNumber(value.y, fallback[1]),
        ];
    }
    function pointRadiusParam(name: any, fallback: any) {
        var value = params[name] || {};
        return [
            finiteNumber(value.x, fallback[0]),
            finiteNumber(value.y, fallback[1]),
            finiteNumber(value.radius, fallback[2]),
        ];
    }
    function pointAngleRadiusParam(name: any, fallback: any) {
        var p = params[name];
        if (p && typeof p === "object") {
            return [
                finiteNumber(p.x, fallback[0]),
                finiteNumber(p.y, fallback[1]),
                finiteNumber(p.radius, fallback[2]),
                finiteNumber(p.angle, fallback[3]),
            ];
        }
        return fallback;
    }
    var output = frame.output || {};
    var width = Math.max(1, finiteNumber(output.width, 1));
    var height = Math.max(1, finiteNumber(output.height, 1));
    var time = 0;
    var inputWidth = width;
    var inputHeight = height;
    var t = pointAngleRadiusParam("transform", [50, 50, 8, 0]);
    device.queue.writeBuffer(frame.state.uniformBuf, 0, new Float32Array([
        time,
        width,
        height,
        0,
        inputWidth,
        inputHeight,
        0,
        0,
        t[0],
        t[1],
        t[2],
        (t[3] * 3.14159265358979) / 180.0,
        numberParam("offset", 0) * 0.01,
        numberParam("smoothness", 0.5) * 0.01,
        numberParam("falloff", 0.0) * 0.01,
        0,
    ]));
    var inputView = frame.input != null
        ? frame.input.createView()
        : frame.state.placeholder.createView();
    var outputFormat = frame.output.format;
    if (frame.state.pipeline == null ||
        frame.state.pipelineFormat !== outputFormat) {
        frame.state.pipeline = device.createRenderPipeline({
            layout: "auto",
            vertex: {
                module: frame.state.module,
                entryPoint: "vs_main",
                buffers: [
                    {
                        arrayStride: 16,
                        attributes: [
                            {
                                shaderLocation: 0,
                                format: "float32x2",
                                offset: 0,
                            },
                            {
                                shaderLocation: 1,
                                format: "float32x2",
                                offset: 8,
                            },
                        ],
                    },
                ],
            },
            fragment: {
                module: frame.state.module,
                entryPoint: "fs_main",
                targets: [{ format: outputFormat }],
            },
            primitive: { topology: "triangle-list" },
        });
        frame.state.pipelineFormat = outputFormat;
    }
    var bindGroup = device.createBindGroup({
        layout: frame.state.pipeline.getBindGroupLayout(0),
        entries: [
            {
                binding: 0,
                resource: { buffer: frame.state.uniformBuf },
            },
            { binding: 1, resource: frame.state.sampler },
            { binding: 2, resource: inputView },
        ],
    });
    var encoder = device.createCommandEncoder();
    var pass = encoder.beginRenderPass({
        colorAttachments: [
            {
                view: frame.output.createView(),
                loadOp: "clear",
                clearValue: { r: 0, g: 0, b: 0, a: 0 },
                storeOp: "store",
            },
        ],
    });
    pass.setPipeline(frame.state.pipeline);
    pass.setBindGroup(0, bindGroup);
    pass.setVertexBuffer(0, frame.state.quad);
    pass.draw(6);
    pass.end();
    device.queue.submit([encoder.finish()]);
}
defineProperties(Effect, {
    offset: {
        type: "number",
        label: "Offset",
        defaultValue: -100,
        control: "slider",
        min: -100,
        max: 100,
        step: 1,
        unit: "%",
    },
    smoothness: {
        type: "number",
        label: "Smoothness",
        defaultValue: 0,
        control: "slider",
        min: 0,
        max: 100,
        step: 1,
        unit: "%",
    },
    falloff: {
        type: "number",
        label: "Falloff",
        defaultValue: 0,
        control: "slider",
        min: -100,
        max: 100,
        step: 1,
        unit: "%",
    },
    transform: {
        type: "point-angle-radius",
        label: "Transform",
        defaultValue: { x: 50, y: 50, radius: 100, angle: 0 },
        positionUnit: "%",
    },
});
