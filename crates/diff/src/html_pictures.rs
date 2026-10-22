//! Safe local pictures, canvas locators and effective render measures.
use crate::html::escape;
use crate::{Region, RenderDiff, ScreenDiff};
use std::fmt::Write;

pub(crate) fn relative_link(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains([':', '\\', '?', '#', '%'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, ".." | ""))
}

pub(crate) fn effective(render: &RenderDiff) -> Option<u64> {
    render.pixels_changed
}

pub(crate) fn picture(out: &mut String, path: &Option<String>, label: &str, class: &str) {
    picture_sized(out, path, label, class, None);
}

/// `size` is the picture's pixels at 2x; the attributes give its natural CSS size, which CSS
/// only ever shrinks (`max-width:100%`), so a small crop stays sharp and is never upscaled.
fn picture_sized(
    out: &mut String,
    path: &Option<String>,
    label: &str,
    class: &str,
    size: Option<(u32, u32)>,
) {
    if let Some(path) = path.as_deref().filter(|p| relative_link(p)) {
        let path = escape(path);
        let dims = size.map_or(String::new(), |(w, h)| {
            format!(" width=\"{}\" height=\"{}\"", w.div_ceil(2), h.div_ceil(2))
        });
        // The label sits above the picture, never over its pixels.
        let px = size.map_or(String::new(), |(w, h)| {
            format!(
                "<span class=\"px\">{}\u{d7}{}</span>",
                w.div_ceil(2),
                h.div_ceil(2)
            )
        });
        let _ = write!(
            out,
            "<figure class=\"{class}\"><figcaption><span class=\"pill\">{label}</span>{px}</figcaption><a href=\"./{path}\"><img src=\"./{path}\"{dims} loading=\"lazy\" alt=\"{label}\"></a></figure>"
        );
    }
}

pub(crate) fn compare(out: &mut String, region: &Region, k: usize) {
    let size = region.image_width.zip(region.image_height);
    compare_sized(
        out,
        (&region.old_png, &region.new_png),
        size,
        &format!("Area {k}"),
    );
}

pub(crate) fn compare_sized(
    out: &mut String,
    (old, new): (&Option<String>, &Option<String>),
    size: Option<(u32, u32)>,
    label: &str,
) {
    let _ = write!(
        out,
        "<div class=\"cmp\" tabindex=\"0\" aria-label=\"{}: Before / After\">",
        escape(label)
    );
    picture_sized(out, old, "Before", "bf", size);
    picture_sized(out, new, "After", "af", size);
    out.push_str(
        "<span class=\"hint\">Showing After · hover, focus or press to see Before</span></div>",
    );
}

pub(crate) fn locator(out: &mut String, render: &ScreenDiff, region: &Region) {
    let Some(path) = render.thumb_new.as_deref().filter(|p| relative_link(p)) else {
        return;
    };
    let (Some(w), Some(h)) = (
        Some(render.width).filter(|w| *w > 0),
        Some(render.height).filter(|h| *h > 0),
    ) else {
        return;
    };
    let pct = |n: u32, total: u32| (100.0 * f64::from(n) / f64::from(total)).clamp(0.0, 100.0);
    let _ = write!(
        out,
        "<figure class=\"loc\" aria-label=\"Where this area is on the screen\"><img src=\"./{}\" alt=\"\" loading=\"lazy\"><span class=\"box\" style=\"left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%\"></span></figure>",
        escape(path),
        pct(region.x, w),
        pct(region.y, h),
        pct(region.width, w),
        pct(region.height, h)
    );
}

pub(crate) fn whole_page(out: &mut String, render: &RenderDiff) {
    out.push_str("<div class=\"sec\"><details class=\"whole\"><summary>Whole page · Before / After / Diff</summary>");
    if effective(render) == Some(0) {
        out.push_str("<p class=\"none\">No visual change</p>");
    }
    out.push_str("<div class=\"pics\">");
    if effective(render) != Some(0) {
        picture(out, &render.old_png, "Before", "bf");
    }
    picture(out, &render.new_png, "After", "af");
    if effective(render) != Some(0) {
        picture(out, &render.diff_png, "Diff", "df");
    }
    out.push_str("</div></details></div>");
}

pub(crate) fn measures(render: &RenderDiff) -> String {
    let mut parts = Vec::new();
    if let Some(pixels) = effective(render) {
        parts.push(format!("{pixels} pixels changed"));
    }
    if let Some(worst) = render.worst_channel_delta {
        parts.push(format!("worst channel delta {worst}"));
    }
    if let (Some(w), Some(h)) = (render.width, render.height) {
        parts.push(format!("{w}x{h}"));
    }
    parts.join(", ")
}
