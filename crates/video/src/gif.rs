//! Animated GIF (looping), through the `image` crate's encoder.

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};

use crate::{Encoder, Size, VideoError};

/// Where the encoder writes; the bytes are taken once it is dropped (it writes the trailer
/// then: `image` gives no other way back to its writer).
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) struct Gif {
    size: Size,
    fps: u32,
    bytes: Shared,
    out: Option<GifEncoder<Shared>>,
    frames: usize,
}

impl Gif {
    pub fn new(size: Size, fps: u32) -> Self {
        Gif {
            size,
            fps,
            bytes: Shared::default(),
            out: None,
            frames: 0,
        }
    }
}

fn gif_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::new(format!("GIF: {e}"))
}

impl Encoder for Gif {
    fn push(&mut self, rgba: &[u8]) -> Result<(), VideoError> {
        self.size.check(rgba)?;
        let out = match &mut self.out {
            Some(out) => out,
            None => {
                // Speed 10 of 30: a fair palette per frame without minutes per frame.
                let mut out = GifEncoder::new_with_speed(self.bytes.clone(), 10);
                out.set_repeat(Repeat::Infinite).map_err(gif_error)?;
                self.out.insert(out)
            }
        };
        let image = RgbaImage::from_raw(
            u32::from(self.size.width),
            u32::from(self.size.height),
            rgba.to_vec(),
        )
        .ok_or_else(|| gif_error("bad frame"))?;
        // GIF counts in hundredths of a second: each frame ends at its own rounded time, so
        // 30 fps runs 3, 3, 4… and the whole animation keeps its length.
        let at = |i: usize| (i as u64 * 100 + u64::from(self.fps) / 2) / u64::from(self.fps);
        let cs = at(self.frames + 1) - at(self.frames);
        let delay = Delay::from_numer_denom_ms(u32::try_from(cs * 10).unwrap_or(u32::MAX), 1);
        out.encode_frame(Frame::from_parts(image, 0, 0, delay))
            .map_err(gif_error)?;
        self.frames += 1;
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, VideoError> {
        let Some(out) = self.out else {
            return Err(gif_error("no frames"));
        };
        drop(out);
        let bytes =
            std::mem::take(&mut *self.bytes.0.lock().unwrap_or_else(PoisonError::into_inner));
        Ok(bytes)
    }
}
