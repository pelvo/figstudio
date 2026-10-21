//! Browser colour emoji. JS objects are created on the calling render/export worker.
use figstudio_render::{EmojiRaster, EmojiSource};
use wasm_bindgen::JsCast as _;
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};

use crate::emoji_canvas_bounds::{self as geometry, Measured};

/// Contains no JS handles, so the renderer can move between Rayon workers safely.
#[derive(Debug)]
pub(crate) struct BrowserEmoji;

impl EmojiSource for BrowserEmoji {
    fn raster(&self, cluster: &str, pixels: u16) -> Option<EmojiRaster> {
        let canvas = OffscreenCanvas::new(1, 1).ok()?;
        let ctx: OffscreenCanvasRenderingContext2d =
            canvas.get_context("2d").ok()??.dyn_into().ok()?;
        let mut pixels = geometry::initial_pixels(pixels);
        let (font, bounds) = loop {
            let font = format!(
                "{pixels}px \"Apple Color Emoji\",\"Segoe UI Emoji\",\"Noto Color Emoji\",sans-serif"
            );
            ctx.set_font(&font);
            ctx.set_text_baseline("alphabetic");
            ctx.set_text_align("left");
            let metrics = ctx.measure_text(cluster).ok()?;
            match geometry::measure(
                pixels,
                [
                    metrics.actual_bounding_box_left(),
                    metrics.actual_bounding_box_right(),
                    metrics.actual_bounding_box_ascent(),
                    metrics.actual_bounding_box_descent(),
                ],
                metrics.width(),
                [
                    metrics.font_bounding_box_ascent(),
                    metrics.font_bounding_box_descent(),
                ],
            )? {
                Measured::Retry(smaller) => pixels = smaller,
                Measured::Raster(bounds) => break (font, bounds),
            }
        };
        canvas.set_width(u32::from(bounds.width));
        canvas.set_height(u32::from(bounds.height));
        // Resizing resets the context's font, baseline, alignment and fill style.
        ctx.set_font(&font);
        ctx.set_text_baseline("alphabetic");
        ctx.set_text_align("left");
        ctx.fill_text(cluster, -bounds.left, -bounds.top).ok()?;
        let rgba = ctx
            .get_image_data(0.0, 0.0, f64::from(bounds.width), f64::from(bounds.height))
            .ok()?
            .data()
            .0;
        // Canvas exposes neither the used font nor shaped glyphs; leave support unverified.
        EmojiRaster::from_rgba(rgba, bounds.width, bounds.height, bounds.left, bounds.top)?
            .with_ppem(pixels)
    }
}
