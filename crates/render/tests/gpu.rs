//! A recorded tile replayed into a `vello_gpu` scene and rendered on this machine's GPU draws
//! what the CPU replay draws (feature `gpu`). Skips, with a printed reason, when there is no
//! GPU adapter (CI, headless machines).
#![cfg(feature = "gpu")]
mod common;
mod gpu_support;
use common::*;
use figstudio_fig::{Document, Guid};
use figstudio_render::GpuUnsupported;
use gpu_support::Gpu;
use gpu_support::docs::*;

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
