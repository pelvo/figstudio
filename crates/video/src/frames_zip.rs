//! A zip of PNG frames named `frame_00001.png` onward, for editors that take image sequences.

use std::io::{Cursor, Write};

use image::ImageEncoder;
use image::codecs::png::PngEncoder;
use zip::write::SimpleFileOptions;

use crate::{Encoder, Size, VideoError};

pub(crate) struct FramesZip {
    size: Size,
    zip: zip::ZipWriter<Cursor<Vec<u8>>>,
    frames: usize,
}

impl FramesZip {
    pub fn new(size: Size) -> Self {
        FramesZip {
            size,
            zip: zip::ZipWriter::new(Cursor::new(Vec::new())),
            frames: 0,
        }
    }
}

fn zip_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::new(format!("PNG frames: {e}"))
}

impl Encoder for FramesZip {
    fn push(&mut self, rgba: &[u8]) -> Result<(), VideoError> {
        self.size.check(rgba)?;
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(
                rgba,
                u32::from(self.size.width),
                u32::from(self.size.height),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(zip_error)?;
        self.frames += 1;
        // PNG is compressed already: stored, not deflated again.
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(false);
        self.zip
            .start_file(format!("frame_{:05}.png", self.frames), options)
            .map_err(zip_error)?;
        self.zip.write_all(&png).map_err(zip_error)
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, VideoError> {
        if self.frames == 0 {
            return Err(zip_error("no frames"));
        }
        Ok(self.zip.finish().map_err(zip_error)?.into_inner())
    }
}
