import { defineProperties } from "figma:shaders";
export default function Effect() { }
export function setup(device: any, frame: any) {
    // Forward tessellation: a triangle grid in parameter space (s,t) is warped by
    // the bicubic mesh in the VERTEX shader; the rasterizer Gouraud-interpolates
    // color across triangles. No per-pixel inverse solve.
    //
    // Anti-aliasing: 4x MSAA. The mesh is rendered into a multisampled target,
    // then a manual resolve pass un-premultiplies (color stays full strength,
    // alpha = sub-pixel coverage). This avoids the dark edge you get when the
    // transparent black clear is averaged into the color along the silhouette.
    var meshWgsl = `
struct Uniforms {
  frameData: vec4f,
  inputDimsData: vec4f,
  p00: vec4f,  p00Color: vec4f,
  p10: vec4f,  p10Color: vec4f,
  p20: vec4f,  p20Color: vec4f,
  p30: vec4f,  p30Color: vec4f,
  p01: vec4f,  p01Color: vec4f,
  p11: vec4f,  p11Color: vec4f,
  p21: vec4f,  p21Color: vec4f,
  p31: vec4f,  p31Color: vec4f,
  p02: vec4f,  p02Color: vec4f,
  p12: vec4f,  p12Color: vec4f,
  p22: vec4f,  p22Color: vec4f,
  p32: vec4f,  p32Color: vec4f,
  p03: vec4f,  p03Color: vec4f,
  p13: vec4f,  p13Color: vec4f,
  p23: vec4f,  p23Color: vec4f,
  p33: vec4f,  p33Color: vec4f,
  reserved0: vec4f,
  reserved1: vec4f,
  reserved2: vec4f,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) color: vec4f,   // linear-light + alpha, Gouraud-interpolated
};

fn cr1(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
  return 0.5*((2.0*p1) + (-p0+p2)*t + (2.0*p0-5.0*p1+4.0*p2-p3)*t*t + (-p0+3.0*p1-3.0*p2+p3)*t*t*t);
}
fn cr2(p0: vec2f, p1: vec2f, p2: vec2f, p3: vec2f, t: f32) -> vec2f {
  return vec2f(cr1(p0.x,p1.x,p2.x,p3.x,t), cr1(p0.y,p1.y,p2.y,p3.y,t));
}
fn cr3(p0: vec3f, p1: vec3f, p2: vec3f, p3: vec3f, t: f32) -> vec3f {
  return vec3f(cr1(p0.x,p1.x,p2.x,p3.x,t), cr1(p0.y,p1.y,p2.y,p3.y,t), cr1(p0.z,p1.z,p2.z,p3.z,t));
}
fn cr4(p0: vec4f, p1: vec4f, p2: vec4f, p3: vec4f, t: f32) -> vec4f {
  return vec4f(cr1(p0.x,p1.x,p2.x,p3.x,t), cr1(p0.y,p1.y,p2.y,p3.y,t), cr1(p0.z,p1.z,p2.z,p3.z,t), cr1(p0.w,p1.w,p2.w,p3.w,t));
}
fn ix(i: i32, j: i32, cols: i32, rows: i32) -> i32 {
  let ci = clamp(i, 0, cols - 1);
  let cj = clamp(j, 0, rows - 1);
  return cj * cols + ci;
}
fn srgb2lin(c: vec3f) -> vec3f {
  let lo = c / 12.92;
  let hi = pow((c + 0.055) / 1.055, vec3f(2.4));
  return select(lo, hi, c > vec3f(0.04045));
}
fn lin2srgb(c: vec3f) -> vec3f {
  let lo = c * 12.92;
  let hi = 1.055 * pow(clamp(c, vec3f(0.0), vec3f(1.0)), vec3f(1.0/2.4)) - 0.055;
  return select(lo, hi, c > vec3f(0.0031308));
}

@vertex fn vs_main(@location(0) st: vec2f) -> VsOut {
  var pos_arr: array<vec2f, 16>;
  var col_arr: array<vec4f, 16>;
  pos_arr[0]  = u.p00.xy / 100.0; col_arr[0]  = vec4f(srgb2lin(u.p00Color.rgb), u.p00Color.a);
  pos_arr[1]  = u.p10.xy / 100.0; col_arr[1]  = vec4f(srgb2lin(u.p10Color.rgb), u.p10Color.a);
  pos_arr[2]  = u.p20.xy / 100.0; col_arr[2]  = vec4f(srgb2lin(u.p20Color.rgb), u.p20Color.a);
  pos_arr[3]  = u.p30.xy / 100.0; col_arr[3]  = vec4f(srgb2lin(u.p30Color.rgb), u.p30Color.a);
  pos_arr[4]  = u.p01.xy / 100.0; col_arr[4]  = vec4f(srgb2lin(u.p01Color.rgb), u.p01Color.a);
  pos_arr[5]  = u.p11.xy / 100.0; col_arr[5]  = vec4f(srgb2lin(u.p11Color.rgb), u.p11Color.a);
  pos_arr[6]  = u.p21.xy / 100.0; col_arr[6]  = vec4f(srgb2lin(u.p21Color.rgb), u.p21Color.a);
  pos_arr[7]  = u.p31.xy / 100.0; col_arr[7]  = vec4f(srgb2lin(u.p31Color.rgb), u.p31Color.a);
  pos_arr[8]  = u.p02.xy / 100.0; col_arr[8]  = vec4f(srgb2lin(u.p02Color.rgb), u.p02Color.a);
  pos_arr[9]  = u.p12.xy / 100.0; col_arr[9]  = vec4f(srgb2lin(u.p12Color.rgb), u.p12Color.a);
  pos_arr[10] = u.p22.xy / 100.0; col_arr[10] = vec4f(srgb2lin(u.p22Color.rgb), u.p22Color.a);
  pos_arr[11] = u.p32.xy / 100.0; col_arr[11] = vec4f(srgb2lin(u.p32Color.rgb), u.p32Color.a);
  pos_arr[12] = u.p03.xy / 100.0; col_arr[12] = vec4f(srgb2lin(u.p03Color.rgb), u.p03Color.a);
  pos_arr[13] = u.p13.xy / 100.0; col_arr[13] = vec4f(srgb2lin(u.p13Color.rgb), u.p13Color.a);
  pos_arr[14] = u.p23.xy / 100.0; col_arr[14] = vec4f(srgb2lin(u.p23Color.rgb), u.p23Color.a);
  pos_arr[15] = u.p33.xy / 100.0; col_arr[15] = vec4f(srgb2lin(u.p33Color.rgb), u.p33Color.a);

  let uu = st.x * 3.0;
  let vv = st.y * 3.0;
  let i = clamp(i32(floor(uu)), 0, 2);
  let j = clamp(i32(floor(vv)), 0, 2);
  let fu = uu - f32(i);
  let fv = vv - f32(j);

  var prow: array<vec2f, 4>;
  var crow: array<vec4f, 4>;
  for (var m = 0; m < 4; m = m + 1) {
    let jj = j - 1 + m;
    let a0 = pos_arr[ix(i-1, jj, 4, 4)];
    let a1 = pos_arr[ix(i,   jj, 4, 4)];
    let a2 = pos_arr[ix(i+1, jj, 4, 4)];
    let a3 = pos_arr[ix(i+2, jj, 4, 4)];
    prow[m] = cr2(a0, a1, a2, a3, fu);
    let c0 = col_arr[ix(i-1, jj, 4, 4)];
    let c1 = col_arr[ix(i,   jj, 4, 4)];
    let c2 = col_arr[ix(i+1, jj, 4, 4)];
    let c3 = col_arr[ix(i+2, jj, 4, 4)];
    crow[m] = cr4(c0, c1, c2, c3, fu);
  }
  let pos = cr2(prow[0], prow[1], prow[2], prow[3], fv);
  let col = cr4(crow[0], crow[1], crow[2], crow[3], fv);

  var out: VsOut;
  out.position = vec4f(pos.x * 2.0 - 1.0, 1.0 - pos.y * 2.0, 0.0, 1.0);
  out.color = vec4f(col.rgb, clamp(col.a, 0.0, 1.0));
  return out;
}

@fragment fn fs_main(in: VsOut) -> @location(0) vec4f {
  let lin = clamp(in.color.rgb, vec3f(0.0), vec3f(1.0));
  let alpha = in.color.a;
  return vec4f(lin2srgb(lin), alpha);
}
`;
    // Manual MSAA resolve. Coverage comes from rasterization: covered samples hold
    // (color, 1), uncovered samples keep the clear (0,0,0,0). Accumulate premultiplied
    // color and divide it back out so transparent black never tints the edge; emit
    // alpha = mean coverage. Output is straight (non-premultiplied) alpha.
    var resolveWgsl = `
@group(0) @binding(0) var ms: texture_multisampled_2d<f32>;

@vertex fn vs_resolve(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4f {
  var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(p[vid], 0.0, 1.0);
}

@fragment fn fs_resolve(@builtin(position) fragCoord: vec4f) -> @location(0) vec4f {
  let coord = vec2i(i32(fragCoord.x), i32(fragCoord.y));
  let n = 4;
  var accColor = vec3f(0.0);
  var accAlpha = 0.0;
  for (var s = 0; s < n; s = s + 1) {
    let t = textureLoad(ms, coord, s);
    accColor = accColor + t.rgb * t.a;
    accAlpha = accAlpha + t.a;
  }
  let rgb = accColor / max(accAlpha, 1e-5);
  let a = accAlpha / f32(n);
  return vec4f(rgb, a);
}
`;
    frame.state.module = device.createShaderModule({ code: meshWgsl });
    frame.state.resolveModule = device.createShaderModule({ code: resolveWgsl });
    frame.state.pipeline = null;
    frame.state.pipelineFormat = null;
    frame.state.resolvePipeline = null;
    frame.state.resolveFormat = null;
    frame.state.tess = 0;
    frame.state.vbuf = null;
    frame.state.ibuf = null;
    frame.state.indexCount = 0;
    frame.state.msTex = null;
    frame.state.msW = 0;
    frame.state.msH = 0;
    frame.state.msFmt = null;
    frame.state.uniformBuf = device.createBuffer({
        size: 592,
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
    var output = frame.output || {};
    var width = Math.max(1, finiteNumber(output.width, 1));
    var height = Math.max(1, finiteNumber(output.height, 1));
    var outputFormat = frame.output.format;
    // ---- Tessellation grid (rebuilt only when the quality slider changes) ----
    var tess = Math.max(8, Math.min(256, Math.round(numberParam("tessellation", 128))));
    if (frame.state.tess !== tess || frame.state.vbuf == null) {
        var N = tess + 1;
        var vdata = new Float32Array(N * N * 2);
        var o = 0;
        for (var b = 0; b < N; b++) {
            for (var a = 0; a < N; a++) {
                vdata[o++] = a / tess;
                vdata[o++] = b / tess;
            }
        }
        var idata = new Uint32Array(tess * tess * 6);
        o = 0;
        for (var b2 = 0; b2 < tess; b2++) {
            for (var a2 = 0; a2 < tess; a2++) {
                var i0 = b2 * N + a2, i1 = i0 + 1, i2 = i0 + N, i3 = i2 + 1;
                idata[o++] = i0;
                idata[o++] = i2;
                idata[o++] = i1;
                idata[o++] = i1;
                idata[o++] = i2;
                idata[o++] = i3;
            }
        }
        if (frame.state.vbuf && frame.state.vbuf.destroy)
            frame.state.vbuf.destroy();
        if (frame.state.ibuf && frame.state.ibuf.destroy)
            frame.state.ibuf.destroy();
        frame.state.vbuf = device.createBuffer({
            size: vdata.byteLength, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
        });
        device.queue.writeBuffer(frame.state.vbuf, 0, vdata);
        frame.state.ibuf = device.createBuffer({
            size: idata.byteLength, usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
        });
        device.queue.writeBuffer(frame.state.ibuf, 0, idata);
        frame.state.indexCount = idata.length;
        frame.state.tess = tess;
    }
    // ---- Multisampled target (rebuilt when size / format changes) ----
    if (frame.state.msTex == null || frame.state.msW !== width ||
        frame.state.msH !== height || frame.state.msFmt !== outputFormat) {
        if (frame.state.msTex && frame.state.msTex.destroy)
            frame.state.msTex.destroy();
        frame.state.msTex = device.createTexture({
            size: { width: width, height: height },
            format: outputFormat,
            sampleCount: 4,
            usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
        });
        frame.state.msW = width;
        frame.state.msH = height;
        frame.state.msFmt = outputFormat;
    }
    // ---- Uniforms (layout unchanged; reserved slots zeroed) ----
    device.queue.writeBuffer(frame.state.uniformBuf, 0, new Float32Array([
        0, width, height, 0,
        width, height, 0, 0,
        ...colorPointParam("p00", [0, 0, 1, 0.42, 0.42, 1]),
        ...colorPointParam("p10", [33, 0, 1, 0.64, 0.42, 1]),
        ...colorPointParam("p20", [67, 0, 1, 0.82, 0.42, 1]),
        ...colorPointParam("p30", [100, 0, 1, 0.82, 0.4, 1]),
        ...colorPointParam("p01", [0, 33, 0.7, 0.3, 0.6, 1]),
        ...colorPointParam("p11", [33, 33, 0.8, 0.55, 0.5, 1]),
        ...colorPointParam("p21", [67, 33, 0.9, 0.7, 0.45, 1]),
        ...colorPointParam("p31", [100, 33, 0.5, 0.7, 0.35, 1]),
        ...colorPointParam("p02", [0, 67, 0.4, 0.5, 0.7, 1]),
        ...colorPointParam("p12", [33, 67, 0.35, 0.65, 0.65, 1]),
        ...colorPointParam("p22", [67, 67, 0.2, 0.65, 0.55, 1]),
        ...colorPointParam("p32", [100, 67, 0.1, 0.55, 0.7, 1]),
        ...colorPointParam("p03", [0, 100, 0.02, 0.84, 0.63, 1]),
        ...colorPointParam("p13", [33, 100, 0.1, 0.7, 0.65, 1]),
        ...colorPointParam("p23", [67, 100, 0.1, 0.6, 0.7, 1]),
        ...colorPointParam("p33", [100, 100, 0.07, 0.54, 0.7, 1]),
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
    ]));
    // ---- Pipelines (lazy, keyed on output format) ----
    if (frame.state.pipeline == null || frame.state.pipelineFormat !== outputFormat) {
        frame.state.pipeline = device.createRenderPipeline({
            layout: 'auto',
            vertex: {
                module: frame.state.module,
                entryPoint: 'vs_main',
                buffers: [{ arrayStride: 8, attributes: [{ shaderLocation: 0, format: 'float32x2', offset: 0 }] }],
            },
            fragment: { module: frame.state.module, entryPoint: 'fs_main', targets: [{ format: outputFormat }] },
            primitive: { topology: 'triangle-list' },
            multisample: { count: 4 },
        });
        frame.state.pipelineFormat = outputFormat;
    }
    if (frame.state.resolvePipeline == null || frame.state.resolveFormat !== outputFormat) {
        frame.state.resolvePipeline = device.createRenderPipeline({
            layout: 'auto',
            vertex: { module: frame.state.resolveModule, entryPoint: 'vs_resolve' },
            fragment: { module: frame.state.resolveModule, entryPoint: 'fs_resolve', targets: [{ format: outputFormat }] },
            primitive: { topology: 'triangle-list' },
        });
        frame.state.resolveFormat = outputFormat;
    }
    var meshBind = device.createBindGroup({
        layout: frame.state.pipeline.getBindGroupLayout(0),
        entries: [{ binding: 0, resource: { buffer: frame.state.uniformBuf } }],
    });
    var resolveBind = device.createBindGroup({
        layout: frame.state.resolvePipeline.getBindGroupLayout(0),
        entries: [{ binding: 0, resource: frame.state.msTex.createView() }],
    });
    var encoder = device.createCommandEncoder();
    // Pass 1: render the mesh into the multisampled target.
    var pass1 = encoder.beginRenderPass({
        colorAttachments: [{
                view: frame.state.msTex.createView(),
                loadOp: 'clear', clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: 'store',
            }],
    });
    pass1.setPipeline(frame.state.pipeline);
    pass1.setBindGroup(0, meshBind);
    pass1.setVertexBuffer(0, frame.state.vbuf);
    pass1.setIndexBuffer(frame.state.ibuf, 'uint32');
    pass1.drawIndexed(frame.state.indexCount);
    pass1.end();
    // Pass 2: resolve (un-premultiplied) into the output.
    var pass2 = encoder.beginRenderPass({
        colorAttachments: [{
                view: frame.output.createView(),
                loadOp: 'clear', clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: 'store',
            }],
    });
    pass2.setPipeline(frame.state.resolvePipeline);
    pass2.setBindGroup(0, resolveBind);
    pass2.draw(3);
    pass2.end();
    device.queue.submit([encoder.finish()]);
}
defineProperties(Effect, {
    "p00": { type: "color-point", label: "Point (0,0)", defaultValue: { "x": 0, "y": 0, "color": { "r": 1, "g": 0.42, "b": 0.42, "a": 1 } }, mode: "canvas", unit: "%" },
    "p10": { type: "color-point", label: "Point (1,0)", defaultValue: { "x": 33, "y": 0, "color": { "r": 1, "g": 0.64, "b": 0.42, "a": 1 } }, mode: "canvas", unit: "%" },
    "p20": { type: "color-point", label: "Point (2,0)", defaultValue: { "x": 67, "y": 0, "color": { "r": 1, "g": 0.82, "b": 0.42, "a": 1 } }, mode: "canvas", unit: "%" },
    "p30": { type: "color-point", label: "Point (3,0)", defaultValue: { "x": 100, "y": 0, "color": { "r": 1, "g": 0.82, "b": 0.4, "a": 1 } }, mode: "canvas", unit: "%" },
    "p01": { type: "color-point", label: "Point (0,1)", defaultValue: { "x": 0, "y": 33, "color": { "r": 0.7, "g": 0.3, "b": 0.6, "a": 1 } }, mode: "canvas", unit: "%" },
    "p11": { type: "color-point", label: "Point (1,1)", defaultValue: { "x": 33, "y": 33, "color": { "r": 0.8, "g": 0.55, "b": 0.5, "a": 1 } }, mode: "canvas", unit: "%" },
    "p21": { type: "color-point", label: "Point (2,1)", defaultValue: { "x": 67, "y": 33, "color": { "r": 0.9, "g": 0.7, "b": 0.45, "a": 1 } }, mode: "canvas", unit: "%" },
    "p31": { type: "color-point", label: "Point (3,1)", defaultValue: { "x": 100, "y": 33, "color": { "r": 0.5, "g": 0.7, "b": 0.35, "a": 1 } }, mode: "canvas", unit: "%" },
    "p02": { type: "color-point", label: "Point (0,2)", defaultValue: { "x": 0, "y": 67, "color": { "r": 0.4, "g": 0.5, "b": 0.7, "a": 1 } }, mode: "canvas", unit: "%" },
    "p12": { type: "color-point", label: "Point (1,2)", defaultValue: { "x": 33, "y": 67, "color": { "r": 0.35, "g": 0.65, "b": 0.65, "a": 1 } }, mode: "canvas", unit: "%" },
    "p22": { type: "color-point", label: "Point (2,2)", defaultValue: { "x": 67, "y": 67, "color": { "r": 0.2, "g": 0.65, "b": 0.55, "a": 1 } }, mode: "canvas", unit: "%" },
    "p32": { type: "color-point", label: "Point (3,2)", defaultValue: { "x": 100, "y": 67, "color": { "r": 0.1, "g": 0.55, "b": 0.7, "a": 1 } }, mode: "canvas", unit: "%" },
    "p03": { type: "color-point", label: "Point (0,3)", defaultValue: { "x": 0, "y": 100, "color": { "r": 0.02, "g": 0.84, "b": 0.63, "a": 1 } }, mode: "canvas", unit: "%" },
    "p13": { type: "color-point", label: "Point (1,3)", defaultValue: { "x": 33, "y": 100, "color": { "r": 0.1, "g": 0.7, "b": 0.65, "a": 1 } }, mode: "canvas", unit: "%" },
    "p23": { type: "color-point", label: "Point (2,3)", defaultValue: { "x": 67, "y": 100, "color": { "r": 0.1, "g": 0.6, "b": 0.7, "a": 1 } }, mode: "canvas", unit: "%" },
    "p33": { type: "color-point", label: "Point (3,3)", defaultValue: { "x": 100, "y": 100, "color": { "r": 0.07, "g": 0.54, "b": 0.7, "a": 1 } }, mode: "canvas", unit: "%" },
    "tessellation": {
        type: "number",
        label: "Tessellation",
        defaultValue: 128,
        control: "slider",
        min: 2,
        max: 256,
        step: 1,
    },
});
