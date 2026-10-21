//! Browser colour emoji. JS objects are created on the calling render/export worker.
use figstudio_render::{EmojiRaster, EmojiSource};
use wasm_bindgen::JsCast as _;
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};

/// Contains no JS handles, so the renderer can move between Rayon workers safely.
#[derive(Debug)]
pub(crate) struct BrowserEmoji;

impl EmojiSource for BrowserEmoji {
    fn raster(&self, cluster: &str, pixels: u16) -> Option<EmojiRaster> {
        let canvas = OffscreenCanvas::new(1, 1).ok()?;
        let ctx: OffscreenCanvasRenderingContext2d =
            canvas.get_context("2d").ok()??.dyn_into().ok()?;
        let font = format!(
            "{pixels}px \"Apple Color Emoji\",\"Segoe UI Emoji\",\"Noto Color Emoji\",sans-serif"
        );
        ctx.set_font(&font);
        ctx.set_text_baseline("alphabetic");
        ctx.set_text_align("left");
        let metrics = ctx.measure_text(cluster).ok()?;
        let bounds = [
            metrics.actual_bounding_box_left(),
            metrics.actual_bounding_box_right(),
            metrics.actual_bounding_box_ascent(),
            metrics.actual_bounding_box_descent(),
            metrics.width(),
        ];
        if !bounds.iter().all(|v| v.is_finite()) || metrics.width() <= 0.0 {
            return None;
        }
        // Round outward, including a pixel of antialiasing margin around the measured ink.
        let left = (-bounds[0]).floor() - 1.0;
        let top = (-bounds[2]).floor() - 1.0;
        let width = bounds[1].max(bounds[4]).ceil() + 1.0 - left;
        let height = bounds[3].ceil() + 1.0 - top;
        if !(1.0..=1024.0).contains(&width) || !(1.0..=1024.0).contains(&height) {
            return None;
        }
        canvas.set_width(width as u32);
        canvas.set_height(height as u32);
        // Resizing resets the context's font, baseline, alignment and fill style.
        ctx.set_font(&font);
        ctx.set_text_baseline("alphabetic");
        ctx.set_text_align("left");
        ctx.fill_text(cluster, -left, -top).ok()?;
        let rgba = ctx.get_image_data(0.0, 0.0, width, height).ok()?.data().0;
        EmojiRaster::from_rgba(rgba, width as u16, height as u16, left, top)
    }
}
