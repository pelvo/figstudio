import { defineProperties } from "figma:shaders";
export default function Effect() { }
export function setup(device: any, frame: any) {
    var shaderCode = `
diagnostic(off,derivative_uniformity);

struct Uniforms {
  cloudColor:        vec4f,
  cloudShadowColor:  vec4f,
  skyGradC0:         vec4f,
  skyGradC1:         vec4f,
  skyGradC2:         vec4f,
  skyGradC3:         vec4f,
  skyGradP:          vec4f,
  skyGradN:          f32,
  _sgPad0:           f32,
  _sgPad1:           f32,
  _sgPad2:           f32,
  globalPosX:        f32,
  globalPosY:        f32,
  globalRotation:    f32,
  globalZoom:        f32,
  evolution:         f32,
  globalDetail:      f32,
  _pad0:             f32,
  _pad1:             f32,
  coverage:          f32,
  density:           f32,
  brightness:        f32,
  shadingVariation:  f32,
  warpAmount:        f32,
  warpScale:         f32,
  stretch:           f32,
  _padSA:            f32,
  cloudPosX:         f32,
  cloudPosY:         f32,
  cloudRotation:     f32,
  time:              f32,
  dimX:              f32,
  dimY:              f32,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
}

@vertex
fn vs_main(@location(0) pos: vec2f, @location(1) uv: vec2f) -> VsOut {
  var out: VsOut;
  out.position = vec4f(pos, 0.0, 1.0);
  out.uv = uv;
  return out;
}

fn hash2(p: vec2f) -> vec2f {
  var q = vec2f(dot(p, vec2f(127.1, 311.7)), dot(p, vec2f(269.5, 183.3)));
  return -1.0 + 2.0 * fract(sin(q) * 43758.5453123);
}

fn noise(p: vec2f) -> f32 {
  let K1 = 0.366025404;
  let K2 = 0.211324865;
  let i = floor(p + (p.x + p.y) * K1);
  let a = p - i + (i.x + i.y) * K2;
  var o: vec2f;
  if (a.x > a.y) {
    o = vec2f(1.0, 0.0);
  } else {
    o = vec2f(0.0, 1.0);
  }
  let b = a - o + K2;
  let c = a - 1.0 + 2.0 * K2;
  let h = max(0.5 - vec3f(dot(a, a), dot(b, b), dot(c, c)), vec3f(0.0));
  let n = h * h * h * h * vec3f(dot(a, hash2(i)), dot(b, hash2(i + o)), dot(c, hash2(i + 1.0)));
  return dot(n, vec3f(70.0));
}

fn rotate_scale(v: vec2f) -> vec2f {
  return vec2f(1.6 * v.x + 1.2 * v.y, -1.2 * v.x + 1.6 * v.y);
}

fn rot2(a: f32) -> mat2x2f {
  let s = sin(a);
  let c = cos(a);
  return mat2x2f(c, s, -s, c);
}

fn field(uv0: vec2f, octaves: i32, startW: f32, decay: f32, ridged: bool, t: f32) -> f32 {
  var v = uv0;
  var total = 0.0;
  var w = startW;
  for (var i = 0; i < octaves; i = i + 1) {
    var s = noise(v);
    if (ridged) { s = abs(s); }
    total = total + w * s;
    v = rotate_scale(v) + t;
    w = w * decay;
  }
  return total;
}

fn applyGlobal(uvA: vec2f, aspect: f32) -> vec2f {
  let centerOff = vec2f(0.5 * aspect, 0.5);
  var v = uvA - centerOff;
  v = v * u.globalZoom;
  v = v + centerOff;
  v = v + vec2f(u.globalPosX, u.globalPosY);
  return v;
}

fn applyLayer(uvA: vec2f, aspect: f32, lRotDeg: f32, lPos: vec2f) -> vec2f {
  let centerOff = vec2f(0.5 * aspect, 0.5);
  var v = uvA - centerOff;
  v = rot2(radians(lRotDeg)) * v;
  v = v + centerOff;
  v = v + lPos;
  return v;
}

fn applyStretch(uvA: vec2f, aspect: f32, stretch: f32, angleDeg: f32) -> vec2f {
  let centerOff = vec2f(0.5 * aspect, 0.5);
  let a = radians(angleDeg);
  var pp = uvA - centerOff;
  pp = rot2(-a) * pp;
  pp.x = pp.x / stretch;
  pp = rot2(a) * pp;
  pp = pp + centerOff;
  return pp;
}

fn cloudDensity(uv: vec2f) -> f32 {
  let cd = clamp(1.0 * u.globalDetail, 0.4, 1.4);
  let baseT = u.evolution;
  let cloudT = baseT + 0.0;
  let cs = 1.56;
  let wa = u.warpAmount;
  let q = field(uv * cs * u.warpScale, 7, 0.1, 0.4, false, 0.0);
  let r = field(uv * cs - (q * wa - cloudT), 8, 0.8, 0.7 * cd, true,  cloudT);
  var f = field(uv * cs - (q * wa - cloudT), 8, 0.7, 0.6 * cd, false, cloudT);
  f = f * (r + f);
  return u.coverage + u.density * f * r;
}

fn skyGradient(t: f32) -> vec4f {
  let n = i32(clamp(u.skyGradN, 1.0, 4.0));
  if (t <= u.skyGradP.x) { return u.skyGradC0; }
  if (n >= 2 && t <= u.skyGradP.y) {
    let lt = (t - u.skyGradP.x) / max(u.skyGradP.y - u.skyGradP.x, 0.0001);
    return mix(u.skyGradC0, u.skyGradC1, clamp(lt, 0.0, 1.0));
  }
  if (n >= 3 && t <= u.skyGradP.z) {
    let lt = (t - u.skyGradP.y) / max(u.skyGradP.z - u.skyGradP.y, 0.0001);
    return mix(u.skyGradC1, u.skyGradC2, clamp(lt, 0.0, 1.0));
  }
  if (n >= 4 && t <= u.skyGradP.w) {
    let lt = (t - u.skyGradP.z) / max(u.skyGradP.w - u.skyGradP.z, 0.0001);
    return mix(u.skyGradC2, u.skyGradC3, clamp(lt, 0.0, 1.0));
  }
  if (n == 1) { return u.skyGradC0; }
  if (n == 2) { return u.skyGradC1; }
  if (n == 3) { return u.skyGradC2; }
  return u.skyGradC3;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let dims = vec2f(u.dimX, u.dimY);
  let p = in.uv;
  let aspect = dims.x / dims.y;
  let aspectV = vec2f(aspect, 1.0);

  let baseA = p * aspectV;
  let g = applyGlobal(baseA, aspect);
  let gs = applyStretch(g, aspect, u.stretch, 0.0);
  let cloudUV = applyLayer(gs, aspect, u.cloudRotation, vec2f(u.cloudPosX, u.cloudPosY));

  let cd = clamp(1.0 * u.globalDetail, 0.4, 1.4);

  let baseT   = u.evolution;
  let cloudT  = baseT + 0.0;
  let cloudT2 = baseT * 2.0 + 0.0;
  let cloudT3 = baseT * 3.0 + 0.0;

  let cs = 1.56;
  let wa = u.warpAmount;

  let q = field(cloudUV * cs * u.warpScale, 7, 0.1, 0.4, false, 0.0);

  let r = field(cloudUV * cs - (q * wa - cloudT), 8, 0.8, 0.7 * cd, true,  cloudT);
  var f = field(cloudUV * cs - (q * wa - cloudT), 8, 0.7, 0.6 * cd, false, cloudT);
  f = f * (r + f);

  var c  = field(cloudUV * cs * 2.0 - (q * wa - cloudT2), 7, 0.4, 0.6 * cd, false, cloudT2);
  let c1 = field(cloudUV * cs * 3.0 - (q * wa - cloudT3), 7, 0.4, 0.6 * cd, true,  cloudT3);
  c = c + c1;

  let fd = u.coverage + u.density * f * r;

  let sunRad = radians(140.0);
  let sunDir = vec2f(cos(sunRad), sin(sunRad));
  let dLit = cloudDensity(cloudUV + sunDir * 0.15);
  let diffuse = clamp((fd - dLit) / (u.density * 0.15 + 0.001), 0.0, 1.0);

  let skySample = skyGradient(p.y);
  let skycolour = skySample.rgb;
  let skyAlpha  = skySample.a;
  let shading = clamp(u.brightness + u.shadingVariation * c + 0.15 * diffuse, 0.0, 1.0);
  let cloudcolour = mix(u.cloudShadowColor.rgb, u.cloudColor.rgb, shading);
  let cloudColorFinal = cloudcolour;

  let blend = clamp(fd + c, 0.0, 1.0);

  let skyFull = vec4f(skycolour, skyAlpha);
  let base = mix(vec4f(0.0, 0.0, 0.0, 0.0), skyFull, skyAlpha);
  var result = mix(base, vec4f(cloudColorFinal, 1.0), blend);
  return result;
}
`;
    frame.state.shaderModule = device.createShaderModule({ code: shaderCode });
    frame.state.uniformBuf = device.createBuffer({
        size: 224,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
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
    var p = frame.params;
    var ca = function (col: any) { return (col && col.a !== undefined) ? col.a : 1.0; };
    var time = 0.0;
    var w = frame.output.width;
    var h = frame.output.height;
    // Build sky gradient stops
    var stops = p.skyColor && p.skyColor.stops ? p.skyColor.stops : [];
    var n = Math.min(stops.length, 4);
    if (n === 0) {
        stops = [
            { color: { r: 0.329, g: 0.588, b: 0.796, a: 1 }, position: 0 },
            { color: { r: 0.188, g: 0.427, b: 0.694, a: 1 }, position: 1 }
        ];
        n = 2;
    }
    stops = stops.slice(0, n).sort(function (a: any, b: any) { return a.position - b.position; });
    var lastStop = stops[stops.length - 1];
    while (stops.length < 4) {
        stops.push(lastStop);
    }
    var data = new Float32Array(56);
    // cloudColor: f[0-3]
    data[0] = p.cloudColor.r;
    data[1] = p.cloudColor.g;
    data[2] = p.cloudColor.b;
    data[3] = ca(p.cloudColor);
    // cloudShadowColor: f[4-7]
    data[4] = p.cloudShadowColor.r;
    data[5] = p.cloudShadowColor.g;
    data[6] = p.cloudShadowColor.b;
    data[7] = ca(p.cloudShadowColor);
    // skyGradC0: f[8-11]
    data[8] = stops[0].color.r;
    data[9] = stops[0].color.g;
    data[10] = stops[0].color.b;
    data[11] = stops[0].color.a !== undefined ? stops[0].color.a : 1.0;
    // skyGradC1: f[12-15]
    data[12] = stops[1].color.r;
    data[13] = stops[1].color.g;
    data[14] = stops[1].color.b;
    data[15] = stops[1].color.a !== undefined ? stops[1].color.a : 1.0;
    // skyGradC2: f[16-19]
    data[16] = stops[2].color.r;
    data[17] = stops[2].color.g;
    data[18] = stops[2].color.b;
    data[19] = stops[2].color.a !== undefined ? stops[2].color.a : 1.0;
    // skyGradC3: f[20-23]
    data[20] = stops[3].color.r;
    data[21] = stops[3].color.g;
    data[22] = stops[3].color.b;
    data[23] = stops[3].color.a !== undefined ? stops[3].color.a : 1.0;
    // skyGradP: f[24-27]
    data[24] = stops[0].position;
    data[25] = stops[1].position;
    data[26] = stops[2].position;
    data[27] = stops[3].position;
    // skyGradN + padding: f[28-31]
    data[28] = n;
    data[29] = 0.0;
    data[30] = 0.0;
    data[31] = 0.0;
    // globalTransform handle: XY = zoom pivot, angle = cloud rotation, radius = zoom level
    var gt = p.globalTransform;
    var aspect = w / h;
    data[32] = -((gt.x - 50) / 100); // globalPosX: pan offset (0 at center, ±0.5 range)
    data[33] = -((gt.y - 50) / 100); // globalPosY: pan offset (0 at center, ±0.5 range)
    data[34] = 0.0; // globalRotation hardcoded
    data[35] = 0.2 + (1.0 - gt.radius / 100) * 2.8; // globalZoom
    var pctEvolution = p.evolution;
    data[36] = -5.0 + (pctEvolution / 100) * 10.0;
    var pctGlobalDetail = p.globalDetail;
    data[37] = 0.5 + (pctGlobalDetail / 100) * 0.8;
    data[38] = 0.0; // _pad0
    data[39] = 0.0; // _pad1
    var pctCoverage = p.coverage;
    data[40] = -0.2 + (pctCoverage / 100) * 1.2;
    var pctDensity = p.density;
    data[41] = 1.0 + (pctDensity / 100) * 19.0;
    var pctBrightness = p.brightness;
    data[42] = pctBrightness / 100;
    var pctShadingVariation = p.shadingVariation;
    data[43] = pctShadingVariation / 100;
    var pctWarpAmount = p.warpAmount;
    data[44] = (pctWarpAmount / 100) * 3.0;
    var pctWarpScale = p.warpScale;
    data[45] = 0.1 + (pctWarpScale / 100) * 1.9;
    var pctStretch = p.stretch;
    data[46] = 1.0 + (pctStretch / 100) * 5.0;
    data[47] = 0.0; // _padSA
    data[48] = 0.0; // cloudPosX hardcoded
    data[49] = 0.0; // cloudPosY hardcoded
    data[50] = -gt.angle; // cloudRotation driven by handle angle
    data[51] = time;
    data[52] = w;
    data[53] = h;
    device.queue.writeBuffer(frame.state.uniformBuf, 0, data);
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
        ],
    });
    var encoder = device.createCommandEncoder();
    var pass = encoder.beginRenderPass({
        colorAttachments: [{
                view: frame.output.createView(),
                loadOp: 'clear',
                clearValue: { r: 0, g: 0, b: 0, a: 1 },
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
    cloudColor: { type: "color", label: "Cloud color", defaultValue: { r: 0.929, g: 0.973, b: 1.0, a: 1.0 } },
    cloudShadowColor: { type: "color", label: "Shadow color", defaultValue: { r: 0.267, g: 0.518, b: 0.800, a: 1.0 } },
    skyColor: {
        type: "gradient",
        label: "Sky color",
        defaultValue: {
            stops: [
                { color: { r: 0.0, g: 0.161, b: 0.471, a: 1.0 }, position: 0.0 },
                { color: { r: 0.329, g: 0.588, b: 0.796, a: 1.0 }, position: 1.0 }
            ]
        }
    },
    coverage: { type: "number", label: "Coverage", defaultValue: 50, control: "slider", min: 0, max: 100, step: 1 },
    density: { type: "number", label: "Density", defaultValue: 50, control: "slider", min: 0, max: 100, step: 1 },
    brightness: { type: "number", label: "Brightness", defaultValue: 70, control: "slider", min: 0, max: 100, step: 1 },
    globalDetail: { type: "number", label: "Detail", defaultValue: 50, control: "slider", min: 0, max: 100, step: 1 },
    shadingVariation: { type: "number", label: "Variation", defaultValue: 50, control: "slider", min: 0, max: 100, step: 1 },
    warpAmount: { type: "number", label: "Warp amount", defaultValue: 0, control: "slider", min: 0, max: 100, step: 1 },
    warpScale: { type: "number", label: "Warp scale", defaultValue: 0, control: "slider", min: 0, max: 100, step: 1 },
    stretch: { type: "number", label: "Stretch", defaultValue: 0, control: "slider", min: 0, max: 100, step: 1 },
    evolution: { type: "number", label: "Phase", defaultValue: 0, control: "slider", min: 0, max: 100, step: 1 },
    globalTransform: { type: "point-angle-radius", label: "Transform", positionUnit: "%", radiusUnit: "%", defaultValue: { x: 50, y: 50, angle: 0, radius: 60 } },
});
