//! CLI format defaults, independent of explicit output filenames.
use figstudio_render::encoding::ImageFormat;
use std::path::Path;

pub(crate) fn format(explicit: Option<ImageFormat>, out: &Path) -> ImageFormat {
    explicit.unwrap_or_else(|| {
        if out
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("webp"))
        {
            ImageFormat::Webp
        } else {
            ImageFormat::Png
        }
    })
}
