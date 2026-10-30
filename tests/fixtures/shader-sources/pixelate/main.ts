import { defineProperties } from "figma:shaders";
export default function Effect() { }
export function setup(device: any, frame: any) {
    var SHADER = `
diagnostic(off,derivative_uniformity);
struct Uniforms {
  region: vec4f,
  pixelSize: vec2f,
  dims: vec2f,
  colorLevels: f32,
  shape: f32,
  knockout: f32,
  gap: f32,
  dissolve: f32,
  dissolveMode: f32,
  averageColor: f32,
  falloff: f32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var inputTex: texture_2d<f32>;

struct VsOut { @builtin(position) pos: vec4f, @location(0) uv: vec2f };

@vertex fn vs_main(@location(0) pos: vec2f, @location(1) uv: vec2f) -> VsOut {
  return VsOut(vec4f(pos, 0.0, 1.0), uv);
}

fn hash21(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, vec3f(p3.y, p3.z, p3.x) + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn posterizeToPalette(color: vec4f, colorCount: f32) -> vec4f {
  let levels = max(floor(colorCount + 0.5), 2.0);
  let steps = levels - 1.0;
  let a = max(color.a, 0.0001);
  let straight = clamp(color.rgb / a, vec3f(0.0), vec3f(1.0));
  let quantized = floor(straight * steps + 0.5) / steps;
  return vec4f(clamp(quantized, vec3f(0.0), vec3f(1.0)) * a, color.a);
}

fn pixelShapeDistance(p: vec2f, shape: f32) -> f32 {
  let q = abs(p);
  if (shape < 0.5) {
    return max(q.x, q.y) - 1.0;
  }
  if (shape < 1.5) {
    return length(p) - 1.0;
  }
  if (shape < 2.5) {
    return max(q.y, q.x + q.y * 0.5) - 1.0;
  }
  let side = q.x - (p.y + 1.0) * 0.5773503;
  return max(max(-1.0 - p.y, p.y - 0.5), side);
}

fn samplePixelAt(pp: vec2f) -> vec4f {
  let bs = max(u.pixelSize.yx, vec2f(1.0));
  let srcUv = clamp(pp / u.dims, vec2f(0.0), vec2f(1.0));

  let center = u.region.xy * u.dims;
  let radius = u.region.z * 0.01 * max(u.dims.x, u.dims.y);
  let angle = u.region.w * 0.01745329;
  let cosA = cos(-angle);
  let sinA = sin(-angle);
  let rel = pp - center;
  let localCoord = vec2f(rel.x * cosA - rel.y * sinA, rel.x * sinA + rel.y * cosA);
  let gapPct = max(u.gap, 0.0);
  let gapPx = gapPct;
  let gridBs = bs + vec2f(gapPx);
  let isHex = u.shape >= 1.5 && u.shape < 2.5;
  let isTri = u.shape >= 2.5;

  var localSample = localCoord;
  var shapeCoord = vec2f(0.0);

  if (isHex) {
    // R from x-dimension; hexYScale is the stretch ratio (1.0 = regular hexagons)
    let R = gridBs.x * 0.5;
    let hexYScale = gridBs.y / gridBs.x;
    // Normalize y into regular hex space before cube-coord snapping
    let nx = localCoord.x;
    let ny = localCoord.y / hexYScale;

    let fq = nx * 0.6666667 / R;
    let fr = (-nx * 0.3333333 + ny * 0.5773503) / R;
    let fsv = -fq - fr;

    var rq = floor(fq + 0.5);
    var rr = floor(fr + 0.5);
    var rs = floor(fsv + 0.5);
    let dq = abs(rq - fq);
    let dr = abs(rr - fr);
    let ds = abs(rs - fsv);
    if (dq > dr && dq > ds) {
      rq = -rr - rs;
    } else if (dr > ds) {
      rr = -rq - rs;
    }

    let hcx = 1.5 * R * rq;
    let hcy = 1.7320508 * R * (rr + rq * 0.5) * hexYScale;
    localSample = vec2f(hcx, hcy);

    // Natural hex half-extents: x=R, y=sqrt(3)/2*R, scaled by hexYScale on y
    let halfExt = max(vec2f(R, 0.8660254 * R * hexYScale), vec2f(0.5));
    let inCell = localCoord - localSample;
    shapeCoord = inCell / halfExt;

  } else if (isTri) {
    let col = floor(localCoord.x / gridBs.x);
    let row = floor(localCoord.y / gridBs.y);
    let fx = (localCoord.x - col * gridBs.x) / gridBs.x;
    let fy = (localCoord.y - row * gridBs.y) / gridBs.y;
    let parity = (col + row) - floor((col + row) * 0.5) * 2.0;

    var inLower = true;
    if (parity < 0.5) {
      inLower = (fx + fy) < 1.0;
    } else {
      inLower = fy < fx;
    }

    var centX = 0.0;
    var centY = 0.0;
    if (parity < 0.5) {
      centX = select(2.0 / 3.0, 1.0 / 3.0, inLower);
      centY = select(2.0 / 3.0, 1.0 / 3.0, inLower);
    } else {
      centX = select(1.0 / 3.0, 2.0 / 3.0, inLower);
      centY = select(2.0 / 3.0, 1.0 / 3.0, inLower);
    }
    localSample = vec2f(col * gridBs.x + centX * gridBs.x, row * gridBs.y + centY * gridBs.y);

    var triPixOff = vec2f((fx - centX) * gridBs.x, (fy - centY) * gridBs.y);
    if (!inLower) { triPixOff = -triPixOff; }
    if (parity >= 0.5) { triPixOff = vec2f(triPixOff.y, triPixOff.x); }
    let triHalfExt = max(min(bs.x, bs.y) * 0.333, 0.5);
    shapeCoord = triPixOff / triHalfExt;

  } else {
    let cell = floor(localCoord / gridBs);
    let cellCenter = (cell + 0.5) * gridBs;
    localSample = cellCenter;
    let radii = max(bs * 0.5, vec2f(0.5));
    let inCell = localCoord - cellCenter;
    shapeCoord = inCell / radii;
  }

  let cosB = cos(angle);
  let sinB = sin(angle);
  let sampleP = center + vec2f(localSample.x * cosB - localSample.y * sinB, localSample.x * sinB + localSample.y * cosB);
  let sampleUv = clamp(sampleP / u.dims, vec2f(0.0), vec2f(1.0));

  let regionMask = length(localSample) <= radius + length(bs) * 0.5;
  let cellHash = hash21(localSample);
  let isScaleDissolve = u.dissolveMode >= 0.5;

  // Radial dropout gradient: cells near edge of region dissolve more
  let distFraction = clamp(length(localSample) / max(radius, 0.001), 0.0, 1.0);
  let falloffBand = smoothstep(1.0 - u.falloff, 1.0, distFraction);
  let effectiveDissolve = mix(u.dissolve, 1.0, falloffBand);

  var scaleFactor = 1.0;
  var dissolved = false;
  if (isScaleDissolve) {
    scaleFactor = clamp(1.0 - effectiveDissolve / max(cellHash, 0.001), 0.0, 1.0);
    dissolved = scaleFactor <= 0.0;
  } else {
    dissolved = cellHash < effectiveDissolve;
  }
  let scaledCoord = shapeCoord / max(scaleFactor, 0.001);
  let canTile = u.shape < 0.5 || isHex || isTri;
  let noGapFill = gapPct <= 0.0 && canTile && scaleFactor >= 1.0;

  // Hard shape clipping — no per-cell edge softening
  let shapeDist = pixelShapeDistance(scaledCoord, u.shape);
  let shapeEdgeAlpha = select(f32(shapeDist <= 0.0), 1.0, noGapFill);
  let finalAlpha = select(0.0, shapeEdgeAlpha, regionMask && !dissolved);

  let srcColor = textureSampleLevel(inputTex, samp, srcUv, 0.0);
  var rawSample = textureSampleLevel(inputTex, samp, sampleUv, 0.0);
  if (u.averageColor > 0.0) {
    let tileStep = (bs * u.averageColor * 0.5) / u.dims;
    var avgSum = vec4f(0.0);
    for (var di: i32 = -1; di <= 1; di++) {
      for (var dj: i32 = -1; dj <= 1; dj++) {
        let off = sampleUv + vec2f(f32(di), f32(dj)) * tileStep;
        avgSum += textureSampleLevel(inputTex, samp, clamp(off, vec2f(0.0), vec2f(1.0)), 0.0);
      }
    }
    rawSample = mix(rawSample, avgSum * (1.0 / 9.0), u.averageColor);
  }
  var pix = posterizeToPalette(rawSample, u.colorLevels);
  let knockoutColor = select(srcColor, vec4f(0.0), u.knockout >= 0.5);
  return mix(knockoutColor, pix, finalAlpha);
}

@fragment fn fs_main(@location(0) uv: vec2f) -> @location(0) vec4f {
  let pp = uv * u.dims;
  let d = 0.25;
  return (
    samplePixelAt(pp + vec2f(-d, -d)) +
    samplePixelAt(pp + vec2f( d, -d)) +
    samplePixelAt(pp + vec2f(-d,  d)) +
    samplePixelAt(pp + vec2f( d,  d))
  ) * 0.25;
}
`;
    frame.state.shaderModule = device.createShaderModule({ code: SHADER });
    frame.state.uniformBuf = device.createBuffer({ size: 64, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    frame.state.sampler = device.createSampler({ magFilter: "nearest", minFilter: "nearest" });
    frame.state.quad = device.createBuffer({ size: 96, usage: GPUBufferUsage.VERTEX, mappedAtCreation: true });
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
    var s = frame.state;
    if (s.pipelineFormat !== frame.output.format) {
        s.pipeline = device.createRenderPipeline({
            layout: "auto",
            vertex: {
                module: s.shaderModule,
                entryPoint: "vs_main",
                buffers: [{
                        arrayStride: 16,
                        attributes: [
                            { shaderLocation: 0, format: "float32x2", offset: 0 },
                            { shaderLocation: 1, format: "float32x2", offset: 8 },
                        ],
                    }],
            },
            fragment: {
                module: s.shaderModule,
                entryPoint: "fs_main",
                targets: [{ format: frame.output.format }],
            },
            primitive: { topology: "triangle-list" },
        });
        s.pipelineFormat = frame.output.format;
    }
    if (frame.input == null)
        return;
    var p = frame.params ?? {};
    var reg = p.region ?? { x: 50, y: 50, radius: 75, angle: 90 };
    var size = p.size ?? p.pixelSizeX ?? p.pixelSizeY ?? 16;
    size = Math.max(size, 2);
    var stretchAmount = (p.stretchSize ?? 100) / 100;
    var pixelSizeX = size;
    var pixelSizeY = size;
    if ((p.stretch ?? 0) < 0.5) {
        pixelSizeX *= stretchAmount;
    }
    else {
        pixelSizeY *= stretchAmount;
    }
    device.queue.writeBuffer(s.uniformBuf, 0, new Float32Array([
        reg.x / 100, reg.y / 100, reg.radius ?? 75, reg.angle ?? 90,
        pixelSizeX, pixelSizeY,
        frame.output.width, frame.output.height,
        (18 - (p.colorLevels ?? 14)), p.pixelShape ?? 0, p.knockout ? 1 : 0, p.gap ?? 0,
        (p.dissolve ?? 0) / 100, p.dissolveMode ?? 0,
        (p.averageColor ?? 0) / 100, (p.falloff ?? 0) / 100,
    ]));
    var bindGroup = device.createBindGroup({
        layout: s.pipeline.getBindGroupLayout(0),
        entries: [
            { binding: 0, resource: { buffer: s.uniformBuf } },
            { binding: 1, resource: s.sampler },
            { binding: 2, resource: frame.input.createView() },
        ],
    });
    var encoder = device.createCommandEncoder();
    var pass = encoder.beginRenderPass({
        colorAttachments: [{
                view: frame.output.createView(),
                loadOp: "clear",
                clearValue: { r: 0, g: 0, b: 0, a: 0 },
                storeOp: "store",
            }],
    });
    pass.setPipeline(s.pipeline);
    pass.setBindGroup(0, bindGroup);
    pass.setVertexBuffer(0, s.quad);
    pass.draw(6);
    pass.end();
    device.queue.submit([encoder.finish()]);
}
defineProperties(Effect, {
    pixelShape: {
        type: "number",
        label: "Shape",
        defaultValue: 0,
        control: "select",
        options: [
            { value: 0, label: "Rectangle" },
            { value: 1, label: "Ellipse" },
            { value: 2, label: "Hexagon" },
            { value: 3, label: "Triangle" },
        ],
    },
    size: { type: "number", label: "Size", defaultValue: 10, control: "slider", min: 2, max: 100, step: 1 },
    stretchSize: { type: "number", label: "Stretch", defaultValue: 100, control: "slider", min: 0, max: 2000, step: 1, unit: "%" },
    gap: { type: "number", label: "Gap", defaultValue: 0, control: "slider", min: 0, max: 50, step: 1, unit: "px" },
    colorLevels: { type: "number", label: "Color trim", defaultValue: 2, control: "slider", min: 2, max: 16, step: 1 },
    averageColor: { type: "number", label: "Average color", defaultValue: 80, control: "slider", min: 0, max: 100, step: 1, unit: "%" },
    region: { type: "point-angle-radius", label: "Region", defaultValue: { x: 50, y: 50, radius: 75, angle: 90 }, mode: "canvas", positionUnit: "%", radiusUnit: "%" },
    dissolveMode: {
        type: "number",
        label: "Dissolve mode",
        defaultValue: 0,
        control: "select",
        options: [
            { value: 0, label: "Dropout" },
            { value: 1, label: "Scale" },
        ],
    },
    dissolve: { type: "number", label: "Dissolve", defaultValue: 0, control: "slider", min: 0, max: 100, step: 1, unit: "%" },
    falloff: { type: "number", label: "Falloff", defaultValue: 0, control: "slider", min: 0, max: 100, step: 1, unit: "%" },
    knockout: { type: "boolean", label: "Knockout", defaultValue: true },
});
