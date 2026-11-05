//! The macOS side of [`crate::os_mp4`]: AVFoundation's AVAssetWriter (VideoToolbox below it).

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::os_mp4::mp4_error;
use crate::{Size, VideoError};

/// Longest the writer may wait to take a frame or to finish the file.
const WAIT: Duration = Duration::from_secs(30);

#[expect(
    unsafe_code,
    reason = "AVFoundation and CoreVideo calls (approved FFI exception, AGENTS.md rule 10)"
)]
pub(crate) fn write(
    path: &Path,
    size: Size,
    fps: u32,
    ready: &Sender<Result<(), VideoError>>,
    frames: &Receiver<Vec<u8>>,
) -> Result<(), VideoError> {
    use std::ptr::NonNull;

    use objc2::runtime::AnyObject;
    use objc2_av_foundation::{
        AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor,
        AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeVideo, AVVideoAverageBitRateKey,
        AVVideoCodecKey, AVVideoCodecTypeH264, AVVideoColorPrimaries_ITU_R_709_2,
        AVVideoColorPrimariesKey, AVVideoColorPropertiesKey, AVVideoCompressionPropertiesKey,
        AVVideoHeightKey, AVVideoMaxKeyFrameIntervalKey, AVVideoTransferFunction_ITU_R_709_2,
        AVVideoTransferFunctionKey, AVVideoWidthKey, AVVideoYCbCrMatrix_ITU_R_709_2,
        AVVideoYCbCrMatrixKey,
    };
    use objc2_core_foundation::CFRetained;
    use objc2_core_media::{CMTime, CMTimeFlags};
    use objc2_core_video::{
        CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress,
        CVPixelBufferGetBytesPerRow, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
        CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
    };
    use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

    let setup = || -> Result<_, VideoError> {
        // SAFETY: the AVFoundation key and type constants are immutable framework statics,
        // read once here; a missing one is an error, not a null dereference.
        let key =
            |k: Option<&'static NSString>| k.ok_or_else(|| mp4_error("a missing AVFoundation key"));
        let (codec_key, h264, width_key, height_key) = unsafe {
            (
                key(AVVideoCodecKey)?,
                key(AVVideoCodecTypeH264)?,
                key(AVVideoWidthKey)?,
                key(AVVideoHeightKey)?,
            )
        };
        let (compression_key, bitrate_key, keyframe_key) = unsafe {
            (
                key(AVVideoCompressionPropertiesKey)?,
                key(AVVideoAverageBitRateKey)?,
                key(AVVideoMaxKeyFrameIntervalKey)?,
            )
        };
        let (color_key, primaries_key, transfer_key, matrix_key, bt709p, bt709t, bt709m) = unsafe {
            (
                key(AVVideoColorPropertiesKey)?,
                key(AVVideoColorPrimariesKey)?,
                key(AVVideoTransferFunctionKey)?,
                key(AVVideoYCbCrMatrixKey)?,
                key(AVVideoColorPrimaries_ITU_R_709_2)?,
                key(AVVideoTransferFunction_ITU_R_709_2)?,
                key(AVVideoYCbCrMatrix_ITU_R_709_2)?,
            )
        };
        let (file_type, media_type) = unsafe {
            (
                AVFileTypeMPEG4.ok_or_else(|| mp4_error("no MPEG-4 file type"))?,
                AVMediaTypeVideo.ok_or_else(|| mp4_error("no video media type"))?,
            )
        };
        // About 0.2 bits a pixel: crisp for flat UI colours and text.
        let pixels = u64::from(size.width) * u64::from(size.height) * u64::from(fps);
        let bitrate =
            NSNumber::numberWithInteger((pixels / 5).clamp(1_000_000, 50_000_000) as isize);
        let interval = NSNumber::numberWithInteger(fps as isize * 2);
        let compression: objc2::rc::Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(
                &[bitrate_key, keyframe_key],
                &[bitrate.as_ref(), interval.as_ref()],
            );
        let color: objc2::rc::Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(
                &[primaries_key, transfer_key, matrix_key],
                &[bt709p.as_ref(), bt709t.as_ref(), bt709m.as_ref()],
            );
        let width = NSNumber::numberWithInteger(size.width as isize);
        let height = NSNumber::numberWithInteger(size.height as isize);
        let settings: objc2::rc::Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(
                &[codec_key, width_key, height_key, compression_key, color_key],
                &[
                    h264.as_ref(),
                    width.as_ref(),
                    height.as_ref(),
                    compression.as_ref(),
                    color.as_ref(),
                ],
            );
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: the URL is a file URL to a path that does not exist yet, the file type and
        // media type are AVFoundation's own constants, and `settings` holds only the
        // documented video keys with values of their documented types (NSString, NSNumber,
        // NSDictionary). Every object is retained for as long as it is used.
        unsafe {
            let writer = AVAssetWriter::assetWriterWithURL_fileType_error(&url, file_type)
                .map_err(|e| mp4_error(e.localizedDescription()))?;
            let input = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                media_type,
                Some(&settings),
            );
            input.setExpectsMediaDataInRealTime(false);
            // The movie box first, so a player starts before the whole file has arrived.
            writer.setShouldOptimizeForNetworkUse(true);
            if !writer.canAddInput(&input) {
                return Err(mp4_error("the writer refused the H.264 settings"));
            }
            writer.addInput(&input);
            let adaptor =
                AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(&input, None);
            if !writer.startWriting() {
                let why = writer.error().map(|e| e.localizedDescription().to_string());
                return Err(mp4_error(why.unwrap_or_else(|| "it did not start".into())));
            }
            let zero = CMTime {
                value: 0,
                timescale: fps as i32,
                flags: CMTimeFlags::Valid,
                epoch: 0,
            };
            writer.startSessionAtSourceTime(zero);
            Ok((writer, input, adaptor))
        }
    };
    let (writer, input, adaptor) = match setup() {
        Ok(parts) => {
            let _ = ready.send(Ok(()));
            parts
        }
        Err(e) => {
            let _ = ready.send(Err(mp4_error(&e)));
            return Err(e);
        }
    };
    let failed = |what: &str| {
        // SAFETY: reading the writer's error object, retained by the writer.
        let why = unsafe { writer.error() }.map(|e| e.localizedDescription().to_string());
        mp4_error(format!(
            "{what}: {}",
            why.unwrap_or_else(|| "no reason given".into())
        ))
    };
    let (w, h) = (usize::from(size.width), usize::from(size.height));
    let mut index = 0i64;
    while let Ok(rgba) = frames.recv() {
        let started = Instant::now();
        // SAFETY: a property read on the retained input.
        while !unsafe { input.isReadyForMoreMediaData() } {
            if started.elapsed() > WAIT {
                return Err(failed("the encoder stopped taking frames"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut raw: *mut CVPixelBuffer = std::ptr::null_mut();
        // SAFETY: `raw` is a valid place for the out pointer; on success CoreVideo returns a
        // buffer we own (+1), wrapped at once so it is released.
        let buffer = unsafe {
            let status = CVPixelBufferCreate(
                None,
                w,
                h,
                kCVPixelFormatType_32BGRA,
                None,
                NonNull::from(&mut raw),
            );
            if status != kCVReturnSuccess {
                return Err(mp4_error(format!("no pixel buffer ({status})")));
            }
            CFRetained::from_raw(NonNull::new(raw).ok_or_else(|| mp4_error("no pixel buffer"))?)
        };
        // SAFETY: the buffer is locked while its memory is written: `bytes_per_row × h`
        // bytes from its base address, of which each row's first `w × 4` are the pixels. The
        // slices end before the unlock.
        unsafe {
            if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags(0)) != kCVReturnSuccess
            {
                return Err(mp4_error("could not lock a pixel buffer"));
            }
            let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRow(&buffer);
            if !base.is_null() && stride >= w * 4 {
                for y in 0..h {
                    let row = std::slice::from_raw_parts_mut(base.add(y * stride), w * 4);
                    for (dst, src) in row
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(rgba[y * w * 4..].as_chunks::<4>().0)
                    {
                        dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
                    }
                }
            }
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags(0));
        }
        let at = CMTime {
            value: index,
            timescale: fps as i32,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        };
        // SAFETY: appending a filled, unlocked buffer of the session's size at its time.
        if !unsafe { adaptor.appendPixelBuffer_withPresentationTime(&buffer, at) } {
            return Err(failed("a frame was refused"));
        }
        index += 1;
    }
    if index == 0 {
        return Err(mp4_error("no frames"));
    }
    let (done_tx, done) = channel();
    let block = block2::RcBlock::new(move || {
        let _ = done_tx.send(());
    });
    // SAFETY: the input is finished before the writer; the completion block only sends on a
    // channel (it is Send) and is kept alive until it has run or the wait gives up.
    unsafe {
        input.markAsFinished();
        writer.finishWritingWithCompletionHandler(&block);
    }
    if done.recv_timeout(WAIT).is_err() {
        return Err(mp4_error("the file did not finish"));
    }
    // SAFETY: a property read on the retained writer.
    if unsafe { writer.status() } != AVAssetWriterStatus::Completed {
        return Err(failed("the file was not written"));
    }
    Ok(())
}
