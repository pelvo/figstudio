import { defineProperties } from "figma:shaders"

export default function Effect() {}

function finiteNumber(value, fallback) {
  const number = Number(value)
  return Number.isFinite(number) ? number : fallback
}

function numberParam(params, name, fallback) {
  return finiteNumber(params[name], fallback)
}

function stringParam(params, name, fallback) {
  const value = params[name]
  if (value && typeof value.characters === "string") {
    return value.characters
  }
  return typeof value === "string" ? value : fallback
}

function writeUniformsIfChanged(device, buffer, data, lastData) {
  let changed = false
  for (let i = 0; i < data.length; i++) {
    if (!Object.is(data[i], lastData[i])) {
      changed = true
      break
    }
  }

  if (changed) {
    device.queue.writeBuffer(buffer, 0, data)
    lastData.set(data)
  }
}

export function setup(device, frame) {
  const uniformsWGSL = `
diagnostic(off,derivative_uniformity);

struct Uniforms {
  frequency: f32,
  threshold: f32,
  softness: f32,
  pad0: f32,
  strength: f32,
  chromaticAberration: f32,
  detail: f32,
  edgeOnly: f32,
  outputWidth: f32,
  outputHeight: f32,
  shadow: f32,
  mode: f32,
  outputPad: f32,
  inputPad: f32,
  coreWidth: f32,
  coreHeight: f32,
  loopCos: f32,
  loopSin: f32,
  hueCos: f32,
  hueSin: f32,
};

struct VertexOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};
`

  const vertexWGSL = `
diagnostic(off,derivative_uniformity);

struct VertexOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
  let positions = array<vec2f, 3>(
    vec2f(-1.0, -1.0),
    vec2f( 3.0, -1.0),
    vec2f(-1.0,  3.0),
  );
  let p = positions[index];
  var out: VertexOut;
  out.position = vec4f(p, 0.0, 1.0);
  out.uv = vec2f(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
  return out;
}
`

  const fieldWGSL = uniformsWGSL + `
@group(0) @binding(0) var<uniform> u: Uniforms;

fn hashBits4(p: vec4i) -> u32 {
  var h = bitcast<u32>(p.x) * 0x8da6b343u;
  h = h ^ (bitcast<u32>(p.y) * 0xd8163841u);
  h = h ^ (bitcast<u32>(p.z) * 0xcb1ab31fu);
  h = h ^ (bitcast<u32>(p.w) * 0x165667b1u);
  h = (h ^ (h >> 16u)) * 0x7feb352du;
  h = (h ^ (h >> 15u)) * 0x846ca68bu;
  return h ^ (h >> 16u);
}

fn gradient4(cell: vec4i) -> vec4f {
  let h = hashBits4(cell);
  let channels = vec4u(
    h & 255u,
    (h >> 8u) & 255u,
    (h >> 16u) & 255u,
    (h >> 24u) & 255u,
  );
  let gradient = vec4f(channels) * (2.0 / 255.0) - 1.0;
  return normalize(gradient + vec4f(0.0001));
}

fn simplexCorner(cell: vec4i, offset: vec4f) -> f32 {
  let falloff = 0.6 - dot(offset, offset);
  if (falloff <= 0.0) {
    return 0.0;
  }
  let falloff2 = falloff * falloff;
  return falloff2 * falloff2 * dot(gradient4(cell), offset);
}

fn noise4(p: vec4f) -> f32 {
  let F4 = 0.30901699437494745;
  let G4 = 0.1381966011250105;
  let cell = vec4i(floor(p + dot(p, vec4f(F4))));
  let cellf = vec4f(cell);
  let x0 = p - cellf + dot(cellf, vec4f(G4));

  var rank = vec4i(0);
  if (x0.x > x0.y) { rank.x = rank.x + 1; } else { rank.y = rank.y + 1; }
  if (x0.x > x0.z) { rank.x = rank.x + 1; } else { rank.z = rank.z + 1; }
  if (x0.x > x0.w) { rank.x = rank.x + 1; } else { rank.w = rank.w + 1; }
  if (x0.y > x0.z) { rank.y = rank.y + 1; } else { rank.z = rank.z + 1; }
  if (x0.y > x0.w) { rank.y = rank.y + 1; } else { rank.w = rank.w + 1; }
  if (x0.z > x0.w) { rank.z = rank.z + 1; } else { rank.w = rank.w + 1; }

  let i1 = vec4i(
    select(0, 1, rank.x >= 3),
    select(0, 1, rank.y >= 3),
    select(0, 1, rank.z >= 3),
    select(0, 1, rank.w >= 3),
  );
  let i2 = vec4i(
    select(0, 1, rank.x >= 2),
    select(0, 1, rank.y >= 2),
    select(0, 1, rank.z >= 2),
    select(0, 1, rank.w >= 2),
  );
  let i3 = vec4i(
    select(0, 1, rank.x >= 1),
    select(0, 1, rank.y >= 1),
    select(0, 1, rank.z >= 1),
    select(0, 1, rank.w >= 1),
  );

  let x1 = x0 - vec4f(i1) + vec4f(G4);
  let x2 = x0 - vec4f(i2) + vec4f(2.0 * G4);
  let x3 = x0 - vec4f(i3) + vec4f(3.0 * G4);
  let x4 = x0 - 1.0 + vec4f(4.0 * G4);

  let sum =
    simplexCorner(cell, x0) +
    simplexCorner(cell + i1, x1) +
    simplexCorner(cell + i2, x2) +
    simplexCorner(cell + i3, x3) +
    simplexCorner(cell + vec4i(1), x4);
  return clamp(0.5 + 13.5 * sum, 0.0, 1.0);
}

fn liquidField(p: vec2f, loopA: vec2f, loopB: vec2f, frequency: f32) -> f32 {
  let warpX = noise4(vec4f(
    p * frequency * 0.58 + vec2f(5.3, 1.7),
    loopB * 0.72,
  ));
  let warpY = noise4(vec4f(
    p * frequency * 0.58 + vec2f(-2.1, 8.4),
    loopA * 0.72,
  ));
  let warped = p + (vec2f(warpX, warpY) - 0.5) * 0.52;
  let broad = noise4(vec4f(warped * frequency, loopA * 0.88));

  // When Detail is zero, avoid a fourth 4D-noise evaluation per field call.
  var value = broad;
  if (u.detail > 0.0001) {
    let fine = noise4(vec4f(
      warped * frequency * 1.92 + 11.0,
      loopB * 1.18,
    ));
    value = mix(broad, fine, u.detail);
  }
  return value;
}

@fragment
fn field_fs(in: VertexOut) -> @location(0) vec4f {
  let outputUv = clamp(in.uv, vec2f(0.0), vec2f(1.0));
  let outputPx = outputUv * vec2f(u.outputWidth, u.outputHeight);
  let coreUv =
    (outputPx - vec2f(u.outputPad)) /
    vec2f(u.coreWidth, u.coreHeight);
  let aspect = u.coreWidth / u.coreHeight;
  let p = (coreUv - 0.5) * vec2f(aspect, 1.0);

  let loopA = vec2f(u.loopCos, u.loopSin);
  let loopB = vec2f(-loopA.y, loopA.x);
  let n = liquidField(p, loopA, loopB, u.frequency);

  // A fixed p-space finite difference keeps the gradient smooth and independent
  // per texel. Screen-space derivatives create visible 2x2 blocks once their
  // normals are magnified by strong displacement.
  let e = 0.008;
  let gradient = vec2f(
    liquidField(p + vec2f(e, 0.0), loopA, loopB, u.frequency) - n,
    liquidField(p + vec2f(0.0, e), loopA, loopB, u.frequency) - n,
  ) / e;
  return vec4f(n, gradient, 1.0);
}
`

  const compositeWGSL = uniformsWGSL + `
@group(0) @binding(0) var linearSampler: sampler;
@group(0) @binding(1) var inputTexture: texture_2d<f32>;
@group(0) @binding(2) var fieldTexture: texture_2d<f32>;
@group(0) @binding(3) var<uniform> u: Uniforms;

fn hashPixel(p: vec2u) -> f32 {
  var h = (p.x * 0x8da6b343u) ^ (p.y * 0xd8163841u);
  h = (h ^ (h >> 16u)) * 0x7feb352du;
  h = (h ^ (h >> 15u)) * 0x846ca68bu;
  h = h ^ (h >> 16u);
  return f32(h) * (1.0 / 4294967295.0);
}

fn rotateHue(color: vec3f, cosine: f32, sine: f32) -> vec3f {
  let axis = normalize(vec3f(1.0));
  return color * cosine +
    cross(axis, color) * sine +
    axis * dot(axis, color) * (1.0 - cosine);
}

fn outputToCoreUv(outputUv: vec2f) -> vec2f {
  let outputPx = outputUv * vec2f(u.outputWidth, u.outputHeight);
  return
    (outputPx - vec2f(u.outputPad)) /
    vec2f(u.coreWidth, u.coreHeight);
}

fn sampleInput(coreUv: vec2f) -> vec4f {
  let inputDims = vec2f(textureDimensions(inputTexture));
  let inputPx = coreUv * vec2f(u.coreWidth, u.coreHeight) + vec2f(u.inputPad);
  let halfPixel = 0.5 / inputDims;
  let textureUv = clamp(
    inputPx / inputDims,
    halfPixel,
    vec2f(1.0) - halfPixel,
  );
  return textureSampleLevel(inputTexture, linearSampler, textureUv, 0.0);
}

fn unpremultiply(sample: vec4f) -> vec3f {
  return sample.rgb / max(sample.a, 0.00001);
}

fn fieldMetrics(n: f32) -> vec2f {
  let blob = smoothstep(
    u.threshold - u.softness,
    u.threshold + u.softness,
    n,
  );
  let rim = exp(
    -abs(n - u.threshold) /
    max(u.softness * 0.72, 0.002),
  );
  let edgeBand = pow(rim, 0.72);
  let displacementMask = mix(
    blob,
    edgeBand,
    clamp(u.edgeOnly, 0.0, 1.0),
  );
  return vec2f(rim, displacementMask);
}

fn shadeAt(
  outputUv: vec2f,
  field: vec4f,
  rim: f32,
  displacementMask: f32,
) -> vec4f {
  let coreUv = outputToCoreUv(outputUv);
  let gradient = field.gb;
  let strength = clamp(u.strength, -1.0, 1.0);
  let magnitude = abs(strength);

  var sampleUv = coreUv;
  if (u.mode < 0.5) {
    let tangent = normalize(
      vec2f(-gradient.y, gradient.x) + vec2f(0.0001),
    );
    let opticalOffset =
      gradient * displacementMask + tangent * rim * 0.14;
    sampleUv = coreUv + opticalOffset * (0.075 * strength);
  } else if (u.mode < 1.5) {
    // Negative UV y moves the sampled content upward on screen.
    let lateralOffset =
      vec2f(1.0, -0.18) * (0.075 * strength * displacementMask);
    sampleUv = coreUv + lateralOffset;
  } else if (u.mode > 1.5 && u.mode < 2.5) {
    let aspect = u.coreWidth / u.coreHeight;
    let gradientUv = gradient * vec2f(aspect, 1.0);
    let normalUv = normalize(gradientUv + vec2f(0.0001));
    let bulgeOffset =
      normalUv * (0.065 * strength * displacementMask);
    sampleUv = coreUv + bulgeOffset;
  }

  let middle = sampleInput(sampleUv);
  var alpha = middle.a;
  var color = unpremultiply(middle);
  // Chromatic aberration is independent of the effect mode. At zero, avoid
  // sampling the same artwork coordinate two additional times. Separating
  // coverage as well as RGB keeps the effect visible on black transparent art.
  if (u.chromaticAberration > 0.000001) {
    let split = gradient * u.chromaticAberration * rim;
    let left = sampleInput(sampleUv - split);
    let right = sampleInput(sampleUv + split);
    let channelAlpha = vec3f(left.a, middle.a, right.a);
    alpha = max(channelAlpha.r, max(channelAlpha.g, channelAlpha.b));
    let channelPremultiplied = vec3f(left.r, middle.g, right.b);
    let aberratedPremultiplied =
      channelPremultiplied + vec3f(alpha) - channelAlpha;
    color = aberratedPremultiplied / max(alpha, 0.00001);
  }

  var highlight = 0.0;
  if (u.mode < 0.5 || u.shadow > 0.0) {
    let lightDirection = normalize(vec2f(-0.7, -1.0));
    highlight = pow(
      max(
        dot(
          normalize(gradient + vec2f(0.0001)),
          -lightDirection,
        ),
        0.0,
      ),
      7.0,
    );
  }
  if (u.mode < 0.5) {
    color = mix(
      color,
      vec3f(1.0),
      rim * highlight * 0.34 * magnitude,
    );
  } else if (u.mode > 2.5 && u.mode < 3.5) {
    let hueRotated = rotateHue(color, u.hueCos, u.hueSin);
    color = mix(color, hueRotated, displacementMask);
  } else if (u.mode > 3.5 && u.mode < 4.5) {
    color = mix(
      color,
      vec3f(1.0) - color,
      magnitude * displacementMask,
    );
  }

  // Tonal shading is independent of the effect mode. The post-composite pass
  // below handles cast shadows from the finished distorted alpha silhouette.
  let tonalShadow = clamp(
    rim * (1.0 - highlight) * u.shadow,
    0.0,
    1.0,
  );
  color = mix(color, vec3f(0.0), tonalShadow);
  color = clamp(color, vec3f(0.0), vec3f(1.0));

  // Figma's effect textures are premultiplied; displace alpha with the artwork
  // and convert the processed straight color back to premultiplied output.
  return vec4f(color * alpha, alpha);
}

fn sampleShaded(outputUvRaw: vec2f) -> vec4f {
  let outputUv = clamp(outputUvRaw, vec2f(0.0), vec2f(1.0));
  let field = textureSampleLevel(
    fieldTexture,
    linearSampler,
    outputUv,
    0.0,
  );
  let metrics = fieldMetrics(field.r);
  return shadeAt(outputUv, field, metrics.x, metrics.y);
}

@fragment
fn composite_fs(in: VertexOut) -> @location(0) vec4f {
  let uv = clamp(in.uv, vec2f(0.0), vec2f(1.0));
  let field = textureSampleLevel(fieldTexture, linearSampler, uv, 0.0);
  let metrics = fieldMetrics(field.r);
  let rim = metrics.x;
  let displacementMask = metrics.y;
  let distortionInfluence = max(displacementMask, rim);
  let magnitude = abs(clamp(u.strength, -1.0, 1.0));
  let spatialMode = u.mode < 2.5;
  let center = shadeAt(uv, field, rim, displacementMask);
  let centerLuma = dot(center.rgb, vec3f(0.2126, 0.7152, 0.0722));
  let edgeContrast = fwidth(centerLuma) + fwidth(center.a);
  let supersample = spatialMode &&
    magnitude > 0.2 &&
    distortionInfluence > 0.01 &&
    edgeContrast > 0.015;

  var result = center;
  if (supersample) {
    let pixel = 1.0 / vec2f(u.outputWidth, u.outputHeight);
    let uv0 = uv + pixel * vec2f(-0.4375,  0.0625);
    let uv1 = uv + pixel * vec2f(-0.3125, -0.3125);
    let uv2 = uv + pixel * vec2f(-0.0625, -0.1875);
    let uv3 = uv + pixel * vec2f( 0.1875, -0.4375);
    let uv4 = uv + pixel * vec2f( 0.3125, -0.0625);
    let uv5 = uv + pixel * vec2f( 0.4375,  0.3125);
    let uv6 = uv + pixel * vec2f( 0.0625,  0.4375);
    let uv7 = uv + pixel * vec2f(-0.1875,  0.1875);
    result = (
      sampleShaded(uv0) +
      sampleShaded(uv1) +
      sampleShaded(uv2) +
      sampleShaded(uv3) +
      sampleShaded(uv4) +
      sampleShaded(uv5) +
      sampleShaded(uv6) +
      sampleShaded(uv7)
    ) * 0.125;
  }

  let grain = (hashPixel(vec2u(in.position.xy)) - 0.5) * 0.018;
  let grainedRgb = clamp(
    result.rgb + grain * result.a,
    vec3f(0.0),
    vec3f(result.a),
  );
  return vec4f(grainedRgb, result.a);
}
`

  const shadowWGSL = uniformsWGSL + `
@group(0) @binding(0) var linearSampler: sampler;
@group(0) @binding(1) var processedTexture: texture_2d<f32>;
@group(0) @binding(2) var<uniform> u: Uniforms;

@fragment
fn shadow_fs(in: VertexOut) -> @location(0) vec4f {
  let uv = clamp(in.uv, vec2f(0.0), vec2f(1.0));
  let foreground = textureSampleLevel(
    processedTexture,
    linearSampler,
    uv,
    0.0,
  );
  let shadowStrength = clamp(u.shadow / 0.6, 0.0, 1.0);
  let shadowDirection = normalize(vec2f(0.72, 1.0));
  let shadowUv = uv - shadowDirection * (0.022 * shadowStrength);

  var shadowAlpha = 0.0;
  if (all(shadowUv >= vec2f(0.0)) && all(shadowUv <= vec2f(1.0))) {
    let shadowSource = textureSampleLevel(
      processedTexture,
      linearSampler,
      shadowUv,
      0.0,
    );
    shadowAlpha = shadowSource.a * shadowStrength * 0.85;
  }

  // Premultiplied foreground over a black premultiplied shadow.
  let outputAlpha = foreground.a + shadowAlpha * (1.0 - foreground.a);
  return vec4f(foreground.rgb, outputAlpha);
}
`

  frame.state.vertexModule = device.createShaderModule({ code: vertexWGSL })
  frame.state.fieldModule = device.createShaderModule({ code: fieldWGSL })
  frame.state.compositeModule = device.createShaderModule({ code: compositeWGSL })
  frame.state.shadowModule = device.createShaderModule({ code: shadowWGSL })

  frame.state.uniformBuffer = device.createBuffer({
    size: 80,
    usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
  })
  frame.state.uniformData = new Float32Array(20)
  frame.state.lastUniformData = new Float32Array(20)
  frame.state.lastUniformData.fill(Number.NaN)

  frame.state.linearSampler = device.createSampler({
    magFilter: "linear",
    minFilter: "linear",
    addressModeU: "clamp-to-edge",
    addressModeV: "clamp-to-edge",
  })

  frame.state.placeholder = device.createTexture({
    size: [1, 1, 1],
    format: "rgba8unorm",
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
  })
  device.queue.writeTexture(
    { texture: frame.state.placeholder },
    new Uint8Array([0, 0, 0, 0]),
    { bytesPerRow: 4 },
    { width: 1, height: 1, depthOrArrayLayers: 1 },
  )
  frame.state.placeholderView = frame.state.placeholder.createView()

  frame.state.fieldPipeline = device.createRenderPipeline({
    layout: "auto",
    vertex: {
      module: frame.state.vertexModule,
      entryPoint: "vs_main",
    },
    fragment: {
      module: frame.state.fieldModule,
      entryPoint: "field_fs",
      targets: [{ format: "rgba16float" }],
    },
    primitive: { topology: "triangle-list" },
  })

  frame.state.fieldBindGroup = device.createBindGroup({
    layout: frame.state.fieldPipeline.getBindGroupLayout(0),
    entries: [
      { binding: 0, resource: { buffer: frame.state.uniformBuffer } },
    ],
  })

  frame.state.fieldTexture = null
  frame.state.fieldView = null
  frame.state.fieldWidth = 0
  frame.state.fieldHeight = 0
  frame.state.fieldDirty = true
  frame.state.lastFieldPhase = Number.NaN
  frame.state.lastFieldScale = Number.NaN
  frame.state.lastFieldDetail = Number.NaN
  frame.state.lastFieldOutputPad = Number.NaN
  frame.state.lastFieldOutputWidth = Number.NaN
  frame.state.lastFieldOutputHeight = Number.NaN

  frame.state.compositePipeline = null
  frame.state.compositePipelineFormat = null
  frame.state.compositeBindGroup = null
  frame.state.inputTexture = frame.state.placeholder
  frame.state.inputView = frame.state.placeholderView

  frame.state.processedTexture = null
  frame.state.processedView = null
  frame.state.processedWidth = 0
  frame.state.processedHeight = 0
  frame.state.processedFormat = null
  frame.state.shadowPipeline = null
  frame.state.shadowPipelineFormat = null
  frame.state.shadowBindGroup = null

  frame.state.outputTexture = null
  frame.state.outputView = null
}

export function render(device, frame) {
  if (frame.output == null) {
    return
  }

  const params = frame.params || {}
  const width = Math.max(1, Math.round(finiteNumber(frame.output.width, 1)))
  const height = Math.max(1, Math.round(finiteNumber(frame.output.height, 1)))
  const outputPad = Math.max(0, finiteNumber(frame.outputPadding, 0))
  const inputPad = Math.max(0, finiteNumber(frame.inputPadding, 0))
  const coreWidth = Math.max(1, width - 2 * outputPad)
  const coreHeight = Math.max(1, height - 2 * outputPad)

  const speed = Math.max(0, Math.min(100, numberParam(params, "speed", 10)))
  const loopLength = 90 - (speed / 100) * 78
  const loopPhase = 0.45
  const timeSeconds = finiteNumber(frame.time, 0) * 0.001
  let phase = loopPhase
  if (speed > 0) {
    phase = timeSeconds / loopLength + loopPhase
    phase -= Math.floor(phase)
  }

  const modeName = stringParam(params, "mode", "Refraction")
  const mode = modeName === "Displacement" ? 1 :
    modeName === "Bulge / Pinch" ? 2 :
    modeName === "Hue Shift" ? 3 :
    modeName === "Invert" ? 4 : 0
  const regionName = stringParam(params, "region", "Whole Area")
  const edgeOnly = regionName === "Edges Only" ? 1 : 0
  const strength = Math.max(
    -1,
    Math.min(1, numberParam(params, "strength", 100) / 100),
  )
  const scaleControl = Math.max(
    30,
    Math.min(300, numberParam(params, "scale", 100)),
  )
  // Visual scale is inverse to frequency; 100% maps to frequency 3.33.
  const scale = (3.33 * 100) / scaleControl
  const detailControl = Math.max(
    0,
    Math.min(100, numberParam(params, "detail", 10)),
  )
  const detail = (detailControl / 100) * 0.45
  const softnessControl = Math.max(
    0,
    Math.min(100, numberParam(params, "softness", 0)),
  )
  const softness = 0.001 + (softnessControl / 100) * (0.12 - 0.001)
  const loopAngle = phase * Math.PI * 2
  const hueAngle = strength * Math.PI
  const thresholdControl = Math.max(
    0,
    Math.min(100, numberParam(params, "threshold", 60)),
  )
  const chromaticControl = Math.max(
    0,
    Math.min(100, numberParam(params, "chromaticAberration", 50)),
  )
  const shadowControl = Math.max(
    0,
    Math.min(100, numberParam(params, "shadow", 20)),
  )
  const shadow = (shadowControl / 100) * 0.6
  const shadowEnabled = shadowControl > 0.0001

  const uniforms = frame.state.uniformData
  uniforms[0] = scale
  uniforms[1] = 0.25 + (thresholdControl / 100) * 0.5
  uniforms[2] = softness
  uniforms[3] = 0
  uniforms[4] = strength
  uniforms[5] = (chromaticControl / 100) * 0.024
  uniforms[6] = detail
  uniforms[7] = edgeOnly
  uniforms[8] = width
  uniforms[9] = height
  uniforms[10] = shadow
  uniforms[11] = mode
  uniforms[12] = outputPad
  uniforms[13] = inputPad
  uniforms[14] = coreWidth
  uniforms[15] = coreHeight
  uniforms[16] = Math.cos(loopAngle)
  uniforms[17] = Math.sin(loopAngle)
  uniforms[18] = Math.cos(hueAngle)
  uniforms[19] = Math.sin(hueAngle)
  writeUniformsIfChanged(
    device,
    frame.state.uniformBuffer,
    uniforms,
    frame.state.lastUniformData,
  )

  const fieldWidth = Math.max(1, Math.ceil(width * 0.5))
  const fieldHeight = Math.max(1, Math.ceil(height * 0.5))
  if (
    frame.state.fieldTexture == null ||
    frame.state.fieldWidth !== fieldWidth ||
    frame.state.fieldHeight !== fieldHeight
  ) {
    if (frame.state.fieldTexture != null) {
      frame.state.fieldTexture.destroy()
    }
    frame.state.fieldTexture = device.createTexture({
      size: [fieldWidth, fieldHeight, 1],
      format: "rgba16float",
      usage:
        GPUTextureUsage.TEXTURE_BINDING |
        GPUTextureUsage.RENDER_ATTACHMENT,
    })
    frame.state.fieldView = frame.state.fieldTexture.createView()
    frame.state.fieldWidth = fieldWidth
    frame.state.fieldHeight = fieldHeight
    frame.state.fieldDirty = true
    frame.state.compositeBindGroup = null
  }

  const outputFormat = frame.output.format
  if (
    shadowEnabled &&
    (
      frame.state.processedTexture == null ||
      frame.state.processedWidth !== width ||
      frame.state.processedHeight !== height ||
      frame.state.processedFormat !== outputFormat
    )
  ) {
    if (frame.state.processedTexture != null) {
      frame.state.processedTexture.destroy()
    }
    frame.state.processedTexture = device.createTexture({
      size: [width, height, 1],
      format: outputFormat,
      usage:
        GPUTextureUsage.TEXTURE_BINDING |
        GPUTextureUsage.RENDER_ATTACHMENT,
    })
    frame.state.processedView = frame.state.processedTexture.createView()
    frame.state.processedWidth = width
    frame.state.processedHeight = height
    frame.state.processedFormat = outputFormat
    frame.state.shadowBindGroup = null
  }

  if (
    frame.state.compositePipeline == null ||
    frame.state.compositePipelineFormat !== outputFormat
  ) {
    frame.state.compositePipeline = device.createRenderPipeline({
      layout: "auto",
      vertex: {
        module: frame.state.vertexModule,
        entryPoint: "vs_main",
      },
      fragment: {
        module: frame.state.compositeModule,
        entryPoint: "composite_fs",
        targets: [{ format: outputFormat }],
      },
      primitive: { topology: "triangle-list" },
    })
    frame.state.compositePipelineFormat = outputFormat
    frame.state.compositeBindGroup = null
  }

  if (
    shadowEnabled &&
    (
      frame.state.shadowPipeline == null ||
      frame.state.shadowPipelineFormat !== outputFormat
    )
  ) {
    frame.state.shadowPipeline = device.createRenderPipeline({
      layout: "auto",
      vertex: {
        module: frame.state.vertexModule,
        entryPoint: "vs_main",
      },
      fragment: {
        module: frame.state.shadowModule,
        entryPoint: "shadow_fs",
        targets: [{ format: outputFormat }],
      },
      primitive: { topology: "triangle-list" },
    })
    frame.state.shadowPipelineFormat = outputFormat
    frame.state.shadowBindGroup = null
  }

  const inputTexture = frame.input != null
    ? frame.input
    : frame.state.placeholder
  if (frame.state.inputTexture !== inputTexture) {
    frame.state.inputTexture = inputTexture
    frame.state.inputView = inputTexture === frame.state.placeholder
      ? frame.state.placeholderView
      : inputTexture.createView()
    frame.state.compositeBindGroup = null
  }

  if (frame.state.outputTexture !== frame.output) {
    frame.state.outputTexture = frame.output
    frame.state.outputView = frame.output.createView()
  }

  if (frame.state.compositeBindGroup == null) {
    frame.state.compositeBindGroup = device.createBindGroup({
      layout: frame.state.compositePipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: frame.state.linearSampler },
        { binding: 1, resource: frame.state.inputView },
        { binding: 2, resource: frame.state.fieldView },
        { binding: 3, resource: { buffer: frame.state.uniformBuffer } },
      ],
    })
  }

  if (shadowEnabled && frame.state.shadowBindGroup == null) {
    frame.state.shadowBindGroup = device.createBindGroup({
      layout: frame.state.shadowPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: frame.state.linearSampler },
        { binding: 1, resource: frame.state.processedView },
        { binding: 2, resource: { buffer: frame.state.uniformBuffer } },
      ],
    })
  }

  const encoder = device.createCommandEncoder()

  const needsField = frame.state.fieldDirty ||
    phase !== frame.state.lastFieldPhase ||
    scale !== frame.state.lastFieldScale ||
    detail !== frame.state.lastFieldDetail ||
    outputPad !== frame.state.lastFieldOutputPad ||
    width !== frame.state.lastFieldOutputWidth ||
    height !== frame.state.lastFieldOutputHeight
  if (needsField) {
    const fieldPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: frame.state.fieldView,
        loadOp: "clear",
        clearValue: { r: 0.5, g: 0, b: 0, a: 1 },
        storeOp: "store",
      }],
    })
    fieldPass.setPipeline(frame.state.fieldPipeline)
    fieldPass.setBindGroup(0, frame.state.fieldBindGroup)
    fieldPass.draw(3)
    fieldPass.end()
    frame.state.fieldDirty = false
    frame.state.lastFieldPhase = phase
    frame.state.lastFieldScale = scale
    frame.state.lastFieldDetail = detail
    frame.state.lastFieldOutputPad = outputPad
    frame.state.lastFieldOutputWidth = width
    frame.state.lastFieldOutputHeight = height
  }

  const compositePass = encoder.beginRenderPass({
    colorAttachments: [{
      view: shadowEnabled
        ? frame.state.processedView
        : frame.state.outputView,
      loadOp: "clear",
      clearValue: { r: 0, g: 0, b: 0, a: 0 },
      storeOp: "store",
    }],
  })
  compositePass.setPipeline(frame.state.compositePipeline)
  compositePass.setBindGroup(0, frame.state.compositeBindGroup)
  compositePass.draw(3)
  compositePass.end()

  if (shadowEnabled) {
    const shadowPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: frame.state.outputView,
        loadOp: "clear",
        clearValue: { r: 0, g: 0, b: 0, a: 0 },
        storeOp: "store",
      }],
    })
    shadowPass.setPipeline(frame.state.shadowPipeline)
    shadowPass.setBindGroup(0, frame.state.shadowBindGroup)
    shadowPass.draw(3)
    shadowPass.end()
  }

  device.queue.submit([encoder.finish()])
}

defineProperties(Effect, {
  "mode": {
    type: "string",
    label: "Mode",
    defaultValue: "Refraction",
    control: "select",
    options: [
      { value: "Refraction", label: "Refraction" },
      { value: "Displacement", label: "Displacement" },
      { value: "Bulge / Pinch", label: "Bulge / pinch" },
      { value: "Hue Shift", label: "Hue shift" },
      { value: "Invert", label: "Invert" },
    ],
  },
  "region": {
    type: "string",
    label: "Region",
    defaultValue: "Whole Area",
    control: "select",
    options: [
      { value: "Whole Area", label: "Whole area" },
      { value: "Edges Only", label: "Edges only" },
    ],
  },
  "strength": {
    type: "number",
    label: "Strength",
    defaultValue: 100,
    control: "slider",
    unitLabel: "%",
    min: -100,
    max: 100,
    step: 1,
  },
  "scale": {
    type: "number",
    label: "Scale",
    defaultValue: 100,
    control: "slider",
    unitLabel: "%",
    min: 30,
    max: 300,
    step: 1,
  },
  "detail": {
    type: "number",
    label: "Detail",
    defaultValue: 10,
    control: "slider",
    unitLabel: "%",
    min: 0,
    max: 100,
    step: 1,
  },
  "threshold": {
    type: "number",
    label: "Threshold",
    defaultValue: 60,
    control: "slider",
    unitLabel: "%",
    min: 0,
    max: 100,
    step: 0.1,
  },
  "softness": {
    type: "number",
    label: "Softness",
    defaultValue: 0,
    control: "slider",
    unitLabel: "%",
    min: 0,
    max: 100,
    step: 1,
  },
  "shadow": {
    type: "number",
    label: "Shadow",
    defaultValue: 20,
    control: "slider",
    unitLabel: "%",
    min: 0,
    max: 100,
    step: 1,
  },
  "chromaticAberration": {
    type: "number",
    label: "Dispersion",
    defaultValue: 50,
    control: "slider",
    unitLabel: "%",
    min: 0,
    max: 100,
    step: 0.1,
  },
  "speed": {
    type: "number",
    label: "Speed",
    defaultValue: 10,
    control: "slider",
    unitLabel: "%",
    min: 0,
    max: 100,
    step: 0.1,
  },
})
