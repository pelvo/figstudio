//! Tiles drawn side by side at a fractional scale and pan match one render of the same area:
//! the GPU samples each tile texture bilinearly and clamps at its edge, which this test
//! reproduces on the CPU.

use figstudio_fig::Document;
use figstudio_render::kurbo::Affine;
use figstudio_render::{Renderer, Viewport};

use super::*;

/// Premultiplied RGBA pixels of a square image.
struct Image {
    side: usize,
    px: Vec<[f32; 4]>,
}

impl Image {
    fn render(doc: &Document, page: Guid, side: u16, transform: Affine) -> Image {
        let vp = Viewport {
            width: side,
            height: side,
            transform,
            background: Some(figstudio_render::page_background(doc, page)),
        };
        let frame = Renderer::new().render(doc, page, &vp);
        let px = frame
            .pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| c.map(f32::from))
            .collect();
        Image {
            side: usize::from(side),
            px,
        }
    }

    /// Bilinear sample at texture coordinate `(u, v)` in 0..1, clamped to the edge texels
    /// (egui's `TextureOptions::LINEAR`).
    fn sample(&self, u: f64, v: f64) -> [f32; 4] {
        let n = self.side as f64;
        let (x, y) = (u * n - 0.5, v * n - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let at = |x: f64, y: f64| {
            let c = |p: f64| p.clamp(0.0, n - 1.0) as usize;
            self.px[c(y) * self.side + c(x)]
        };
        let (a, b, c, d) = (
            at(x0, y0),
            at(x0 + 1.0, y0),
            at(x0, y0 + 1.0),
            at(x0 + 1.0, y0 + 1.0),
        );
        std::array::from_fn(|i| {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bottom = c[i] + (d[i] - c[i]) * fx;
            top + (bottom - top) * fy
        })
    }
}

/// Tile column and row whose left edge crosses sharp edges of the fixture at level 2.
const TX: i32 = 3;
const TY: i32 = 1;

#[test]
fn tiles_meet_without_a_seam_at_fractional_scale_and_pan() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    );
    let doc = Document::open(path).unwrap();
    let page = doc.pages[0];
    // Level 2 (2 px per unit, as at zoom 1 on a 1.5x screen): the shared edge of tiles
    // (TX - 1, TY) and (TX, TY) crosses sharp edges of the fixture.
    let tiles = [TX - 1, TX].map(|x| Key {
        doc: 1,
        page,
        level: 2,
        x,
        y: TY,
    });
    let images = tiles.map(|k| Image::render(&doc, page, TEXTURE, k.transform()));
    // Reference: one render of both tiles and a margin around them, in a single image.
    let t = f64::from(TILE);
    let pad = f64::from(GUTTER) + 1.0;
    let side = 2 * TILE + 2 * GUTTER + 2;
    let n = f64::from(side);
    let shift = Affine::translate((1.0, 1.0));
    let whole = Image::render(&doc, page, side, shift * tiles[0].transform());
    // Zoom 1 on a 1.5x screen shows level 2 scaled by 0.75, here at a pan of a third of a
    // pixel; compare screen pixels in a band around the shared edge.
    let (shrink, pan) = (0.75, 1.0 / 3.0);
    let edge = (t * shrink + pan) as usize;
    let [u0, u1] = uv().map(f64::from);
    let mut worst = 0.0_f32;
    let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
    for sy in 0..(t * shrink) as usize {
        for sx in edge - 3..edge + 3 {
            let (cx, cy) = (sx as f64 + 0.5, sy as f64 + 0.5);
            // Tile pixels under the screen pixel centre.
            let (tx, ty) = ((cx - pan) / shrink, (cy - pan) / shrink);
            let i = usize::from(tx >= t);
            let local = tx - t * i as f64;
            let tex = |p: f64| u0 + p / t * (u1 - u0);
            let tiled = images[i].sample(tex(local), tex(ty));
            let reference = whole.sample((tx + pad) / n, (ty + pad) / n);
            for c in 0..4 {
                worst = worst.max((tiled[c] - reference[c]).abs());
                lo[c] = lo[c].min(reference[c]);
                hi[c] = hi[c].max(reference[c]);
            }
        }
    }
    let contrast = (0..4).map(|c| hi[c] - lo[c]).fold(0.0, f32::max);
    assert!(
        contrast > 100.0,
        "the band is flat ({contrast}), so it proves nothing"
    );
    assert!(
        worst <= 3.0,
        "seam: a channel differs by {worst}/255 at the tile edge"
    );
}
