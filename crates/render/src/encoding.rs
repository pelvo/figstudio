//! Lossless image output for the CLI, MCP, desktop app and comparison reports.
//!
//! Input is **straight** RGBA8, as stored in PNG/WebP. For Vello frames use
//! `pixmap.take_rgba8(ImageAlphaType::Alpha)`, matching `Pixmap::into_png`'s conversion.

use image::{
    ExtendedColorType, ImageEncoder,
    codecs::{png::PngEncoder, webp::WebPEncoder},
};
use std::io;

/// Supported lossless output formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageFormat {
    #[default]
    Png,
    Webp,
}

impl ImageFormat {
    /// File extension without the leading dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }

    /// MIME type for inline images, downloads and clipboard data.
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

impl std::str::FromStr for ImageFormat {
    type Err = io::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "png" => Ok(Self::Png),
            "webp" => Ok(Self::Webp),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "format must be png or webp",
            )),
        }
    }
}

/// Encodes a rendered pixmap, preserving the optimized PNG path and its alpha conversion.
pub fn encode_pixmap(pixmap: vello_cpu::Pixmap, format: ImageFormat) -> io::Result<Vec<u8>> {
    if format == ImageFormat::Png {
        return pixmap.into_png().map_err(io::Error::other);
    }
    let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    let rgba = pixmap.take_rgba8(vello_cpu::peniko::ImageAlphaType::Alpha);
    encode(&rgba, w, h, format)
}

/// Encodes exactly `width * height` straight RGBA8 pixels without changing their values.
/// Invalid dimensions or buffer lengths return an error rather than panicking in a codec.
pub fn encode(rgba: &[u8], width: u32, height: u32, format: ImageFormat) -> io::Result<Vec<u8>> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len() as u64) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid RGBA image dimensions or length",
        ));
    }
    let mut bytes = Vec::new();
    match format {
        ImageFormat::Png => {
            PngEncoder::new(&mut bytes).write_image(rgba, width, height, ExtendedColorType::Rgba8)
        }
        ImageFormat::Webp => WebPEncoder::new_lossless(&mut bytes).write_image(
            rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
    }
    .map_err(io::Error::other)?;
    Ok(bytes)
}
