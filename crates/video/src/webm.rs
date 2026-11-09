//! WebM: AV1 packets in a Matroska file (`webm-iterable` writes the tags).

use webm_iterable::WebmWriter;
use webm_iterable::matroska_spec::{Master, MatroskaSpec, SimpleBlock};

use crate::av1::{Av1, Packet};
use crate::{Encoder, Size, VideoError};

/// A cluster's block times are 16-bit milliseconds from its start: a new cluster every 10 s.
const CLUSTER_MS: u64 = 10_000;

pub(crate) struct Webm {
    av1: Av1,
    size: Size,
    fps: u32,
    packets: Vec<Packet>,
}

impl Webm {
    pub fn new(size: Size, fps: u32) -> Result<Self, VideoError> {
        Ok(Webm {
            av1: Av1::new(size, fps)?,
            size,
            fps,
            packets: Vec::new(),
        })
    }
}

fn webm_error(e: impl std::fmt::Display) -> VideoError {
    VideoError::new(format!("WebM: {e}"))
}

impl Encoder for Webm {
    fn push(&mut self, rgba: &[u8]) -> Result<(), VideoError> {
        let ready = self.av1.push(rgba)?;
        self.packets.extend(ready);
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Vec<u8>, VideoError> {
        let rest = self.av1.finish()?;
        self.packets.extend(rest);
        if self.packets.is_empty() {
            return Err(webm_error("no frames"));
        }
        let ms = |i: usize| i as u64 * 1000 / u64::from(self.fps);
        let mut out = Vec::new();
        let mut w = WebmWriter::new(&mut out);
        let ebml = MatroskaSpec::Ebml(Master::Full(vec![
            MatroskaSpec::EbmlVersion(1),
            MatroskaSpec::EbmlReadVersion(1),
            MatroskaSpec::EbmlMaxIdLength(4),
            MatroskaSpec::EbmlMaxSizeLength(8),
            MatroskaSpec::DocType("webm".into()),
            MatroskaSpec::DocTypeVersion(4),
            MatroskaSpec::DocTypeReadVersion(2),
        ]));
        w.write(&ebml).map_err(webm_error)?;
        w.write(&MatroskaSpec::Segment(Master::Start))
            .map_err(webm_error)?;
        let info = MatroskaSpec::Info(Master::Full(vec![
            MatroskaSpec::TimestampScale(1_000_000),
            MatroskaSpec::MuxingApp("figstudio".into()),
            MatroskaSpec::WritingApp("figstudio".into()),
            MatroskaSpec::Duration(ms(self.packets.len()) as f64),
        ]));
        w.write(&info).map_err(webm_error)?;
        let tracks = MatroskaSpec::Tracks(Master::Full(vec![MatroskaSpec::TrackEntry(
            Master::Full(vec![
                MatroskaSpec::TrackNumber(1),
                MatroskaSpec::TrackUID(1),
                MatroskaSpec::TrackType(1),
                MatroskaSpec::FlagLacing(0),
                MatroskaSpec::CodecID("V_AV1".into()),
                MatroskaSpec::CodecPrivate(self.av1.config_record()),
                MatroskaSpec::DefaultDuration(1_000_000_000 / u64::from(self.fps)),
                MatroskaSpec::Video(Master::Full(vec![
                    MatroskaSpec::PixelWidth(u64::from(self.size.width)),
                    MatroskaSpec::PixelHeight(u64::from(self.size.height)),
                ])),
            ]),
        )]));
        w.write(&tracks).map_err(webm_error)?;
        let mut cluster_start: Option<u64> = None;
        for (i, p) in self.packets.iter().enumerate() {
            let t = ms(i);
            if cluster_start.is_none_or(|s| t - s >= CLUSTER_MS) {
                if cluster_start.is_some() {
                    w.write(&MatroskaSpec::Cluster(Master::End))
                        .map_err(webm_error)?;
                }
                w.write(&MatroskaSpec::Cluster(Master::Start))
                    .map_err(webm_error)?;
                w.write(&MatroskaSpec::Timestamp(t)).map_err(webm_error)?;
                cluster_start = Some(t);
            }
            let rel = i16::try_from(t - cluster_start.unwrap_or(0)).map_err(webm_error)?;
            let block = SimpleBlock::new_uncheked(&p.data, 1, rel, false, None, false, p.key);
            w.write(&MatroskaSpec::from(block)).map_err(webm_error)?;
        }
        w.write(&MatroskaSpec::Cluster(Master::End))
            .map_err(webm_error)?;
        w.write(&MatroskaSpec::Segment(Master::End))
            .map_err(webm_error)?;
        drop(w);
        Ok(out)
    }
}
