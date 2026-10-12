//! Tiles, recordings, comparisons and synthetic documents shared by the GPU test suites.
use figstudio_fig::{Document, Guid, Val};
use figstudio_render::kurbo::{Affine, Rect};
use figstudio_render::{GpuUnsupported, Recording, Renderer, Viewport};

use super::{Gpu, SIDE};
use crate::common::*;

pub const TILE: f64 = 512.0;

pub fn renderer() -> Renderer {
    let mut r = Renderer::new();
    r.system_fonts = false;
    r
}

pub fn tile(scale: f64, x: i32, y: i32) -> Viewport {
    Viewport {
        width: SIDE,
        height: SIDE,
        transform: Affine::translate((2.0 - f64::from(x) * TILE, 2.0 - f64::from(y) * TILE))
            * Affine::scale(scale),
        background: Some(figstudio_render::color::AlphaColor::new([1.0; 4])),
    }
}

pub fn record(doc: &Document, page: Guid, scale: f64) -> Recording {
    let e = 64.0 * TILE;
    let area = Rect::new(-e, -e, e, e);
    let (rec, _) = renderer().record(doc, page, Affine::scale(scale), area, usize::MAX);
    rec.expect("no limit")
}

/// Channel differences between the GPU and the CPU replay of a tile: the largest, the 99.9th
/// percentile, and whether the tile had any content.
#[derive(Default)]
pub struct Diff {
    pub max: u8,
    pub p999: u8,
    pub ink: bool,
}

/// Compares tile `(x, y)`; `Err` when the GPU refused it.
pub fn compare(gpu: &mut Gpu, rec: &Recording, vp: &Viewport) -> Result<Diff, GpuUnsupported> {
    let (cpu, _) = rec.render(vp).expect("same zoom");
    let (scene, _) = rec.gpu_scene(vp, gpu.max_texture()).expect("same zoom")?;
    // The scan accepted the tile: vello_gpu must draw it (an error here is a scan gap).
    let gpu = gpu
        .render(&scene)
        .unwrap_or_else(|e| panic!("accepted tile failed on the GPU: {e}"));
    let cpu = cpu.pixmap.data_as_u8_slice();
    // Every pixel is opaque (white background), so premultiplied and straight RGBA agree.
    let mut hist = [0usize; 256];
    for (a, b) in cpu.iter().zip(&gpu) {
        hist[usize::from(a.abs_diff(*b))] += 1;
    }
    let cut = cpu.len() - cpu.len() / 1000;
    let (mut seen, mut p999) = (0, 0);
    for (d, n) in hist.iter().enumerate() {
        seen += n;
        if seen >= cut {
            p999 = d;
            break;
        }
    }
    Ok(Diff {
        max: hist.iter().rposition(|&n| n > 0).unwrap_or(0) as u8,
        p999: p999 as u8,
        ink: cpu.iter().any(|&c| c != 255),
    })
}

pub fn open(name: &str) -> Document {
    Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(format!("{name}.fig")),
    )
    .unwrap()
}

/// One rectangle (rotated 20°, half off tile (0, 0) to the left) with one feature.
pub fn feature_doc(feature: &str) -> Document {
    let gradient = |kind: &str| {
        let stop = |at: f64, r: f64, b: f64| {
            obj(&[
                ("position", Val::Float(at)),
                (
                    "color",
                    obj(&[
                        ("r", Val::Float(r)),
                        ("b", Val::Float(b)),
                        ("a", Val::Float(1.0)),
                    ]),
                ),
            ])
        };
        Val::Arr(vec![obj(&[
            ("type", Val::Str(kind.into())),
            ("visible", Val::Bool(true)),
            (
                "stops",
                Val::Arr(vec![
                    stop(0.0, 1.0, 0.0),
                    stop(0.6, 0.2, 0.8),
                    stop(1.0, 0.0, 0.2),
                ]),
            ),
        ])])
    };
    // "layer blur 150": the effect radius last (default 20).
    let radius = feature
        .rsplit(' ')
        .next()
        .and_then(|r| r.parse().ok())
        .unwrap_or(20.0);
    let parent = if feature == "clip" { 9 } else { 1 };
    let mut rect = node(2, parent, "RECTANGLE");
    rect.data.set("size", size(200.0, 140.0));
    let (c, s) = (20f64.to_radians().cos(), 20f64.to_radians().sin());
    rect.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(c)),
            ("m01", Val::Float(-s)),
            ("m10", Val::Float(s)),
            ("m11", Val::Float(c)),
            ("m02", Val::Float(-60.0)),
            ("m12", Val::Float(40.0)),
        ]),
    );
    rect.data.set("fillPaints", paint(0.9, 0.3, 0.1));
    rect.data.set("cornerRadius", Val::Float(18.0));
    let effect = |kind: &str| {
        Val::Arr(vec![obj(&[
            ("type", Val::Str(kind.into())),
            ("visible", Val::Bool(true)),
            ("color", obj(&[("a", Val::Float(0.6))])),
            (
                "offset",
                obj(&[("x", Val::Float(12.0)), ("y", Val::Float(16.0))]),
            ),
            ("radius", Val::Float(radius)),
        ])])
    };
    match feature {
        "fill" => {}
        "stroke" => {
            rect.data.set("strokePaints", paint(0.1, 0.1, 0.5));
            rect.data.set("strokeWeight", Val::Float(5.0));
        }
        "opacity" => rect.data.set("opacity", Val::Float(0.45)),
        "drop shadow" => rect.data.set("effects", effect("DROP_SHADOW")),
        f if f.starts_with("layer blur") => rect.data.set("effects", effect("LAYER_BLUR")),
        "linear gradient" => rect.data.set("fillPaints", gradient("GRADIENT_LINEAR")),
        "radial gradient" => rect.data.set("fillPaints", gradient("GRADIENT_RADIAL")),
        "angular gradient" => rect.data.set("fillPaints", gradient("GRADIENT_ANGULAR")),
        "clip" => {
            let mut frame = node(9, 1, "FRAME");
            frame.data.set("frameMaskDisabled", Val::Bool(false));
            frame.data.set("size", size(150.0, 150.0));
            frame
                .data
                .set("transform", obj(&[("m02", Val::Float(-30.0))]));
            return document(vec![node(1, 0, "CANVAS"), frame, rect]);
        }
        other => panic!("no feature {other}"),
    }
    document(vec![node(1, 0, "CANVAS"), rect])
}

/// Allowed differences, as (largest channel difference, 99.9th percentile), out of 255. Both
/// sides rasterize the same strips (`vello_common`); the GPU blends in 8-bit textures with its
/// own rounding, so plain fills, strokes, clips, layers and gradients differ by 1-2 at edges.
/// Blurs and shadows run a different decimated blur on each side (`vello_gpu` `filter.rs`):
/// measured on Metal, the GPU is up to 5/255 lighter in the soft ramp for every radius from
/// 2 to 150. That is not visible, so filters stay on the GPU with a looser bound.
pub const EXACT: (u8, u8) = (2, 1);
pub const FILTER: (u8, u8) = (6, 4);
/// Tiles at scales 0.75, 1, √2 (fractional) and 3, around page (0, 0): negative tiles too.
pub const SCALES: [f64; 4] = [0.75, 1.0, std::f64::consts::SQRT_2, 3.0];
/// Eight nested 300 x 300 frames, each blurred with `radius` (a blur inside a blur inside ...), filled.
pub fn nested_blurs(radius: f64) -> Document {
    nested_blurs_n(radius, 8)
}

/// `count` nested blurred frames, as [`nested_blurs`].
pub fn nested_blurs_n(radius: f64, count: u32) -> Document {
    let blur = || {
        Val::Arr(vec![obj(&[
            ("type", Val::Str("LAYER_BLUR".into())),
            ("visible", Val::Bool(true)),
            ("radius", Val::Float(radius)),
        ])])
    };
    let mut nodes = vec![node(1, 0, "CANVAS")];
    for n in 2..count + 2 {
        let mut frame = node(n, n - 1, "FRAME");
        frame.data.set("size", size(300.0, 300.0));
        frame.data.set("fillPaints", paint(0.2, 0.4, 0.8));
        frame.data.set("effects", blur());
        nodes.push(frame);
    }
    document(nodes)
}

/// `n` 100 px ellipses inside one frame with a layer blur of `radius`, all around the first
/// tile.
pub fn ellipses_under_blur(n: u32, radius: f64) -> Document {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set("size", size(500.0, 500.0));
    frame.data.set(
        "effects",
        Val::Arr(vec![obj(&[
            ("type", Val::Str("LAYER_BLUR".into())),
            ("visible", Val::Bool(true)),
            ("radius", Val::Float(radius)),
        ])]),
    );
    let mut nodes = vec![node(1, 0, "CANVAS"), frame];
    for i in 0..n {
        let mut e = node(i + 3, 2, "ELLIPSE");
        e.data.set("size", size(100.0, 100.0));
        let (x, y) = (f64::from(i * 37 % 400), f64::from(i * 53 % 400));
        e.data.set(
            "transform",
            obj(&[
                ("m00", Val::Float(1.0)),
                ("m11", Val::Float(1.0)),
                ("m02", Val::Float(x)),
                ("m12", Val::Float(y)),
            ]),
        );
        e.data.set("fillPaints", paint(0.5, 0.2, 0.2));
        nodes.push(e);
    }
    document(nodes)
}
