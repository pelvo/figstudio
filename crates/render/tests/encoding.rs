//! Lossless output contracts shared by export callers.
use figstudio_render::encoding::{ImageFormat, encode};

#[test]
fn both_formats_preserve_straight_rgba_including_transparency() {
    let rgba = [255, 0, 0, 255, 80, 160, 240, 128, 0, 0, 0, 0, 13, 29, 47, 1];
    for (format, extension, mime, decoded_format) in [
        (
            ImageFormat::Png,
            "png",
            "image/png",
            image::ImageFormat::Png,
        ),
        (
            ImageFormat::Webp,
            "webp",
            "image/webp",
            image::ImageFormat::WebP,
        ),
    ] {
        let bytes = encode(&rgba, 2, 2, format).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), decoded_format);
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.as_raw(), &rgba);
        assert_eq!(format.extension(), extension);
        assert_eq!(format.mime(), mime);
    }
}

#[test]
fn malformed_dimensions_and_buffer_lengths_return_errors() {
    for format in [ImageFormat::Png, ImageFormat::Webp] {
        for (rgba, width, height) in [
            (&[][..], 0, 1),
            (&[][..], 1, 0),
            (&[0; 3][..], 1, 1),
            (&[0; 8][..], 1, 1),
            (&[][..], u32::MAX, u32::MAX),
        ] {
            let error = encode(rgba, width, height, format).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        }
    }
}

#[test]
fn lossless_webp_is_smaller_on_flat_ui_content() {
    let (width, height) = (800, 600);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let pixel = if y < 48 || x < 160 {
                [32, 40, 56, 255]
            } else if (64..560).contains(&y) && (184..776).contains(&x) {
                [255, 255, 255, 255]
            } else {
                [232, 236, 240, 255]
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    let png = encode(&rgba, width as u32, height as u32, ImageFormat::Png).unwrap();
    let webp = encode(&rgba, width as u32, height as u32, ImageFormat::Webp).unwrap();
    assert!(
        webp.len() < png.len(),
        "PNG {}, WebP {}",
        png.len(),
        webp.len()
    );
}
