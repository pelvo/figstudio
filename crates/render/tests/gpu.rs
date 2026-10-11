//! A recorded tile replayed into a `vello_gpu` scene and rendered on this machine's GPU draws
//! what the CPU replay draws (feature `gpu`). Skips, with a printed reason, when there is no
//! GPU adapter (CI, headless machines).
#![cfg(feature = "gpu")]
mod common;
mod gpu_support;
use common::*;
use figstudio_fig::{Document, Guid, Val};
use figstudio_render::kurbo::{Affine, Rect};
use figstudio_render::{GpuUnsupported, Recording, Renderer, Viewport};
use gpu_support::{Gpu, SIDE};

const TILE: f64 = 512.0;

fn renderer() -> Renderer {
    let mut r = Renderer::new();
    r.system_fonts = false;
    r
}

fn tile(scale: f64, x: i32, y: i32) -> Viewport {
    Viewport {
        width: SIDE,
        height: SIDE,
        transform: Affine::translate((2.0 - f64::from(x) * TILE, 2.0 - f64::from(y) * TILE))
            * Affine::scale(scale),
        background: Some(figstudio_render::color::AlphaColor::new([1.0; 4])),
    }
}

fn record(doc: &Document, page: Guid, scale: f64) -> Recording {
    let e = 64.0 * TILE;
    let area = Rect::new(-e, -e, e, e);
    let (rec, _) = renderer().record(doc, page, Affine::scale(scale), area, usize::MAX);
    rec.expect("no limit")
}

/// Channel differences between the GPU and the CPU replay of a tile: the largest, the 99.9th
/// percentile, and whether the tile had any content.
#[derive(Default)]
struct Diff {
    max: u8,
    p999: u8,
    ink: bool,
}

/// Compares tile `(x, y)`; `Err` when the GPU refused it.
fn compare(gpu: &mut Gpu, rec: &Recording, vp: &Viewport) -> Result<Diff, GpuUnsupported> {
    let (cpu, _) = rec.render(vp).expect("same zoom");
    let (scene, _) = rec.gpu_scene(vp).expect("same zoom")?;
    let gpu = gpu.render(&scene);
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

fn open(name: &str) -> Document {
    Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(format!("{name}.fig")),
    )
    .unwrap()
}

/// One rectangle (rotated 20°, half off tile (0, 0) to the left) with one feature.
fn feature_doc(feature: &str) -> Document {
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
const EXACT: (u8, u8) = (2, 1);
const FILTER: (u8, u8) = (6, 4);
/// Tiles at scales 0.75, 1, √2 (fractional) and 3, around page (0, 0): negative tiles too.
const SCALES: [f64; 4] = [0.75, 1.0, std::f64::consts::SQRT_2, 3.0];

/// Compares tiles `-2..=1` each way at every scale; returns the worst max and 99.9th percentile
/// over tiles with content.
fn worst(gpu: &mut Gpu, doc: &Document, page: Guid, what: &str) -> (u8, u8) {
    let (mut max, mut p999, mut inked) = (0, 0, 0);
    for scale in SCALES {
        let rec = record(doc, page, scale);
        for y in -2..=1 {
            for x in -2..=1 {
                let d = compare(gpu, &rec, &tile(scale, x, y))
                    .unwrap_or_else(|e| panic!("{what} at {scale} tile ({x},{y}): {e}"));
                if d.ink {
                    inked += 1;
                    max = max.max(d.max);
                    p999 = p999.max(d.p999);
                }
            }
        }
    }
    assert!(inked > 0, "{what}: no tile had content");
    eprintln!("{what}: max {max}/255, p99.9 {p999}/255 over {inked} tiles");
    (max, p999)
}

#[test]
fn gpu_tiles_draw_each_feature_like_the_cpu() {
    let Some(mut gpu) = Gpu::new() else { return };
    let features = [
        ("fill", EXACT),
        ("stroke", EXACT),
        ("opacity", EXACT),
        ("clip", EXACT),
        ("linear gradient", EXACT),
        ("radial gradient", EXACT),
        ("drop shadow", FILTER),
        ("layer blur", FILTER),
        ("layer blur 150", FILTER),
    ];
    for (feature, (most, p)) in features {
        let (max, p999) = worst(&mut gpu, &feature_doc(feature), id(1), feature);
        assert!(
            max <= most && p999 <= p,
            "{feature}: max {max}, p99.9 {p999}"
        );
    }
    let (max, p999) = worst(&mut gpu, &effects_doc(), id(1), "effects doc");
    assert!(
        max <= FILTER.0 && p999 <= FILTER.1,
        "effects doc: {max}, {p999}"
    );
}

/// Fixture tiles around each page's content draw like the CPU; tiles with images are refused
/// (and only those).
#[test]
fn gpu_tiles_draw_the_fixtures_like_the_cpu() {
    let Some(mut gpu) = Gpu::new() else { return };
    let names = [
        "corners",
        "sections",
        "stacks_wrap",
        "structure",
        "vector",
        "noise_and_grain",
    ];
    let mut refused = 0;
    for name in names {
        let doc = open(name);
        let page = doc.pages[0];
        let b = figstudio_render::content_bounds(&doc, page).unwrap();
        let (mut max, mut p999) = (0, 0);
        for scale in SCALES {
            let rec = record(&doc, page, scale);
            let k = scale / TILE;
            let (x0, y0) = ((b.x0 * k).floor() as i32, (b.y0 * k).floor() as i32);
            for y in y0 - 1..=y0 + 2 {
                for x in x0 - 1..=x0 + 2 {
                    match compare(&mut gpu, &rec, &tile(scale, x, y)) {
                        Ok(d) => (max, p999) = (max.max(d.max), p999.max(d.p999)),
                        Err(GpuUnsupported::Image) if name == "structure" => refused += 1,
                        Err(e) => panic!("{name} at {scale} tile ({x},{y}): {e}"),
                    }
                }
            }
        }
        eprintln!("{name}: max {max}/255, p99.9 {p999}/255");
        assert!(
            max <= EXACT.0 && p999 <= EXACT.1,
            "{name}: max {max}, p99.9 {p999}"
        );
    }
    assert!(refused > 0, "structure's image tiles are refused");
}

/// Refusals need no GPU: the scan runs before any scene is built.
#[test]
fn unsupported_tiles_are_refused_before_encoding() {
    let doc = open("structure");
    let rec = record(&doc, doc.pages[0], 1.0);
    let b = figstudio_render::content_bounds(&doc, doc.pages[0]).unwrap();
    let images = (-1..=2)
        .flat_map(|y| (-1..=2).map(move |x| (x, y)))
        .map(|(x, y)| {
            (
                x + (b.x0 / TILE).floor() as i32,
                y + (b.y0 / TILE).floor() as i32,
            )
        })
        .filter(|&(x, y)| {
            rec.gpu_scene(&tile(1.0, x, y)).unwrap().err() == Some(GpuUnsupported::Image)
        })
        .count();
    assert!(images > 0, "a tile with an image paint");
    // A blur reaching 3 x 1000 px: its layer would not fit the GPU's intermediate textures.
    let blurred = feature_doc("layer blur 2000");
    let rec = record(&blurred, id(1), 1.0);
    let got = rec.gpu_scene(&tile(1.0, 0, 0)).unwrap().err();
    assert_eq!(got, Some(GpuUnsupported::FilterReach));
    // The same blur at a smaller zoom fits.
    let rec = record(&blurred, id(1), 0.25);
    assert!(rec.gpu_scene(&tile(0.25, 0, 0)).unwrap().is_ok());
    // Another zoom is refused like `Recording::render` refuses it.
    assert!(rec.gpu_scene(&tile(1.0, 0, 0)).is_none());
    // An angular gradient's seam differs on the GPU (start against end colour).
    let rec = record(&feature_doc("angular gradient"), id(1), 1.0);
    let got = rec.gpu_scene(&tile(1.0, 0, 0)).unwrap().err();
    assert_eq!(got, Some(GpuUnsupported::SweepGradient));
}
