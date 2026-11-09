//! AV1 through rav1e: frames in, compressed packets out (for the WebM and MP4 writers).

use rav1e::prelude::*;

use crate::{Size, VideoError, yuv};

/// One compressed frame.
pub(crate) struct Packet {
    pub data: Vec<u8>,
    /// A key frame (a random access point).
    pub key: bool,
}

pub(crate) struct Av1 {
    ctx: Context<u8>,
    size: Size,
}

fn av1_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::new(format!("AV1: {e}"))
}

impl Av1 {
    pub fn new(size: Size, fps: u32) -> Result<Self, VideoError> {
        let enc = EncoderConfig {
            width: usize::from(size.width),
            height: usize::from(size.height),
            time_base: Rational::new(1, u64::from(fps)),
            bit_depth: 8,
            chroma_sampling: ChromaSampling::Cs420,
            pixel_range: PixelRange::Limited,
            color_description: Some(ColorDescription {
                color_primaries: ColorPrimaries::BT709,
                transfer_characteristics: TransferCharacteristics::BT709,
                matrix_coefficients: MatrixCoefficients::BT709,
            }),
            // A key frame at least every 2 s, so a player can seek.
            max_key_frame_interval: u64::from(fps) * 2,
            // Good for flat UI colours and sharp edges; lower is better.
            quantizer: 70,
            // The fastest preset: exports take seconds, not minutes.
            speed_settings: SpeedSettings::from_preset(10),
            ..Default::default()
        };
        let ctx = Config::new()
            .with_encoder_config(enc)
            .new_context()
            .map_err(av1_error)?;
        Ok(Av1 { ctx, size })
    }

    /// The `av1C` record both containers carry (no config OBUs: key frames hold the header).
    pub fn config_record(&self) -> Vec<u8> {
        self.ctx.container_sequence_header()
    }

    /// Adds a frame; returns the packets ready so far.
    pub fn push(&mut self, rgba: &[u8]) -> Result<Vec<Packet>, VideoError> {
        self.size.check(rgba)?;
        let (w, h) = (usize::from(self.size.width), usize::from(self.size.height));
        let planes = yuv::from_rgba(rgba, w, h);
        let mut frame = self.ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&planes.y, w, 1);
        frame.planes[1].copy_from_raw_u8(&planes.u, w / 2, 1);
        frame.planes[2].copy_from_raw_u8(&planes.v, w / 2, 1);
        self.ctx.send_frame(frame).map_err(av1_error)?;
        self.drain()
    }

    /// Ends the stream; returns the remaining packets.
    pub fn finish(&mut self) -> Result<Vec<Packet>, VideoError> {
        self.ctx.flush();
        self.drain()
    }

    fn drain(&mut self) -> Result<Vec<Packet>, VideoError> {
        let mut out = Vec::new();
        loop {
            match self.ctx.receive_packet() {
                Ok(p) => out.push(Packet {
                    key: p.frame_type == FrameType::KEY,
                    data: p.data,
                }),
                Err(EncoderStatus::Encoded) => {}
                Err(EncoderStatus::NeedMoreData | EncoderStatus::LimitReached) => return Ok(out),
                Err(e) => return Err(av1_error(e)),
            }
        }
    }
}
