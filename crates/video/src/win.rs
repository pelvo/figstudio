//! The Windows side of [`crate::os_mp4`]: Media Foundation's sink writer with the system's
//! H.264 encoder (hardware where the machine has one).

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};

use crate::os_mp4::mp4_error;
use crate::{Size, VideoError};

/// 100-nanosecond units a second (Media Foundation's clock).
const TICKS: i64 = 10_000_000;

#[expect(
    unsafe_code,
    reason = "Media Foundation and COM calls (approved FFI exception, AGENTS.md rule 10)"
)]
pub(crate) fn write(
    path: &Path,
    size: Size,
    fps: u32,
    ready: &Sender<Result<(), VideoError>>,
    frames: &Receiver<Vec<u8>>,
) -> Result<(), VideoError> {
    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, IMFSinkWriter, MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE,
        MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE,
        MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
        MF_SINK_WRITER_DISABLE_THROTTLING, MF_VERSION, MFCreateAttributes, MFCreateMediaType,
        MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Video,
        MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoFormat_H264, MFVideoFormat_RGB32,
        MFVideoInterlace_Progressive, eAVEncH264VProfile_High,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::core::HSTRING;

    let (w, h) = (u32::from(size.width), u32::from(size.height));
    let pair = |a: u32, b: u32| (u64::from(a) << 32) | u64::from(b);
    let bitrate =
        (u64::from(w) * u64::from(h) * u64::from(fps) / 5).clamp(1_000_000, 50_000_000) as u32;
    let frame_ticks = TICKS / i64::from(fps);

    // SAFETY: COM and Media Foundation are started on this thread (the writer thread, which
    // owns every object below) and shut down at the end of this function, after the last
    // object is dropped: `writer` and the media types are released before `MFShutdown`.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(mp4_error)?;
    }
    // SAFETY: as above; MFStartup pairs with the MFShutdown below.
    let started = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) };
    let mf_started = started.is_ok();
    let result = (|| -> Result<(), VideoError> {
        started.map_err(mp4_error)?;
        // SAFETY: each call gets valid, owned COM objects and GUIDs that live for the call;
        // attribute keys are Media Foundation's own constants with values of their types.
        let setup = || -> windows::core::Result<(IMFSinkWriter, u32)> {
            unsafe {
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 2)?;
                let attrs = attrs.ok_or_else(windows::core::Error::empty)?;
                attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
                // Not a live recording: take frames as fast as they come.
                attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
                let writer =
                    MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attrs)?;
                let out = MFCreateMediaType()?;
                out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
                out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
                out.SetUINT64(&MF_MT_FRAME_SIZE, pair(w, h))?;
                out.SetUINT64(&MF_MT_FRAME_RATE, pair(fps, 1))?;
                out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1))?;
                let stream = writer.AddStream(&out)?;
                let input = MFCreateMediaType()?;
                input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                input.SetUINT64(&MF_MT_FRAME_SIZE, pair(w, h))?;
                input.SetUINT64(&MF_MT_FRAME_RATE, pair(fps, 1))?;
                input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1))?;
                // A positive stride: rows top to bottom (RGB is bottom-up by default).
                input.SetUINT32(&MF_MT_DEFAULT_STRIDE, w * 4)?;
                writer.SetInputMediaType(stream, &input, None)?;
                writer.BeginWriting()?;
                Ok((writer, stream))
            }
        };
        let (writer, stream) = match setup() {
            Ok(parts) => {
                let _ = ready.send(Ok(()));
                parts
            }
            Err(e) => {
                let _ = ready.send(Err(mp4_error(&e)));
                return Err(mp4_error(e));
            }
        };
        let len = w * h * 4;
        let mut index = 0i64;
        while let Ok(rgba) = frames.recv() {
            // SAFETY: the buffer is created with room for one frame and locked while written:
            // `len` bytes from the pointer Lock returns, ended before Unlock. The sample owns
            // the buffer and the writer copies or encodes it in WriteSample.
            unsafe {
                let buffer = MFCreateMemoryBuffer(len).map_err(mp4_error)?;
                let mut data: *mut u8 = std::ptr::null_mut();
                buffer.Lock(&mut data, None, None).map_err(mp4_error)?;
                if data.is_null() {
                    let _ = buffer.Unlock();
                    return Err(mp4_error("no frame memory"));
                }
                let dst = std::slice::from_raw_parts_mut(data, len as usize);
                for (d, s) in dst
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(rgba.as_chunks::<4>().0)
                {
                    d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                }
                buffer.Unlock().map_err(mp4_error)?;
                buffer.SetCurrentLength(len).map_err(mp4_error)?;
                let sample = MFCreateSample().map_err(mp4_error)?;
                sample.AddBuffer(&buffer).map_err(mp4_error)?;
                sample
                    .SetSampleTime(index * frame_ticks)
                    .map_err(mp4_error)?;
                sample.SetSampleDuration(frame_ticks).map_err(mp4_error)?;
                writer.WriteSample(stream, &sample).map_err(mp4_error)?;
            }
            index += 1;
        }
        if index == 0 {
            return Err(mp4_error("no frames"));
        }
        // SAFETY: finishing the writer this thread created; it is dropped right after.
        unsafe { writer.Finalize() }.map_err(mp4_error)
    })();
    // SAFETY: every Media Foundation object was created and dropped inside the closure above.
    unsafe {
        if mf_started {
            let _ = MFShutdown();
        }
        CoUninitialize();
    }
    result
}
