//! Sharp region pictures: page coordinates are rendered again, not enlarged from a thumbnail.
use crate::{Region, RegionImages, render::ImageReader, render_pixels::encode};
use figstudio_fig::{Document, Guid};
use figstudio_render::{
    Renderer, Viewport,
    kurbo::{Affine, Rect},
    page_background,
};

fn crop_size(rect: Rect, available: usize) -> (f64, u32, u32, bool) {
    let max_scale = 2.0_f64.min(2048.0 / rect.width().max(rect.height()).max(1.0));
    let size = |scale: f64| {
        (
            (rect.width() * scale).ceil().max(1.0) as u32,
            (rect.height() * scale).ceil().max(1.0) as u32,
        )
    };
    let fits = |w: u32, h: u32| w as usize * h as usize * 2 <= available;
    let (mut w, mut h) = size(max_scale);
    let mut scale = max_scale;
    if !fits(w, h) {
        let (mut low, mut high) = (0.0, max_scale);
        for _ in 0..48 {
            let mid = (low + high) / 2.0;
            let (w, h) = size(mid);
            if fits(w, h) {
                low = mid;
            } else {
                high = mid;
            }
        }
        scale = low;
        (w, h) = size(scale);
    }
    (scale, w, h, scale < 2.0)
}

/// Smallest context shown around a change, in pixels of the compared screen picture (the
/// canvas, so a downscaled screen shows the same visible context as a full-size one).
const MIN_CROP: (f64, f64) = (320.0, 160.0);

/// Grows `rect` to the minimum context, centred and clamped to the screen canvas.
fn with_context(rect: Rect, canvas: (u32, u32)) -> Rect {
    let grow = |lo: f64, hi: f64, min: f64, limit: f64| {
        let len = (hi - lo).max(min.min(limit));
        let start = ((lo + hi - len) / 2.0).clamp(0.0, (limit - len).max(0.0));
        (start, start + len)
    };
    let (x0, x1) = grow(rect.x0, rect.x1, MIN_CROP.0, f64::from(canvas.0));
    let (y0, y1) = grow(rect.y0, rect.y1, MIN_CROP.1, f64::from(canvas.1));
    Rect::new(x0, y0, x1, y1)
}

pub(crate) fn render_crops(
    renderers: (&mut Renderer, &mut Renderer),
    docs: (&Document, &Document),
    pages: (Guid, Guid),
    readers: (ImageReader<'_>, ImageReader<'_>),
    regions: Vec<Region>,
    (transforms, design_scale, format, canvas): (
        (Affine, Affine),
        f64,
        figstudio_render::encoding::ImageFormat,
        (u32, u32),
    ),
    remaining: &mut usize,
) -> (Vec<Region>, Vec<RegionImages>, bool) {
    let (old_renderer, new_renderer) = renderers;
    let (old, new) = docs;
    let (old_page, new_page) = pages;
    let mut capped = false;
    let mut kept = Vec::new();
    let mut pictures = Vec::new();
    let count = regions.len();
    for (i, mut region) in regions.into_iter().enumerate() {
        let rect = with_context(
            Rect::new(
                region.x as f64,
                region.y as f64,
                (region.x + region.width) as f64,
                (region.y + region.height) as f64,
            ),
            canvas,
        );
        // Share the remaining budget rather than reducing later areas to single pixels.
        let available = *remaining / (count - i);
        if available < 2 {
            capped = true;
            break;
        }
        let design_rect = Rect::new(
            0.0,
            0.0,
            rect.width() / design_scale,
            rect.height() / design_scale,
        );
        let (scale, width, height, limited) = crop_size(design_rect, available);
        *remaining -= width as usize * height as usize * 2;
        capped |= limited;
        let crop = Affine::scale(scale / design_scale) * Affine::translate((-rect.x0, -rect.y0));
        let draw =
            |renderer: &mut Renderer, doc: &Document, page, read: ImageReader, canvas: Affine| {
                encode(
                    &renderer.render_complete(
                        doc,
                        page,
                        &Viewport {
                            width: width as u16,
                            height: height as u16,
                            transform: crop * canvas,
                            background: (doc.node(page).is_some_and(|n| n.kind == "CANVAS"))
                                .then(|| page_background(doc, page)),
                        },
                        read,
                    ),
                    format,
                )
            };
        let before = draw(old_renderer, old, old_page, readers.0, transforms.0);
        let after = draw(new_renderer, new, new_page, readers.1, transforms.1);
        if let (Some(before), Some(after)) = (before, after) {
            region.image_width = Some(width);
            region.image_height = Some(height);
            kept.push(region);
            pictures.push(RegionImages { before, after });
        }
    }
    (kept, pictures, capped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_regions_get_context_clamped_to_the_canvas_and_large_ones_stay() {
        let r = with_context(Rect::new(5.0, 5.0, 15.0, 15.0), (1000, 800));
        assert_eq!(
            (r.x0, r.y0, r.width(), r.height()),
            (0.0, 0.0, 320.0, 160.0)
        );
        let r = with_context(Rect::new(500.0, 400.0, 510.0, 410.0), (1000, 800));
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (345.0, 325.0, 665.0, 485.0));
        let r = with_context(Rect::new(0.0, 0.0, 600.0, 300.0), (1000, 800));
        assert_eq!((r.width(), r.height()), (600.0, 300.0));
        // Near an edge the box shifts inward and keeps its size.
        let r = with_context(Rect::new(995.0, 400.0, 1000.0, 405.0), (1000, 800));
        assert_eq!(
            (r.x0, r.x1, r.width(), r.height()),
            (680.0, 1000.0, 320.0, 160.0)
        );
        let r = with_context(Rect::new(5.0, 790.0, 15.0, 800.0), (1000, 800));
        assert_eq!((r.y0, r.y1, r.width()), (640.0, 800.0, 320.0));
        let r = with_context(Rect::new(10.0, 10.0, 20.0, 20.0), (100, 50));
        assert_eq!((r.width(), r.height()), (100.0, 50.0));
    }
}
