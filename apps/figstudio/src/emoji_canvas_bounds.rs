//! Canvas geometry is independent of JS so font/zoom limits are testable on the host.
#[derive(Debug)]
pub(crate) struct Bounds {
    pub(crate) left: f64,
    pub(crate) top: f64,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

#[derive(Debug)]
pub(crate) enum Measured {
    Raster(Bounds),
    Retry(u16),
}

pub(crate) fn initial_pixels(pixels: u16) -> u16 {
    pixels.clamp(1, 1022)
}

pub(crate) fn measure(
    pixels: u16,
    mut ink: [f64; 4],
    advance: f64,
    font: [f64; 2],
) -> Option<Measured> {
    if !ink.iter().all(|v| v.is_finite()) || !advance.is_finite() || advance <= 0.0 {
        return None;
    }
    if pixels == 0 {
        return None;
    }
    // Some colour-font backends report advance but no ink metrics.
    if ink[2] + ink[3] <= 0.0 {
        let [ascent, descent] = if font.iter().all(|v| v.is_finite()) && font[0] + font[1] > 0.0 {
            font
        } else {
            [f64::from(pixels), 0.0]
        };
        ink[2] = ascent;
        ink[3] = descent;
    }
    if ink[0] + ink[1] <= 0.0 {
        ink[0] = 0.0;
        ink[1] = advance;
    }
    let left = (-ink[0]).floor() - 1.0;
    let top = (-ink[2]).floor() - 1.0;
    let width = ink[1].max(advance).ceil() + 1.0 - left;
    let height = ink[3].ceil() + 1.0 - top;
    if width < 1.0 || height < 1.0 {
        return None;
    }
    if width > 1024.0 || height > 1024.0 {
        if pixels <= 1 {
            return None;
        }
        // Reserve the two antialiasing margins; remeasure at the smaller em.
        let ratio = (1022.0 / (width - 2.0).max(1.0)).min(1022.0 / (height - 2.0).max(1.0));
        let smaller = (f64::from(pixels) * ratio).floor().max(1.0) as u16;
        return Some(Measured::Retry(smaller.min(pixels - 1)));
    }
    Some(Measured::Raster(Bounds {
        left,
        top,
        width: width as u16,
        height: height as u16,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_em_and_wide_clusters_are_rasterized_at_a_smaller_ppem() {
        assert_eq!(initial_pixels(u16::MAX), 1022);
        let Some(Measured::Retry(pixels)) =
            measure(1000, [0.0, 6000.0, 800.0, 200.0], 6000.0, [800.0, 200.0])
        else {
            panic!("must shrink, not skip")
        };
        assert!(pixels < 1000);
        let scale = f64::from(pixels) / 1000.0;
        let Some(Measured::Raster(bounds)) = measure(
            pixels,
            [0.0, 6000.0 * scale, 800.0 * scale, 200.0 * scale],
            6000.0 * scale,
            [800.0 * scale, 200.0 * scale],
        ) else {
            panic!("must fit")
        };
        assert!(bounds.width <= 1024 && bounds.height <= 1024);
    }

    #[test]
    fn zero_ink_metrics_use_font_bounds_then_an_em_square() {
        for (font, height) in [([30.0, 10.0], 42), ([0.0, 0.0], 42)] {
            let Some(Measured::Raster(bounds)) = measure(40, [0.0; 4], 40.0, font) else {
                panic!("must provide a box")
            };
            assert_eq!(bounds.width, 42);
            assert_eq!(bounds.height, height, "not a two-pixel sliver");
            assert_eq!(bounds.left, -1.0);
            assert!(bounds.top <= -31.0);
        }
        assert!(measure(40, [f64::NAN; 4], 40.0, [30.0, 10.0]).is_none());
        assert!(measure(40, [0.0; 4], 0.0, [0.0; 2]).is_none());
    }
}
