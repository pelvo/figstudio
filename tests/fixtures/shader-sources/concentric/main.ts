import { defineProperties } from "figma:shaders";
export default function Effect() { }
export function setup(device: any, frame: any) {
    var wgsl = `
diagnostic(off,derivative_uniformity);
struct Uniforms {
  frameData: vec4f,
  inputDimsData: vec4f,
  shape: vec4f,
  ringCount: vec4f,
  falloff: vec4f,
  inverseFalloff: vec4f,
  color: vec4f,
  zoomLevel: vec4f,
  rotation: vec4f,
  offset: vec4f,
  center: vec4f,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
  @location(0) pos: vec2f,
  @location(1) uv: vec2f,
};
struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

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
  let uv = outputUv;
  // texel and aspect are input-relative to match uv. Zoom fills rescale texel
  // alongside their uv redefinition below (see zoomLocal).
  let texel = inputTexel;
  let aspect = inputDims.x / max(inputDims.y, 1.0);
  // shape: 0=Triangle, 1=Circle, 2=Square
  let shape = i32(u.shape.x + 0.5);
  let ringCount = u.ringCount.x;
  let falloff = u.falloff.x;
  let inverseFalloff = u.inverseFalloff.x >= 0.5;
  let color = u.color;
  let zoomLevel = u.zoomLevel.x;
  let rotation = u.rotation.x;
  let offset = u.offset.x;
  let center = u.center.xy;
  let centerUv = (vec2f(center.x, center.y) / 100.0 - 0.5) / zoomLevel + 0.5;
  let uvZoomed = (uv - 0.5) / zoomLevel + 0.5;
  var p = uvZoomed - centerUv;
  p = p * vec2f(aspect, 1.0);
  let angle = rotation;
  let cosA = cos(angle);
  let sinA = sin(angle);
  let px = vec2f(cosA * p.x - sinA * p.y, sinA * p.x + cosA * p.y);
  var dist: f32;
  if (shape == 0) {
    dist = max(px.y, (sqrt(3.0) * abs(px.x) - px.y) * 0.5);
  } else if (shape == 1) {
    dist = length(px);
  } else {
    dist = max(abs(px.x), abs(px.y));
  }
  let scaledDist = dist * ringCount;
  let fw = fwidth(scaledDist);
  let blendWidth = fw * 1.5;
  let ring = fract(scaledDist + offset);
  let bandOn = smoothstep(0.0, blendWidth, ring);
  let bandOff = smoothstep(0.5 - blendWidth, 0.5, ring);
  let band = bandOn * (1.0 - bandOff);
  let fadeRaw = exp(-abs(dist) * falloff);
  var fade: f32;
  if (inverseFalloff) {
    fade = 1.0 - fadeRaw;
  } else {
    fade = fadeRaw;
  }
  let mask = band * fade;
  let alpha = mask * color.a;
  return vec4f(color.rgb * alpha, alpha);
}
`;
    frame.state.module = device.createShaderModule({ code: wgsl });
    frame.state.pipeline = null;
    frame.state.pipelineFormat = null;
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
    frame.state.uniformBuf = device.createBuffer({
        size: 176,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
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
        if (typeof params[name] === 'boolean')
            return params[name] ? 1 : 0;
        return fallback ? 1 : 0;
    }
    function selectParam(name: any, count: any, fallbackIndex: any) {
        // A select is a numeric dropdown: params[name] is the chosen option's index.
        var index = Math.round(Number(params[name]));
        return Number.isFinite(index) && index >= 0 && index < count ? index : fallbackIndex;
    }
    function colorParam(name: any, fallback: any) {
        var value = params[name] || {};
        return [
            finiteNumber(value.r, fallback[0]),
            finiteNumber(value.g, fallback[1]),
            finiteNumber(value.b, fallback[2]),
            finiteNumber(value.a, fallback[3]),
        ];
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
    function pointPointLineParam(name: any, fallback: any) {
        var value = params[name] || {};
        return [
            finiteNumber(value.x, fallback[0]),
            finiteNumber(value.y, fallback[1]),
            finiteNumber(value.x2, fallback[2]),
            finiteNumber(value.y2, fallback[3]),
        ];
    }
    function pointAngleRadiusParam(name: any, fallback: any) {
        var value = params[name] || {};
        return [
            finiteNumber(value.x, fallback[0]),
            finiteNumber(value.y, fallback[1]),
            finiteNumber(value.radius, fallback[2]),
            finiteNumber(value.angle, fallback[3]),
        ];
    }
    function colorPointParam(name: any, fallback: any) {
        var value = params[name] || {};
        var color = value.color || {};
        return [
            finiteNumber(value.x, fallback[0]),
            finiteNumber(value.y, fallback[1]),
            0,
            0,
            finiteNumber(color.r, fallback[2]),
            finiteNumber(color.g, fallback[3]),
            finiteNumber(color.b, fallback[4]),
            finiteNumber(color.a, fallback[5]),
        ];
    }
    function gradientParam(name: any, fallback: any) {
        var value = params[name] || {};
        var stops = Array.isArray(value.stops) && value.stops.length > 0 ? value.stops : fallback;
        // Layout: 8 stop colors (rgba), then 8 positions packed 4-per-vec4, then the live count.
        var out = new Array(44).fill(0);
        var count = Math.min(stops.length, 8);
        for (var i = 0; i < count; i++) {
            var stop = stops[i] || {};
            var color = stop.color || {};
            out[i * 4] = finiteNumber(color.r, 0);
            out[i * 4 + 1] = finiteNumber(color.g, 0);
            out[i * 4 + 2] = finiteNumber(color.b, 0);
            out[i * 4 + 3] = finiteNumber(color.a, 1);
            out[32 + i] = finiteNumber(stop.position, 0);
        }
        out[40] = count;
        return out;
    }
    var output = frame.output || {};
    var width = Math.max(1, finiteNumber(output.width, 1));
    var height = Math.max(1, finiteNumber(output.height, 1));
    var time = 0;
    var inputWidth = width;
    var inputHeight = height;
    var transform = pointAngleRadiusParam("transform", [50, 50, 50, 0]);
    var zoom = transform[2] / 50;
    device.queue.writeBuffer(frame.state.uniformBuf, 0, new Float32Array([
        time, width, height, 0,
        inputWidth, inputHeight, 0, 0,
        selectParam("shape", 3, 0), 0, 0, 0,
        numberParam("ringCount", 20), 0, 0, 0,
        numberParam("falloff", 1), 0, 0, 0,
        boolParam("inverseFalloff", false), 0, 0, 0,
        ...colorParam("color", [1, 1, 1, 1]),
        zoom, 0, 0, 0,
        -transform[3] * Math.PI / 180, 0, 0, 0,
        numberParam("offset", 0), 0, 0, 0,
        transform[0], transform[1], 0, 0,
    ]));
    var outputFormat = frame.output.format;
    if (frame.state.pipeline == null || frame.state.pipelineFormat !== outputFormat) {
        frame.state.pipeline = device.createRenderPipeline({
            layout: 'auto',
            vertex: {
                module: frame.state.module,
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
                module: frame.state.module,
                entryPoint: 'fs_main',
                targets: [{ format: outputFormat }],
            },
            primitive: { topology: 'triangle-list' },
        });
        frame.state.pipelineFormat = outputFormat;
    }
    var bindGroup = device.createBindGroup({
        layout: frame.state.pipeline.getBindGroupLayout(0),
        entries: [
            { binding: 0, resource: { buffer: frame.state.uniformBuf } },
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
    "shape": {
        type: "number",
        label: "Shape",
        defaultValue: 0,
        control: "select",
        options: [{ "value": 0, "label": "Triangle" }, { "value": 1, "label": "Circle" }, { "value": 2, "label": "Square" }],
    },
    "ringCount": {
        type: "number",
        label: "Ring count",
        defaultValue: 20,
        control: "slider",
        min: 1,
        max: 30,
        step: 1,
    },
    "falloff": {
        type: "number",
        label: "Falloff",
        defaultValue: 1,
        control: "slider",
        min: 0,
        max: 5,
        step: 0.1,
    },
    "inverseFalloff": {
        type: "boolean",
        label: "Inverse falloff",
        defaultValue: false,
    },
    "color": {
        type: "color",
        label: "Color",
        defaultValue: { "r": 1, "g": 1, "b": 1, "a": 1 },
    },
    "offset": {
        type: "number",
        label: "Offset",
        defaultValue: 0,
        control: "slider",
        min: 0,
        max: 1,
        step: 0.01,
    },
    "transform": {
        type: "point-angle-radius",
        label: "Transform",
        defaultValue: { "x": 50, "y": 50, "radius": 50, "angle": 0 },
        control: "point-angle-radius",
        mode: "canvas_and_ui",
        positionUnit: "%",
        minRadius: 0,
        maxRadius: 250,
    },
});
