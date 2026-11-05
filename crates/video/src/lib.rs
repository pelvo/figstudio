//! Encodes the frames of an animation as a file: an animated GIF, a zip of PNG frames, WebM
//! (AV1) or MP4. MP4 is H.264 from the operating system's encoder where there is one (macOS
//! AVFoundation, Windows Media Foundation), so it opens everywhere, and AV1 elsewhere.
//!
//! Frames are RGBA8 rows, `width × height`, pushed one at a time ([`Encoder::push`]), so a long
//! animation never holds all its frames. Video formats ignore alpha (draw the frames on their
//! background) and need even sides ([`frame_size`]).

#[cfg(feature = "av1")]
mod av1;
mod frames_zip;
mod gif;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(feature = "av1")]
mod mp4;
#[cfg(any(target_os = "macos", windows))]
mod os_mp4;
#[cfg(feature = "av1")]
mod webm;
#[cfg(windows)]
mod win;
#[cfg(feature = "av1")]
mod yuv;

use std::fmt;

/// A file format to export an animation as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// Animated GIF, looping (256 colours a frame).
    Gif,
    /// A zip of numbered PNG frames, lossless.
    Frames,
    /// WebM with AV1 video.
    Webm,
    /// MP4: H.264 from the operating system, else AV1.
    Mp4,
}

impl Format {
    /// Every format, in menu order.
    pub const ALL: [Format; 4] = [Format::Mp4, Format::Webm, Format::Gif, Format::Frames];

    /// The file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Gif => "gif",
            Format::Frames => "zip",
            Format::Webm => "webm",
            Format::Mp4 => "mp4",
        }
    }

    /// The format a file name's extension asks for.
    pub fn from_extension(ext: &str) -> Option<Format> {
        Format::ALL
            .into_iter()
            .find(|f| f.extension().eq_ignore_ascii_case(ext))
    }

    /// Its MIME type.
    pub fn mime(self) -> &'static str {
        match self {
            Format::Gif => "image/gif",
            Format::Frames => "application/zip",
            Format::Webm => "video/webm",
            Format::Mp4 => "video/mp4",
        }
    }

    /// A short name for menus.
    pub fn label(self) -> &'static str {
        match self {
            Format::Gif => "GIF",
            Format::Frames => "PNG frames (zip)",
            Format::Webm => "WebM video",
            Format::Mp4 => "MP4 video",
        }
    }

    /// Whether this build can write it.
    pub fn available(self) -> bool {
        match self {
            Format::Gif | Format::Frames => true,
            Format::Webm | Format::Mp4 => cfg!(feature = "av1"),
        }
    }

    fn is_video(self) -> bool {
        matches!(self, Format::Webm | Format::Mp4)
    }
}

/// Why an export failed.
#[derive(Debug)]
pub struct VideoError {
    message: String,
}

impl VideoError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        VideoError {
            message: message.into(),
        }
    }
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VideoError {}

/// Largest side of a frame.
pub const MAX_SIDE: u16 = 3840;

/// Frames to export per second, within reason.
pub const FPS_RANGE: std::ops::RangeInclusive<u32> = 1..=60;

/// The frame size `format` takes for a picture of `width × height`: at most [`MAX_SIDE`], and
/// even sides for video (4:2:0 chroma). Draw the frames at this size.
pub fn frame_size(format: Format, width: u32, height: u32) -> (u16, u16) {
    let fit = |v: u32| v.clamp(2, u32::from(MAX_SIDE));
    let (mut w, mut h) = (fit(width), fit(height));
    if format.is_video() {
        w -= w % 2;
        h -= h % 2;
    }
    (w as u16, h as u16)
}

/// Takes frames and gives the file.
pub trait Encoder: Send {
    /// Adds the next frame: `width × height` RGBA8, rows top to bottom.
    fn push(&mut self, rgba: &[u8]) -> Result<(), VideoError>;
    /// Ends the animation and returns the file's bytes.
    fn finish(self: Box<Self>) -> Result<Vec<u8>, VideoError>;
}

/// An encoder writing `format`, `width × height` (see [`frame_size`]) at `fps` frames a second.
pub fn encoder(
    format: Format,
    width: u16,
    height: u16,
    fps: u32,
) -> Result<Box<dyn Encoder>, VideoError> {
    if !FPS_RANGE.contains(&fps) {
        return Err(VideoError::new(format!(
            "frames per second must be between {} and {}",
            FPS_RANGE.start(),
            FPS_RANGE.end()
        )));
    }
    if frame_size(format, u32::from(width), u32::from(height)) != (width, height) {
        return Err(VideoError::new(format!(
            "{width}×{height} is not a frame size {} can take",
            format.label()
        )));
    }
    let size = Size { width, height };
    match format {
        Format::Gif => Ok(Box::new(gif::Gif::new(size, fps))),
        Format::Frames => Ok(Box::new(frames_zip::FramesZip::new(size))),
        #[cfg(feature = "av1")]
        Format::Webm => Ok(Box::new(webm::Webm::new(size, fps)?)),
        Format::Mp4 => mp4_encoder(size, fps),
        #[cfg(not(feature = "av1"))]
        Format::Webm => Err(VideoError::new(format!(
            "{} is not available in this build",
            format.label()
        ))),
    }
}

/// Largest picture the OS H.264 encoders take (H.264 level 5.2: 4096 × 2304).
const MAX_H264_PIXELS: u32 = 4096 * 2304;

/// MP4: H.264 from the operating system where it has an encoder that takes this size, else
/// AV1.
fn mp4_encoder(size: Size, fps: u32) -> Result<Box<dyn Encoder>, VideoError> {
    let fits = u32::from(size.width) * u32::from(size.height) <= MAX_H264_PIXELS;
    #[cfg(any(target_os = "macos", windows))]
    if fits {
        match os_mp4::OsMp4::new(size, fps) {
            Ok(e) => return Ok(Box::new(e)),
            Err(e) => tracing::warn!("H.264 encoder unavailable, using AV1: {e}"),
        }
    }
    let _ = (fits, fps);
    #[cfg(feature = "av1")]
    {
        Ok(Box::new(mp4::Av1Mp4::new(size, fps)?))
    }
    #[cfg(not(feature = "av1"))]
    {
        Err(VideoError::new("MP4 is not available in this build"))
    }
}

/// A frame size, checked by [`encoder`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Size {
    pub width: u16,
    pub height: u16,
}

impl Size {
    /// Bytes of one RGBA8 frame.
    pub fn rgba_len(self) -> usize {
        usize::from(self.width) * usize::from(self.height) * 4
    }

    /// An error unless `rgba` is one frame of this size.
    pub fn check(self, rgba: &[u8]) -> Result<(), VideoError> {
        if rgba.len() == self.rgba_len() {
            Ok(())
        } else {
            Err(VideoError::new(format!(
                "a frame of {} bytes, not {}",
                rgba.len(),
                self.rgba_len()
            )))
        }
    }
}
