//! RGBA to 8-bit YUV 4:2:0, BT.709 limited range (what video players assume for HD).

/// The three planes of one frame: `width × height` luma, then half-size Cb and Cr.
pub(crate) struct Planes {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

/// Converts `rgba` (`width × height`, even sides) ignoring alpha.
pub(crate) fn from_rgba(rgba: &[u8], width: usize, height: usize) -> Planes {
    let (cw, ch) = (width / 2, height / 2);
    let mut planes = Planes {
        y: vec![0; width * height],
        u: vec![0; cw * ch],
        v: vec![0; cw * ch],
    };
    let px = |x: usize, y: usize| {
        let i = (y * width + x) * 4;
        (
            i32::from(rgba[i]),
            i32::from(rgba[i + 1]),
            i32::from(rgba[i + 2]),
        )
    };
    for y in 0..height {
        for x in 0..width {
            let (r, g, b) = px(x, y);
            // BT.709, scaled by 256: Y = 16 + 0.1826 R + 0.6142 G + 0.0620 B.
            planes.y[y * width + x] = (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8;
        }
    }
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut r, mut g, mut b) = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px(cx * 2 + dx, cy * 2 + dy);
                r += p.0;
                g += p.1;
                b += p.2;
            }
            let (r, g, b) = (r / 4, g / 4, b / 4);
            let u = ((-26 * r - 87 * g + 112 * b + 128) >> 8) + 128;
            let v = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
            planes.u[cy * cw + cx] = u.clamp(0, 255) as u8;
            planes.v[cy * cw + cx] = v.clamp(0, 255) as u8;
        }
    }
    planes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_white_and_red_land_where_bt709_puts_them() {
        let rgba = [
            0, 0, 0, 255, 255, 255, 255, 255, //
            255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let p = from_rgba(&rgba, 2, 2);
        assert_eq!(&p.y[..2], &[16, 235]);
        // Pure red: Y 63, and Cr high, Cb low (BT.709: 63, 102, 240).
        assert!((62..=64).contains(&p.y[2]), "{}", p.y[2]);
    }
}
