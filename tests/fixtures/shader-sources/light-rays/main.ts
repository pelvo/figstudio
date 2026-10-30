import { defineProperties } from "figma:shaders"

export default function Effect() {}

function createPipeline(device, module, fragmentEntryPoint, format) {
  return device.createRenderPipeline({
    layout: "auto",
    vertex: {
      module,
      entryPoint: "vs_main",
      buffers: [{
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
      }],
    },
    fragment: {
      module,
      entryPoint: fragmentEntryPoint,
      targets: [{ format }],
    },
    primitive: {
      topology: "triangle-list",
    },
  })
}

export function setup(device, frame) {
  frame.state.shaderModule = device.createShaderModule({
    code: `
struct Uniforms {
  viewport: vec4f,
  lightSource: vec4f,
  rayControls: vec4f, // x intensity, y spread, z color delta, w edge glow
  color1: vec4f,
  color2: vec4f,
  inputControls: vec4f, // x input mode, y threshold, z density, w softness
  blurControls: vec4f, // x/y near/far sigma, z/w padding
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var texSampler: sampler;
@group(0) @binding(2) var inputTex: texture_2d<f32>;
@group(0) @binding(3) var blurTex: texture_2d<f32>;
@group(0) @binding(4) var nearBlurTex: texture_2d<f32>;
@group(0) @binding(5) var farBlurTex: texture_2d<f32>;

struct VsIn {
  @location(0) pos: vec2f,
  @location(1) uv: vec2f,
}

struct VsOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
}

@vertex fn vs_main(in: VsIn) -> VsOut {
  var out: VsOut;
  out.position = vec4f(in.pos, 0.0, 1.0);
  out.uv = in.uv;
  return out;
}

fn thresholdSignal(signal: f32) -> f32 {
  let threshold = clamp(u.inputControls.y, 0.0, 1.0);
  let halfWidth = max(min(threshold, 1.0 - threshold), 0.00001);
  return smoothstep(
    threshold - halfWidth,
    threshold + halfWidth,
    signal
  );
}

fn selectedSignal(sampledColor: vec4f) -> f32 {
  if (u.inputControls.x >= 0.5) {
    return sampledColor.a;
  }
  return dot(sampledColor.rgb, vec3f(0.2126, 0.7152, 0.0722));
}

fn sourceMaskAt(sampleUv: vec2f) -> f32 {
  if (
    sampleUv.x < 0.0 || sampleUv.x > 1.0 ||
    sampleUv.y < 0.0 || sampleUv.y > 1.0
  ) {
    return 0.0;
  }
  let sampledColor = textureSampleLevel(inputTex, texSampler, sampleUv, 0.0);
  return thresholdSignal(selectedSignal(sampledColor));
}

fn blurInputAt(sampleUv: vec2f) -> f32 {
  return textureSampleLevel(blurTex, texSampler, sampleUv, 0.0).r;
}

fn gaussianWeight(offset: f32, sigma: f32) -> f32 {
  let normalized = offset / max(sigma, 0.35);
  return exp(-0.5 * normalized * normalized);
}

fn denseBlurHorizontal(uv: vec2f, sigma: f32) -> f32 {
  let support = clamp(i32(ceil(sigma * 2.7)), 1, 16);
  let blurDims = textureDimensions(blurTex);
  let texelStep = vec2f(1.0 / max(f32(blurDims.x), 1.0), 0.0);
  var sum = blurInputAt(uv);
  var weightTotal = 1.0;
  for (var pair: i32 = 0; pair < 8; pair = pair + 1) {
    let offset1 = 1 + pair * 2;
    if (offset1 <= support) {
      let offset2 = offset1 + 1;
      let weight1 = gaussianWeight(f32(offset1), sigma);
      let weight2 = select(
        0.0,
        gaussianWeight(f32(offset2), sigma),
        offset2 <= support
      );
      let pairedWeight = weight1 + weight2;
      let pairedOffset =
        (f32(offset1) * weight1 + f32(offset2) * weight2) /
        pairedWeight;
      sum = sum + (
        blurInputAt(uv + texelStep * pairedOffset) +
        blurInputAt(uv - texelStep * pairedOffset)
      ) * pairedWeight;
      weightTotal = weightTotal + 2.0 * pairedWeight;
    }
  }
  return sum / weightTotal;
}

fn denseBlurVertical(uv: vec2f, sigma: f32) -> f32 {
  let support = clamp(i32(ceil(sigma * 2.7)), 1, 16);
  let blurDims = textureDimensions(blurTex);
  let texelStep = vec2f(0.0, 1.0 / max(f32(blurDims.y), 1.0));
  var sum = blurInputAt(uv);
  var weightTotal = 1.0;
  for (var pair: i32 = 0; pair < 8; pair = pair + 1) {
    let offset1 = 1 + pair * 2;
    if (offset1 <= support) {
      let offset2 = offset1 + 1;
      let weight1 = gaussianWeight(f32(offset1), sigma);
      let weight2 = select(
        0.0,
        gaussianWeight(f32(offset2), sigma),
        offset2 <= support
      );
      let pairedWeight = weight1 + weight2;
      let pairedOffset =
        (f32(offset1) * weight1 + f32(offset2) * weight2) /
        pairedWeight;
      sum = sum + (
        blurInputAt(uv + texelStep * pairedOffset) +
        blurInputAt(uv - texelStep * pairedOffset)
      ) * pairedWeight;
      weightTotal = weightTotal + 2.0 * pairedWeight;
    }
  }
  return sum / weightTotal;
}

@fragment fn mask_main(in: VsOut) -> @location(0) f32 {
  return sourceMaskAt(in.uv);
}

// Four bilinear reads implement a separable [1, 3, 3, 1] / 8 filter in
// each axis before every 2x reduction. This prevents high-frequency mask
// detail from folding into the lower-resolution blur levels.
@fragment fn downsample_main(in: VsOut) -> @location(0) f32 {
  let sourceSize = textureDimensions(blurTex);
  let sourceDims = vec2f(f32(sourceSize.x), f32(sourceSize.y));
  let offset = vec2f(0.75) / max(sourceDims, vec2f(1.0));
  return 0.25 * (
    blurInputAt(in.uv + vec2f(-offset.x, -offset.y)) +
    blurInputAt(in.uv + vec2f( offset.x, -offset.y)) +
    blurInputAt(in.uv + vec2f(-offset.x,  offset.y)) +
    blurInputAt(in.uv + vec2f( offset.x,  offset.y))
  );
}

@fragment fn near_h_main(in: VsOut) -> @location(0) f32 {
  return denseBlurHorizontal(in.uv, u.blurControls.x);
}

@fragment fn near_v_main(in: VsOut) -> @location(0) f32 {
  return denseBlurVertical(in.uv, u.blurControls.x);
}

@fragment fn far_h_main(in: VsOut) -> @location(0) f32 {
  return denseBlurHorizontal(in.uv, u.blurControls.y);
}

@fragment fn far_v_main(in: VsOut) -> @location(0) f32 {
  return denseBlurVertical(in.uv, u.blurControls.y);
}

fn rayStrength(
  sourceToPoint: vec2f,
  rayRefDirection: vec2f,
  reach: f32,
  bandBase: f32,
  bandSwing: f32,
  seedA: f32,
  seedB: f32,
  speed: f32,
  time: f32
) -> f32 {
  let sourceDistance = length(sourceToPoint);
  let sourceDirection = select(
    rayRefDirection,
    sourceToPoint / max(sourceDistance, 0.0001),
    sourceDistance > 0.0001
  );
  let cosAngle = dot(sourceDirection, rayRefDirection);
  let wave = (
    0.15 * sin(cosAngle * seedA + time * speed) +
    0.20 * cos(-cosAngle * seedB + time * speed)
  ) / 0.35;
  let bandStrength = clamp(bandBase + bandSwing * wave, 0.0, 1.0);
  let distanceModulation = clamp(
    (reach - sourceDistance) / reach,
    0.5,
    1.0
  );
  let reachFade = 1.0 - smoothstep(reach * 0.35, reach, sourceDistance);
  let pulse = 1.0 - 0.16 * sin(time * speed * 0.55 + seedA * 0.1) - 0.07 * sin(time * speed * 1.27 + seedB * 0.1);
  return bandStrength * distanceModulation * reachFade * pulse;
}

@fragment fn fs_main(in: VsOut) -> @location(0) vec4f {
  let dims = max(u.viewport.xy, vec2f(1.0));
  let shortestSide =
    max(min(dims.x, dims.y), 1.0);
  let uvToEqual = dims / shortestSide;

  let uv = in.uv;
  let point = uv * uvToEqual;
  let lightUv = u.lightSource.xy;
  let lightPoint = lightUv * uvToEqual;

  let sourceToPoint = point - lightPoint;
  let distanceFromLight = length(sourceToPoint);

  let normalizedIntensity =
    clamp(u.rayControls.x, 0.0, 1.0);
  let normalizedSpread =
    clamp(u.rayControls.y, 0.0, 1.0);
  let colorDelta = clamp(u.rayControls.z, -1.0, 1.0);
  let colorBlend = colorDelta * 0.5 + 0.5;
  let normalizedGlow = clamp(u.rayControls.w, 0.0, 1.0);
  let rayDetail = clamp(u.inputControls.z, 0.0, 1.0);
  let detailScale = pow(2.0, mix(-2.4, 2.4, rayDetail));
  let falloff = 1.7;

  let animationSpeed =
    mix(
      0.2,
      4.0,
      clamp(u.viewport.w, 0.0, 1.0)
    );
  let animatedTime = u.viewport.z;

  let PI = 3.14159265359;
  let lightAngle = radians(u.lightSource.z);
  let rayDirection = vec2f(cos(lightAngle), sin(lightAngle));
  let halfArc = mix(0.16, PI * 1.7, normalizedSpread);
  let pointAngle = atan2(sourceToPoint.y, sourceToPoint.x);
  let angleFromAxis = atan2(
    sin(pointAngle - lightAngle),
    cos(pointAngle - lightAngle)
  );
  let rangeMask = 1.0 - smoothstep(
    halfArc * 0.6,
    halfArc,
    abs(angleFromAxis)
  );
  let reach = max(u.lightSource.w, 0.005);
  let softnessInput = 1.0 - clamp(u.inputControls.w, 0.0, 1.0);
  let softnessContrast = softnessInput * softnessInput * softnessInput * (softnessInput * (softnessInput * 6.0 - 15.0) + 10.0);
  let bandBase = mix(1.10, 0.40, softnessContrast);
  let bandSwing = mix(0.05, 0.65, softnessContrast);
  let band1 = rayStrength(sourceToPoint, rayDirection, reach, bandBase, bandSwing, 36.2214 * detailScale, 21.11349 * detailScale, animationSpeed, animatedTime) * rangeMask;
  let band2 = rayStrength(sourceToPoint, rayDirection, reach, bandBase, bandSwing, 22.3991 * detailScale, 18.0234 * detailScale, animationSpeed * 0.2, animatedTime) * rangeMask;
  let normalizedDistance = distanceFromLight / reach;
  let mappedIntensity =
    mix(0.5, 4.0, normalizedIntensity);
  let distanceBrightness =
    mappedIntensity *
    0.4 /
    pow(max(normalizedDistance, 0.001), falloff);

  let tint1 = u.color1.rgb * u.color1.a;
  let tint2 = u.color2.rgb * u.color2.a;
  let rays1 = tint1 * band1 * (1.0 - colorBlend) * 0.9;
  let rays2 = tint2 * band2 * colorBlend * 0.9;
  let rayColor = (rays1 + rays2) * distanceBrightness;

  // Content
  let content =
    textureSample(inputTex, texSampler, uv);
  let alpha = content.a;
  let sourceMask = thresholdSignal(selectedSignal(content));
  let outside = 1.0 - sourceMask;

  // Glow: localLight from max channel (up to 3.0)
  let localLight =
    clamp(
      max(rayColor.r, max(rayColor.g, rayColor.b)),
      0.0,
      3.0
    );
  let rimColor =
    mix(tint1, tint2, colorBlend);
  let glowAmount =
    mix(0.0, 12.0, pow(normalizedGlow, 0.8));
  var glow = vec3f(0.0);
  if (glowAmount > 0.0001 && localLight > 0.001 && sourceMask < 0.999) {
    let blurNear =
      textureSampleLevel(nearBlurTex, texSampler, uv, 0.0).r;
    let blurFar =
      textureSampleLevel(farBlurTex, texSampler, uv, 0.0).r;
    let rim = blurNear * outside;
    let halo = blurFar * outside;
    let glowMask =
      clamp(rim * 0.8 + halo * 0.5, 0.0, 1.0);
    glow = rimColor * glowMask * localLight * glowAmount;
  }

  // The input is premultiplied. Put rays beneath it without multiplying
  // antialiased content RGB by alpha a second time, then add the halo.
  let visibleRayColor = rayColor * outside;
  let lightColor = visibleRayColor + glow;
  let finalColor = content.rgb + lightColor;

  // Alpha: opaque where content or light exists
  let lightLum =
    dot(
      clamp(lightColor, vec3f(0.0), vec3f(1.0)),
      vec3f(0.2126, 0.7152, 0.0722)
    );
  let finalAlpha =
    clamp(alpha + lightLum * (1.0 - alpha), 0.0, 1.0);

  return vec4f(finalColor, finalAlpha);
}
`,
  })

  frame.state.maskPipeline = createPipeline(
    device,
    frame.state.shaderModule,
    "mask_main",
    "r16float",
  )
  frame.state.downsamplePipeline = createPipeline(
    device,
    frame.state.shaderModule,
    "downsample_main",
    "r16float",
  )
  frame.state.nearHorizontalPipeline = createPipeline(
    device,
    frame.state.shaderModule,
    "near_h_main",
    "r16float",
  )
  frame.state.nearVerticalPipeline = createPipeline(
    device,
    frame.state.shaderModule,
    "near_v_main",
    "r16float",
  )
  frame.state.farHorizontalPipeline = createPipeline(
    device,
    frame.state.shaderModule,
    "far_h_main",
    "r16float",
  )
  frame.state.farVerticalPipeline = createPipeline(
    device,
    frame.state.shaderModule,
    "far_v_main",
    "r16float",
  )

  frame.state.quad = device.createBuffer({
    size: 6 * 4 * 4,
    usage: GPUBufferUsage.VERTEX,
    mappedAtCreation: true,
  })
  new Float32Array(
    frame.state.quad.getMappedRange(),
  ).set([
    -1, -1, 0, 1,
     1, -1, 1, 1,
    -1,  1, 0, 0,
    -1,  1, 0, 0,
     1, -1, 1, 1,
     1,  1, 1, 0,
  ])
  frame.state.quad.unmap()

  frame.state.uniformBuf = device.createBuffer({
    size: 112,
    usage:
      GPUBufferUsage.UNIFORM |
      GPUBufferUsage.COPY_DST,
  })
  frame.state.uniformData = new Float32Array(28)

  frame.state.sampler = device.createSampler({
    addressModeU: "clamp-to-edge",
    addressModeV: "clamp-to-edge",
    magFilter: "linear",
    minFilter: "linear",
  })
}

export function render(device, frame) {
  if (frame.input == null) {
    return
  }

  var s = frame.state

  if (s.pipelineFormat !== frame.output.format) {
    s.pipeline = createPipeline(
      device,
      s.shaderModule,
      "fs_main",
      frame.output.format,
    )
    s.pipelineFormat = frame.output.format
  }

  var p = frame.params
  var axis = p.lightSource
  var lightAngle = axis.angle
  var REACH_SCALE = 2.0
  var shortestSide = Math.max(Math.min(frame.output.width, frame.output.height), 1)
  var reach = (axis.radius / 100) * (frame.output.height / shortestSide) * REACH_SCALE
  var normalizedGlow = Math.max(0, Math.min(p.glow, 1))
  var glowRadius = 0.003 + (0.03 - 0.003) * Math.pow(normalizedGlow, 1.6)
  var nearRadiusPx = Math.max(glowRadius * 0.4, 0.001) * shortestSide
  var farRadiusPx = Math.max(glowRadius * 1.4, 0.001) * shortestSide
  var sigmaForRadius = function(radiusPx) {
    var rings = Math.min(Math.max(1 + Math.trunc(radiusPx / 10), 2), 4)
    var taps = Math.min(Math.max(Math.ceil(radiusPx * 1.1), 8), 20)
    var momentTotal = 0
    var ditherSamples = 9
    for (var sampleIndex = 0; sampleIndex < ditherSamples; sampleIndex++) {
      var delta = -0.475 + 0.95 * sampleIndex / (ditherSamples - 1)
      var weightSum = 0
      var weightedRadiusSquared = 0
      for (var ring = 0; ring < rings; ring++) {
        var ringIndex = ring + 1 + delta
        var weightPosition = Math.max(
          0,
          Math.min((ringIndex - 1) / Math.max(rings - 1, 1), 1),
        )
        var ringWeight = 0.7 + (0.2 - 0.7) * weightPosition
        var ringRadiusPx = radiusPx * ringIndex / rings
        weightSum += ringWeight
        weightedRadiusSquared += ringWeight * ringRadiusPx * ringRadiusPx
      }
      momentTotal +=
        taps * weightedRadiusSquared /
        (1 + taps * weightSum)
    }
    return Math.sqrt(momentTotal / ditherSamples * 0.5)
  }
  var nearScale = 1
  var farScale = 1
  while (nearRadiusPx / nearScale > 12) {
    nearScale *= 2
  }
  while (farRadiusPx / farScale > 12) {
    farScale *= 2
  }
  var workingSigma = function(targetSigma, scale) {
    // Each [1, 3, 3, 1] / 8 downsample level contributes 0.75 pixels^2
    // at its own scale, or 0.25 * (scale^2 - 1) in full-resolution units.
    var downsampleVariance = 0.25 * (scale * scale - 1)
    var minimumVariance = 0.35 * 0.35 * scale * scale
    var remainingVariance = Math.max(
      targetSigma * targetSigma - downsampleVariance,
      minimumVariance,
    )
    // A Gaussian truncated at 2.7 sigma retains 94.3334% of its variance.
    return Math.sqrt(remainingVariance) / scale * 1.0296
  }
  var nearSigma = workingSigma(sigmaForRadius(nearRadiusPx), nearScale)
  var farSigma = workingSigma(sigmaForRadius(farRadiusPx), farScale)
  var c1 = p.color1
  var c2 = p.color2
  var d = s.uniformData
  d[0] = frame.output.width
  d[1] = frame.output.height
  d[2] = frame.time * 0.001
  d[3] = p.speed
  d[4] = axis.x / 100
  d[5] = axis.y / 100
  d[6] = lightAngle
  d[7] = reach
  d[8] = p.intensity
  d[9] = p.spread
  d[10] = p.colorBlend
  d[11] = p.glow
  d[12] = c1.r
  d[13] = c1.g
  d[14] = c1.b
  d[15] = c1.a
  d[16] = c2.r
  d[17] = c2.g
  d[18] = c2.b
  d[19] = c2.a
  d[20] = p.inputMode
  d[21] = p.threshold
  d[22] = p.density
  d[23] = p.beamSoftness
  d[24] = nearSigma
  d[25] = farSigma
  d[26] = 0
  d[27] = 0
  device.queue.writeBuffer(
    s.uniformBuf,
    0,
    s.uniformData,
  )

  var nearLevel = Math.round(Math.log2(nearScale))
  var farLevel = Math.round(Math.log2(farScale))
  var maskLevels = Math.max(nearLevel, farLevel) + 1
  var nearWidth = Math.max(1, Math.floor(frame.output.width / nearScale))
  var nearHeight = Math.max(1, Math.floor(frame.output.height / nearScale))
  var farWidth = Math.max(1, Math.floor(frame.output.width / farScale))
  var farHeight = Math.max(1, Math.floor(frame.output.height / farScale))
  var textureLayoutChanged =
    s.blurWidth !== frame.output.width ||
    s.blurHeight !== frame.output.height ||
    s.nearWidth !== nearWidth ||
    s.nearHeight !== nearHeight ||
    s.farWidth !== farWidth ||
    s.farHeight !== farHeight ||
    s.maskLevels !== maskLevels
  if (textureLayoutChanged) {
    if (s.maskTexture) {
      s.maskTexture.destroy()
      s.nearHorizontal.destroy()
      s.nearVertical.destroy()
      s.farHorizontal.destroy()
      s.farVertical.destroy()
    }
    var blurUsage =
      GPUTextureUsage.RENDER_ATTACHMENT |
      GPUTextureUsage.TEXTURE_BINDING
    s.maskTexture = device.createTexture({
      size: {
        width: frame.output.width,
        height: frame.output.height,
      },
      mipLevelCount: maskLevels,
      format: "r16float",
      usage: blurUsage,
    })
    s.nearHorizontal = device.createTexture({
      size: { width: nearWidth, height: nearHeight },
      format: "r16float",
      usage: blurUsage,
    })
    s.nearVertical = device.createTexture({
      size: { width: nearWidth, height: nearHeight },
      format: "r16float",
      usage: blurUsage,
    })
    s.farHorizontal = device.createTexture({
      size: { width: farWidth, height: farHeight },
      format: "r16float",
      usage: blurUsage,
    })
    s.farVertical = device.createTexture({
      size: { width: farWidth, height: farHeight },
      format: "r16float",
      usage: blurUsage,
    })
    s.maskLevelViews = []
    for (var level = 0; level < maskLevels; level++) {
      s.maskLevelViews.push(s.maskTexture.createView({
        baseMipLevel: level,
        mipLevelCount: 1,
      }))
    }
    s.maskView = s.maskLevelViews[0]
    s.nearHorizontalView = s.nearHorizontal.createView()
    s.nearVerticalView = s.nearVertical.createView()
    s.farHorizontalView = s.farHorizontal.createView()
    s.farVerticalView = s.farVertical.createView()
    s.blurWidth = frame.output.width
    s.blurHeight = frame.output.height
    s.nearWidth = nearWidth
    s.nearHeight = nearHeight
    s.farWidth = farWidth
    s.farHeight = farHeight
    s.maskLevels = maskLevels
    s.maskBindGroup = null
  }

  if (
    !s.maskBindGroup ||
    s.bindGroupPipeline !== s.pipeline ||
    s.bindGroupInput !== frame.input
  ) {
    var inputView = frame.input.createView()
    s.maskBindGroup = device.createBindGroup({
      layout: s.maskPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: s.uniformBuf } },
        { binding: 1, resource: s.sampler },
        { binding: 2, resource: inputView },
      ],
    })
    s.downsampleBindGroups = []
    for (var mipLevel = 1; mipLevel < maskLevels; mipLevel++) {
      s.downsampleBindGroups.push(device.createBindGroup({
        layout: s.downsamplePipeline.getBindGroupLayout(0),
        entries: [
          { binding: 1, resource: s.sampler },
          { binding: 3, resource: s.maskLevelViews[mipLevel - 1] },
        ],
      }))
    }
    s.nearHorizontalBindGroup = device.createBindGroup({
      layout: s.nearHorizontalPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: s.uniformBuf } },
        { binding: 1, resource: s.sampler },
        { binding: 3, resource: s.maskLevelViews[nearLevel] },
      ],
    })
    s.nearVerticalBindGroup = device.createBindGroup({
      layout: s.nearVerticalPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: s.uniformBuf } },
        { binding: 1, resource: s.sampler },
        { binding: 3, resource: s.nearHorizontalView },
      ],
    })
    s.farHorizontalBindGroup = device.createBindGroup({
      layout: s.farHorizontalPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: s.uniformBuf } },
        { binding: 1, resource: s.sampler },
        { binding: 3, resource: s.maskLevelViews[farLevel] },
      ],
    })
    s.farVerticalBindGroup = device.createBindGroup({
      layout: s.farVerticalPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: s.uniformBuf } },
        { binding: 1, resource: s.sampler },
        { binding: 3, resource: s.farHorizontalView },
      ],
    })
    s.bindGroup = device.createBindGroup({
      layout: s.pipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: s.uniformBuf } },
        { binding: 1, resource: s.sampler },
        { binding: 2, resource: inputView },
        { binding: 4, resource: s.nearVerticalView },
        { binding: 5, resource: s.farVerticalView },
      ],
    })
    s.bindGroupPipeline = s.pipeline
    s.bindGroupInput = frame.input
  }

  var encoder = device.createCommandEncoder()
  if (p.glow > 0) {
    var maskPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: s.maskView,
        loadOp: "clear",
        clearValue: { r: 0, g: 0, b: 0, a: 0 },
        storeOp: "store",
      }],
    })
    maskPass.setPipeline(s.maskPipeline)
    maskPass.setBindGroup(0, s.maskBindGroup)
    maskPass.setVertexBuffer(0, s.quad)
    maskPass.draw(6)
    maskPass.end()

    for (
      var downsampleLevel = 1;
      downsampleLevel < maskLevels;
      downsampleLevel++
    ) {
      var downsamplePass = encoder.beginRenderPass({
        colorAttachments: [{
          view: s.maskLevelViews[downsampleLevel],
          loadOp: "clear",
          clearValue: { r: 0, g: 0, b: 0, a: 0 },
          storeOp: "store",
        }],
      })
      downsamplePass.setPipeline(s.downsamplePipeline)
      downsamplePass.setBindGroup(
        0,
        s.downsampleBindGroups[downsampleLevel - 1],
      )
      downsamplePass.setVertexBuffer(0, s.quad)
      downsamplePass.draw(6)
      downsamplePass.end()
    }

    var nearHorizontalPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: s.nearHorizontalView,
        loadOp: "clear",
        clearValue: { r: 0, g: 0, b: 0, a: 0 },
        storeOp: "store",
      }],
    })
    nearHorizontalPass.setPipeline(s.nearHorizontalPipeline)
    nearHorizontalPass.setBindGroup(0, s.nearHorizontalBindGroup)
    nearHorizontalPass.setVertexBuffer(0, s.quad)
    nearHorizontalPass.draw(6)
    nearHorizontalPass.end()

    var nearVerticalPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: s.nearVerticalView,
        loadOp: "clear",
        clearValue: { r: 0, g: 0, b: 0, a: 0 },
        storeOp: "store",
      }],
    })
    nearVerticalPass.setPipeline(s.nearVerticalPipeline)
    nearVerticalPass.setBindGroup(0, s.nearVerticalBindGroup)
    nearVerticalPass.setVertexBuffer(0, s.quad)
    nearVerticalPass.draw(6)
    nearVerticalPass.end()

    var farHorizontalPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: s.farHorizontalView,
        loadOp: "clear",
        clearValue: { r: 0, g: 0, b: 0, a: 0 },
        storeOp: "store",
      }],
    })
    farHorizontalPass.setPipeline(s.farHorizontalPipeline)
    farHorizontalPass.setBindGroup(0, s.farHorizontalBindGroup)
    farHorizontalPass.setVertexBuffer(0, s.quad)
    farHorizontalPass.draw(6)
    farHorizontalPass.end()

    var farVerticalPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: s.farVerticalView,
        loadOp: "clear",
        clearValue: { r: 0, g: 0, b: 0, a: 0 },
        storeOp: "store",
      }],
    })
    farVerticalPass.setPipeline(s.farVerticalPipeline)
    farVerticalPass.setBindGroup(0, s.farVerticalBindGroup)
    farVerticalPass.setVertexBuffer(0, s.quad)
    farVerticalPass.draw(6)
    farVerticalPass.end()
  }

  var pass = encoder.beginRenderPass({
    colorAttachments: [{
      view: frame.output.createView(),
      loadOp: "clear",
      clearValue: {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
      },
      storeOp: "store",
    }],
  })
  pass.setPipeline(s.pipeline)
  pass.setBindGroup(0, s.bindGroup)
  pass.setVertexBuffer(0, s.quad)
  pass.draw(6)
  pass.end()
  device.queue.submit([encoder.finish()])
}

defineProperties(Effect, {
  inputMode: {
    type: "number",
    label: "Input mode",
    defaultValue: 0,
    control: "select",
    options: [
      { value: 0, label: "Luma" },
      { value: 1, label: "Alpha" },
    ],
  },
  threshold: {
    type: "number",
    label: "Threshold",
    defaultValue: 0.5,
    control: "slider",
    min: 0,
    max: 1,
    step: 0.01,
  },
  density: {
    type: "number",
    label: "Density",
    defaultValue: 0.5,
    control: "slider",
    min: 0,
    max: 1,
    step: 0.01,
  },
  intensity: {
    type: "number",
    label: "Intensity",
    defaultValue: 0.2,
    control: "slider",
    min: 0,
    max: 1,
    step: 0.01,
  },
  beamSoftness: {
    type: "number",
    label: "Softness",
    defaultValue: 0.5,
    control: "slider",
    min: 0,
    max: 1,
    step: 0.01,
  },
  spread: {
    type: "number",
    label: "Spread",
    defaultValue: 0.5,
    control: "slider",
    min: 0,
    max: 1,
    step: 0.01,
  },
  speed: {
    type: "number",
    label: "Speed",
    defaultValue: 0.5,
    control: "slider",
    min: 0,
    max: 1,
    step: 0.01,
  },
  color1: {
    type: "color",
    label: "Color 1",
    defaultValue: {
      r: 0.10196078431372549,
      g: 0.7372549019607844,
      b: 0.996078431372549,
      a: 1,
    },
  },
  color2: {
    type: "color",
    label: "Color 2",
    defaultValue: {
      r: 0.6352941176470588,
      g: 0.34901960784313724,
      b: 1,
      a: 1,
    },
  },
  colorBlend: {
    type: "number",
    label: "Color blend",
    defaultValue: 0,
    control: "slider",
    min: -1,
    max: 1,
    step: 0.01,
  },
  glow: {
    type: "number",
    label: "Edge glow",
    defaultValue: 0.5,
    control: "slider",
    min: 0,
    max: 1,
    step: 0.01,
  },
  lightSource: {
    type: "point-angle-radius",
    label: "Light source",
    defaultValue: {
      x: 50,
      y: -30,
      angle: 90,
      radius: 85,
    },
    mode: "canvas_and_ui",
    positionUnit: "%",
    radiusUnit: "%",
  },
})
