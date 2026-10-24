//! Image encoding and raw premultiplied pixel comparison.
use figstudio_render::{
    Frame,
    encoding::{ImageFormat, encode as encode_image},
};

pub(crate) fn encode(frame: &Frame, format: ImageFormat) -> Option<Vec<u8>> {
    encode_pixels(
        frame.pixmap.data_as_u8_slice(),
        u32::from(frame.pixmap.width()),
        u32::from(frame.pixmap.height()),
        format,
    )
}

pub(crate) fn encode_pixels(
    pixels: &[u8],
    width: u32,
    height: u32,
    format: ImageFormat,
) -> Option<Vec<u8>> {
    let mut straight = vec![0u8; pixels.len()];
    let (src, _) = pixels.as_chunks::<4>();
    let (dst, _) = straight.as_chunks_mut::<4>();
    for (src, dst) in src.iter().zip(dst) {
        let a = src[3];
        if a != 0 {
            for i in 0..3 {
                dst[i] = u8::try_from((u32::from(src[i]) * 255 + u32::from(a) / 2) / u32::from(a))
                    .unwrap_or(u8::MAX);
            }
            dst[3] = a;
        }
    }
    encode_image(&straight, width, height, format).ok()
}

pub(crate) fn pixel4(px: &[u8], at: usize) -> [u8; 4] {
    px.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .unwrap_or([0; 4])
}

pub(crate) fn channel_delta(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(y))
        .max()
        .unwrap_or(0)
}

pub(crate) fn premul_rgba(
    color: figstudio_render::color::AlphaColor<figstudio_render::color::Srgb>,
) -> [u8; 4] {
    let p = color.premultiply().to_rgba8();
    [p.r, p.g, p.b, p.a]
}

/// The per-pixel channel delta between the two sides (a missing side compares to its page
/// background) and the colour the diff picture shows for an unchanged pixel.
struct Sides<'a> {
    before: &'a Option<Frame>,
    after: &'a Option<Frame>,
    old_bg: [u8; 4],
    new_bg: [u8; 4],
}

impl Sides<'_> {
    fn at(&self, at: usize) -> (u8, [u8; 4]) {
        let px = |frame: &Frame| pixel4(frame.pixmap.data_as_u8_slice(), at);
        let base = match (self.after, self.before) {
            (Some(frame), _) | (None, Some(frame)) => px(frame),
            (None, None) => [0; 4],
        };
        let delta = match (self.before, self.after) {
            (Some(old), Some(new)) => channel_delta(px(old), px(new)),
            (Some(frame), None) => channel_delta(px(frame), self.old_bg),
            (None, Some(frame)) => channel_delta(px(frame), self.new_bg),
            (None, None) => 0,
        };
        (delta, base)
    }
}

/// Changed pixel count, worst channel delta, the diff picture (premultiplied RGBA) and the
/// per-pixel changed mask.
pub(crate) fn compare(
    before: &Option<Frame>,
    after: &Option<Frame>,
    old_bg: [u8; 4],
    new_bg: [u8; 4],
    width: u32,
    height: u32,
    threshold: u8,
) -> (u64, u8, Vec<u8>, Vec<bool>) {
    let count = (width as usize) * (height as usize);
    let sides = Sides {
        before,
        after,
        old_bg,
        new_bg,
    };
    let mut diff = vec![0u8; count * 4];
    let mut changed = 0u64;
    let mut worst = 0u8;
    let mut delta_flags = Vec::with_capacity(count);
    for i in 0..count {
        let at = i * 4;
        let (delta, base) = sides.at(at);
        worst = worst.max(delta);
        delta_flags.push(delta > threshold);
        if delta > threshold {
            changed += 1;
            diff[at..at + 4].copy_from_slice(&[255, 0, 0, 255]);
        } else {
            let gray =
                u8::try_from((u16::from(base[0]) + u16::from(base[1]) + u16::from(base[2])) / 3)
                    .unwrap_or(0);
            diff[at..at + 4].copy_from_slice(&[gray, gray, gray, base[3]]);
        }
    }
    (changed, worst, diff, delta_flags)
}

/// Just the changed mask of [`compare`], without allocating the diff picture.
pub(crate) fn compare_mask(
    before: &Option<Frame>,
    after: &Option<Frame>,
    (old_bg, new_bg): ([u8; 4], [u8; 4]),
    (width, height): (u32, u32),
    threshold: u8,
) -> Vec<bool> {
    let sides = Sides {
        before,
        after,
        old_bg,
        new_bg,
    };
    (0..(width as usize) * (height as usize))
        .map(|i| sides.at(i * 4).0 > threshold)
        .collect()
}
