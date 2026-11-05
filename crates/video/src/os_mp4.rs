//! MP4 with H.264 from the operating system's own encoder (macOS AVFoundation in `mac.rs`,
//! Windows Media Foundation in `win.rs`): licensed by the OS and opened by every player,
//! QuickTime, Keynote and PowerPoint included.
//!
//! The OS writer lives on its own thread (its OS objects never move between threads); frames
//! reach it over a bounded channel, so a slow encoder slows the export, not memory. It writes
//! a temporary file, read back and removed when it is done.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::thread::JoinHandle;

use crate::{Encoder, Size, VideoError};

pub(crate) struct OsMp4 {
    size: Size,
    frames: Option<SyncSender<Vec<u8>>>,
    done: Option<JoinHandle<Result<Vec<u8>, VideoError>>>,
}

pub(crate) fn mp4_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::new(format!("MP4 (H.264): {e}"))
}

impl OsMp4 {
    /// Starts the writer; an error (no H.264 encoder here, or a size it refuses) means use AV1.
    pub fn new(size: Size, fps: u32) -> Result<Self, VideoError> {
        let (frames, rx) = sync_channel::<Vec<u8>>(4);
        let (ready_tx, ready) = channel();
        let done = std::thread::Builder::new()
            .name("mp4-h264".into())
            .spawn(move || session(size, fps, &ready_tx, &rx))
            .map_err(mp4_error)?;
        match ready.recv() {
            Ok(Ok(())) => Ok(OsMp4 {
                size,
                frames: Some(frames),
                done: Some(done),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(done
                .join()
                .ok()
                .and_then(Result::err)
                .unwrap_or_else(|| mp4_error("the writer did not start"))),
        }
    }

    /// The writer thread's error, once it has stopped.
    fn stopped(&mut self) -> VideoError {
        self.frames = None;
        self.done
            .take()
            .and_then(|d| d.join().ok())
            .and_then(Result::err)
            .unwrap_or_else(|| mp4_error("the writer stopped"))
    }
}

impl Encoder for OsMp4 {
    fn push(&mut self, rgba: &[u8]) -> Result<(), VideoError> {
        self.size.check(rgba)?;
        let sent = self
            .frames
            .as_ref()
            .is_some_and(|f| f.send(rgba.to_vec()).is_ok());
        if sent { Ok(()) } else { Err(self.stopped()) }
    }

    fn finish(mut self: Box<Self>) -> Result<Vec<u8>, VideoError> {
        // Closing the channel tells the writer to finish the file.
        self.frames = None;
        self.done
            .take()
            .ok_or_else(|| mp4_error("finished twice"))?
            .join()
            .map_err(|_| mp4_error("the writer thread panicked"))?
    }
}

/// A file name in the temporary folder no other export uses (AVAssetWriter writes to a file
/// that must not exist yet).
fn temp_path() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("figstudio-export-{}-{n}.mp4", std::process::id()))
}

/// The writer thread: sets up, says so on `ready`, writes every frame from `frames`, then
/// returns the file's bytes.
fn session(
    size: Size,
    fps: u32,
    ready: &std::sync::mpsc::Sender<Result<(), VideoError>>,
    frames: &Receiver<Vec<u8>>,
) -> Result<Vec<u8>, VideoError> {
    let path = temp_path();
    let _ = std::fs::remove_file(&path);
    #[cfg(target_os = "macos")]
    let result = crate::mac::write(&path, size, fps, ready, frames);
    #[cfg(windows)]
    let result = crate::win::write(&path, size, fps, ready, frames);
    let bytes = result.and_then(|()| std::fs::read(&path).map_err(mp4_error));
    let _ = std::fs::remove_file(&path);
    bytes
}
