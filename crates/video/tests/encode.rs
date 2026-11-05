//! Each format comes back with the frames it was given: counted, sized and (for the
//! lossless ones) pixel for pixel.

use std::io::{Cursor, Read};

use figstudio_video::{Format, encoder, frame_size};

const W: u16 = 64;
const H: u16 = 48;
const FRAMES: usize = 12;
const FPS: u32 = 12;

/// Frame `i`: grey, with a red square moving right.
fn frame(i: usize) -> Vec<u8> {
    let mut rgba = vec![0; usize::from(W) * usize::from(H) * 4];
    for (n, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (n % usize::from(W), n / usize::from(W));
        let inside = (i * 4..i * 4 + 12).contains(&x) && (18..30).contains(&y);
        px.copy_from_slice(if inside {
            &[230, 30, 30, 255]
        } else {
            &[200, 200, 200, 255]
        });
    }
    rgba
}

fn encode(format: Format) -> Vec<u8> {
    let mut e = encoder(format, W, H, FPS).unwrap();
    for i in 0..FRAMES {
        e.push(&frame(i)).unwrap();
    }
    e.finish().unwrap()
}

#[test]
fn gif_has_every_frame_and_loops() {
    use image::AnimationDecoder;
    let bytes = encode(Format::Gif);
    assert!(bytes.starts_with(b"GIF89a"));
    // NETSCAPE2.0 is the loop extension.
    assert!(bytes.windows(11).any(|w| w == b"NETSCAPE2.0"));
    let frames = image::codecs::gif::GifDecoder::new(Cursor::new(&bytes))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
    assert_eq!(frames.len(), FRAMES);
    let last = frames[FRAMES - 1].buffer();
    assert_eq!(last.dimensions(), (u32::from(W), u32::from(H)));
    // The square has moved: the last frame differs from the first.
    assert_ne!(frames[0].buffer().as_raw(), last.as_raw());
    // 12 frames at 12 fps last 1 s, whatever the rounding to hundredths.
    let total: u32 = frames
        .iter()
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            n / d
        })
        .sum();
    assert_eq!(total, 1000);
}

#[test]
fn png_frames_are_numbered_and_lossless() {
    let bytes = encode(Format::Frames);
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), FRAMES);
    for i in 0..FRAMES {
        let mut png = Vec::new();
        zip.by_name(&format!("frame_{:05}.png", i + 1))
            .unwrap()
            .read_to_end(&mut png)
            .unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.into_raw(), frame(i), "frame {i}");
    }
}

#[test]
fn webm_holds_one_av1_block_per_frame() {
    use webm_iterable::WebmIterator;
    use webm_iterable::matroska_spec::MatroskaSpec;
    let bytes = encode(Format::Webm);
    let (mut blocks, mut codec, mut size, mut keys) = (0, String::new(), (0, 0), 0);
    for tag in WebmIterator::new(Cursor::new(&bytes), &[]) {
        match tag.unwrap() {
            MatroskaSpec::SimpleBlock(data) => {
                blocks += 1;
                // Flags byte after the track number (1 byte) and the 16-bit time.
                if data[3] & 0x80 != 0 {
                    keys += 1;
                }
            }
            MatroskaSpec::CodecID(id) => codec = id,
            MatroskaSpec::PixelWidth(w) => size.0 = w,
            MatroskaSpec::PixelHeight(h) => size.1 = h,
            _ => {}
        }
    }
    assert_eq!(codec, "V_AV1");
    assert_eq!(size, (u64::from(W), u64::from(H)));
    assert_eq!(blocks, FRAMES);
    assert!(keys >= 1, "the first frame is a key frame");
}

#[test]
fn mp4_holds_one_sample_per_frame_with_the_movie_box_first() {
    use mp4_atom::{Any, DecodeMaybe, Moov};
    let bytes = encode(Format::Mp4);
    let mut buf = bytes.as_slice();
    let mut order = Vec::new();
    let mut moov = None;
    while let Some(atom) = Any::decode_maybe(&mut buf).unwrap() {
        match atom {
            Any::Ftyp(_) => order.push("ftyp"),
            Any::Moov(m) => {
                order.push("moov");
                moov = Some(m);
            }
            Any::Mdat(_) => order.push("mdat"),
            _ => {}
        }
    }
    assert_eq!(order, ["ftyp", "moov", "mdat"]);
    let moov: Moov = moov.unwrap();
    let stbl = &moov.trak[0].mdia.minf.stbl;
    // H.264 from the OS where it has an encoder (macOS here), else AV1.
    if cfg!(target_os = "macos") {
        assert!(
            matches!(stbl.stsd.codecs[0], mp4_atom::Codec::Avc1(_)),
            "{:?}",
            stbl.stsd.codecs[0]
        );
    } else {
        assert!(matches!(stbl.stsd.codecs[0], mp4_atom::Codec::Av01(_)));
    }
    let mp4_atom::StszSamples::Different { sizes } = &stbl.stsz.samples else {
        panic!("sizes");
    };
    assert_eq!(sizes.len(), FRAMES);
    // Every chunk lies inside the file, and the samples fit in it.
    let offsets = &stbl.stco.as_ref().unwrap().entries;
    let total: usize = sizes.iter().map(|s| *s as usize).sum();
    assert!(offsets.iter().all(|o| (*o as usize) < bytes.len()));
    assert!(offsets[0] as usize + total <= bytes.len());
    // 12 frames at 12 fps last 1 s, whatever the timescale.
    let mdhd = &moov.trak[0].mdia.mdhd;
    assert_eq!(mdhd.duration * 1000 / u64::from(mdhd.timescale), 1000);
}

#[test]
fn video_sizes_are_even_and_wrong_frames_are_refused() {
    assert_eq!(frame_size(Format::Mp4, 101, 51), (100, 50));
    assert_eq!(frame_size(Format::Gif, 101, 51), (101, 51));
    assert!(encoder(Format::Mp4, 101, 50, 30).is_err(), "odd width");
    assert!(encoder(Format::Gif, 64, 48, 0).is_err(), "0 fps");
    let mut e = encoder(Format::Gif, W, H, FPS).unwrap();
    assert!(e.push(&[0; 10]).is_err(), "a frame of the wrong size");
}

#[test]
fn a_30_fps_gif_keeps_its_length() {
    use image::AnimationDecoder;
    let mut e = encoder(Format::Gif, W, H, 30).unwrap();
    for i in 0..30 {
        e.push(&frame(i % FRAMES)).unwrap();
    }
    let bytes = e.finish().unwrap();
    let total: u32 = image::codecs::gif::GifDecoder::new(Cursor::new(&bytes))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap()
        .iter()
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            n / d
        })
        .sum();
    assert_eq!(total, 1000, "30 frames at 30 fps");
}
