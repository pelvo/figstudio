//! Tiles `vello_gpu` cannot draw are refused before a scene is built (feature `gpu`): the
//! scan needs no GPU; the checks against the GPU itself skip without an adapter.
#![cfg(feature = "gpu")]
mod common;
mod gpu_support;
use common::*;
use figstudio_render::{GPU_LAYER_TEXTURE, GpuUnsupported};
use gpu_support::Gpu;
use gpu_support::docs::*;

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
            rec.gpu_scene(&tile(1.0, x, y), GPU_LAYER_TEXTURE)
                .unwrap()
                .err()
                == Some(GpuUnsupported::Image)
        })
        .count();
    assert!(images > 0, "a tile with an image paint");
    // A blur reaching 3 x 1000 px: its layer would not fit the GPU's intermediate textures.
    let blurred = feature_doc("layer blur 2000");
    let rec = record(&blurred, id(1), 1.0);
    let got = rec
        .gpu_scene(&tile(1.0, 0, 0), GPU_LAYER_TEXTURE)
        .unwrap()
        .err();
    assert_eq!(got, Some(GpuUnsupported::FilterReach));
    // The same blur at a smaller zoom fits.
    let rec = record(&blurred, id(1), 0.125);
    assert!(
        rec.gpu_scene(&tile(0.125, 0, 0), GPU_LAYER_TEXTURE)
            .unwrap()
            .is_ok()
    );
    // Another zoom is refused like `Recording::render` refuses it.
    assert!(rec.gpu_scene(&tile(1.0, 0, 0), GPU_LAYER_TEXTURE).is_none());
    // An angular gradient's seam differs on the GPU (start against end colour).
    let rec = record(&feature_doc("angular gradient"), id(1), 1.0);
    let got = rec
        .gpu_scene(&tile(1.0, 0, 0), GPU_LAYER_TEXTURE)
        .unwrap()
        .err();
    assert_eq!(got, Some(GpuUnsupported::SweepGradient));
}

/// Each nested filter grows `vello_gpu`'s intermediate viewport by its padding: one radius-300
/// blur fits a 516 px tile (about 2.3k px against 4096), eight nested ones do not (vello_gpu
/// returns `IntermediateTextureError::TooLarge` on Metal). They are refused before a scene is
/// built.
#[test]
fn nested_filters_that_outgrow_gpu_intermediates_are_refused() {
    let rec = record(&nested_blurs(300.0), id(1), 1.0);
    let got = rec
        .gpu_scene(&tile(1.0, 0, 0), GPU_LAYER_TEXTURE)
        .unwrap()
        .err();
    assert_eq!(got, Some(GpuUnsupported::FilterReach));
    // One such blur alone fits.
    let rec = record(&nested_blurs_n(300.0, 1), id(1), 1.0);
    assert!(
        rec.gpu_scene(&tile(1.0, 0, 0), GPU_LAYER_TEXTURE)
            .unwrap()
            .is_ok()
    );
    // Small nested blurs fit, and the GPU draws them (when there is one).
    let rec = record(&nested_blurs(8.0), id(1), 1.0);
    assert!(
        rec.gpu_scene(&tile(1.0, 0, 0), GPU_LAYER_TEXTURE)
            .unwrap()
            .is_ok()
    );
    if let Some(mut gpu) = Gpu::new() {
        let d = compare(&mut gpu, &rec, &tile(1.0, 0, 0)).unwrap();
        assert!(d.ink && d.max <= FILTER.0, "nested blurs: max {}", d.max);
    }
}

/// The scan is conservative about `vello_gpu`'s own sizing: across the limit, every tile it
/// accepts renders. It sizes a filter layer as its whole grown viewport, not its content, so it
/// refuses earlier than the GPU fails (measured on Metal: radius 150 against 300).
#[test]
fn every_tile_the_scan_accepts_renders_on_the_gpu() {
    let Some(mut gpu) = Gpu::new() else { return };
    let max = gpu.max_texture();
    let mut first_refused = None;
    let mut first_failed = None;
    for radius in (4..=28).map(|r| f64::from(r) * 25.0) {
        let rec = record(&nested_blurs(radius), id(1), 1.0);
        let vp = tile(1.0, 0, 0);
        let refused = rec.gpu_scene(&vp, max).unwrap().is_err();
        let (scene, _) = rec.gpu_scene(&vp, u16::MAX).unwrap().unwrap();
        let failed = gpu.render(&scene).is_err();
        assert!(
            refused || !failed,
            "radius {radius}: accepted but the GPU failed"
        );
        first_refused = first_refused.or(refused.then_some(radius));
        first_failed = first_failed.or(failed.then_some(radius));
    }
    eprintln!("first refused {first_refused:?}, first failed {first_failed:?}");
    assert!(first_failed.is_some(), "the sweep reaches the GPU limit");
    assert!(
        first_refused <= first_failed,
        "refused no later than it fails"
    );
}
