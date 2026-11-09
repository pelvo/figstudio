//! MP4 with AV1 video (`av01`), for where there is no H.264 encoder: the boxes come from
//! `mp4-atom`; the movie box goes before the data so a player can start at once.

use mp4_atom::{
    Atom, Av01, Av1c, Codec, Dinf, Dref, DrefEntry, Encode, FourCC, Ftyp, Hdlr, Mdat, Mdhd, Mdia,
    Minf, Moov, Mvhd, Stbl, Stco, Stsc, StscEntry, Stsd, Stss, Stsz, StszSamples, Stts, SttsEntry,
    Tkhd, Trak, Url, Visual, Vmhd,
};

use crate::av1::{Av1, Packet};
use crate::{Encoder, Size, VideoError};

pub(crate) struct Av1Mp4 {
    av1: Av1,
    size: Size,
    fps: u32,
    packets: Vec<Packet>,
}

impl Av1Mp4 {
    pub fn new(size: Size, fps: u32) -> Result<Self, VideoError> {
        Ok(Av1Mp4 {
            av1: Av1::new(size, fps)?,
            size,
            fps,
            packets: Vec::new(),
        })
    }
}

fn mp4_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::new(format!("MP4: {e}"))
}

impl Encoder for Av1Mp4 {
    fn push(&mut self, rgba: &[u8]) -> Result<(), VideoError> {
        let ready = self.av1.push(rgba)?;
        self.packets.extend(ready);
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Vec<u8>, VideoError> {
        let rest = self.av1.finish()?;
        self.packets.extend(rest);
        if self.packets.is_empty() {
            return Err(mp4_error("no frames"));
        }
        let record = self.av1.config_record();
        let av1c = Av1c::decode_body(&mut record.as_slice()).map_err(mp4_error)?;
        mux(
            Codec::Av01(Av01 {
                visual: visual(self.size),
                av1c,
                btrt: None,
                ccst: None,
                colr: None,
                pasp: None,
                taic: None,
                spatial: None,
            }),
            self.size,
            self.fps,
            &self.packets,
        )
    }
}

fn visual(size: Size) -> Visual {
    Visual {
        data_reference_index: 1,
        width: size.width,
        height: size.height,
        ..Default::default()
    }
}

/// One video track of `packets` (one sample each, `fps` a second), all in one chunk.
pub(crate) fn mux(
    codec: Codec,
    size: Size,
    fps: u32,
    packets: &[Packet],
) -> Result<Vec<u8>, VideoError> {
    let count = u32::try_from(packets.len()).map_err(mp4_error)?;
    // Timescale = fps: each frame lasts 1 unit.
    let duration = u64::from(count);
    let movie_duration = duration * 1000 / u64::from(fps);
    let sizes = packets
        .iter()
        .map(|p| u32::try_from(p.data.len()).map_err(mp4_error))
        .collect::<Result<Vec<_>, _>>()?;
    let keys: Vec<u32> = (1..=count)
        .zip(packets)
        .filter(|(_, p)| p.key)
        .map(|(i, _)| i)
        .collect();
    let moov = |offset: u32| Moov {
        mvhd: Mvhd {
            timescale: 1000,
            duration: movie_duration,
            next_track_id: 2,
            ..Default::default()
        },
        trak: vec![Trak {
            tkhd: Tkhd {
                track_id: 1,
                duration: movie_duration,
                enabled: true,
                in_movie: true,
                width: size.width.into(),
                height: size.height.into(),
                ..Default::default()
            },
            edts: None,
            meta: None,
            mdia: Mdia {
                mdhd: Mdhd {
                    timescale: fps,
                    duration,
                    language: "und".into(),
                    ..Default::default()
                },
                hdlr: Hdlr {
                    handler: FourCC::new(b"vide"),
                    name: "figstudio".into(),
                },
                minf: Minf {
                    vmhd: Some(Vmhd::default()),
                    dinf: Dinf {
                        dref: Dref {
                            entries: vec![DrefEntry::Url(Url::default())],
                        },
                    },
                    stbl: Stbl {
                        stsd: Stsd {
                            codecs: vec![codec.clone()],
                        },
                        stts: Stts {
                            entries: vec![SttsEntry {
                                sample_count: count,
                                sample_delta: 1,
                            }],
                        },
                        stss: Some(Stss {
                            entries: keys.clone(),
                        }),
                        stsc: Stsc {
                            entries: vec![StscEntry {
                                first_chunk: 1,
                                samples_per_chunk: count,
                                sample_description_index: 1,
                            }],
                        },
                        stsz: Stsz {
                            samples: StszSamples::Different {
                                sizes: sizes.clone(),
                            },
                        },
                        stco: Some(Stco {
                            entries: vec![offset],
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
            senc: None,
            tref: None,
            udta: None,
        }],
        ..Default::default()
    };
    let mut head = Vec::new();
    Ftyp {
        major_brand: FourCC::new(b"isom"),
        minor_version: 512,
        compatible_brands: vec![
            FourCC::new(b"isom"),
            FourCC::new(b"iso6"),
            FourCC::new(b"av01"),
            FourCC::new(b"avc1"),
            FourCC::new(b"mp41"),
        ],
    }
    .encode(&mut head)
    .map_err(mp4_error)?;
    // The chunk's offset depends on the movie box's size, which does not depend on it.
    let mut probe = Vec::new();
    moov(0).encode(&mut probe).map_err(mp4_error)?;
    let data: Vec<u8> = packets
        .iter()
        .flat_map(|p| p.data.iter().copied())
        .collect();
    let mut mdat_header = Vec::new();
    Mdat { data: Vec::new() }
        .encode(&mut mdat_header)
        .map_err(mp4_error)?;
    let offset = u32::try_from(head.len() + probe.len() + mdat_header.len()).map_err(mp4_error)?;
    moov(offset).encode(&mut head).map_err(mp4_error)?;
    Mdat { data }.encode(&mut head).map_err(mp4_error)?;
    Ok(head)
}

#[cfg(test)]
mod tests {
    use mp4_atom::{Any, DecodeMaybe};

    use super::*;

    #[test]
    fn av1_mp4_has_one_sample_per_frame_and_the_movie_box_first() {
        let size = Size {
            width: 32,
            height: 16,
        };
        let mut e = Box::new(Av1Mp4::new(size, 10).unwrap());
        for i in 0..10u8 {
            e.push(&vec![i * 20; size.rgba_len()]).unwrap();
        }
        let bytes = e.finish().unwrap();
        let mut buf = bytes.as_slice();
        let mut kinds = Vec::new();
        let mut moov = None;
        while let Some(atom) = Any::decode_maybe(&mut buf).unwrap() {
            match atom {
                Any::Ftyp(_) => kinds.push("ftyp"),
                Any::Moov(m) => {
                    kinds.push("moov");
                    moov = Some(m);
                }
                Any::Mdat(_) => kinds.push("mdat"),
                _ => {}
            }
        }
        assert_eq!(kinds, ["ftyp", "moov", "mdat"]);
        let stbl = &moov.unwrap().trak[0].mdia.minf.stbl;
        assert!(matches!(stbl.stsd.codecs[0], Codec::Av01(_)));
        let StszSamples::Different { sizes } = &stbl.stsz.samples else {
            panic!("sizes");
        };
        assert_eq!(sizes.len(), 10);
        let offset = stbl.stco.as_ref().unwrap().entries[0] as usize;
        let total: usize = sizes.iter().map(|s| *s as usize).sum();
        assert_eq!(offset + total, bytes.len(), "the data ends the file");
    }
}
