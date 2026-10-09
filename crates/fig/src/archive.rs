//! The zip container of a `.fig`, read piece by piece.
//!
//! [`Document::from_bytes`](crate::Document::from_bytes) needs the whole file in memory. These
//! functions instead work on the few byte ranges that matter (the end of the file, the central
//! directory, one entry), so a caller can open a file far larger than memory allows and fetch
//! images only when they are drawn. The browser app reads ranges from a `File`; the desktop
//! app from disk ([`open_lazy`](crate::Document::open_lazy)).
//!
//! Supported: stored and deflated entries, zip64. Not supported: encryption, split archives.

use std::io::{Read, Seek, SeekFrom};

use crate::doc::FigError;

const EOCD: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const ZIP64_EOCD: u32 = 0x0606_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

/// Bytes at the end of a file that always contain the end-of-central-directory record (and
/// the zip64 records that precede it): the record, the longest comment and the zip64 parts.
pub const TAIL_LEN: u64 = 22 + 65_535 + 20 + 56;

/// Entries [`Document::from_lazy`](crate::Document::from_lazy) needs read up front.
pub const EAGER_ENTRIES: [&str; 3] = ["canvas.fig", "meta.json", "thumbnail.png"];

/// Size of the fixed part of an entry's local header; see [`Entry::data_start`].
pub const LOCAL_HEADER_LEN: u64 = 30;

/// One file inside the archive, as listed by the central directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// 0 stored, 8 deflate.
    pub method: u16,
    pub compressed_size: u64,
    pub size: u64,
    /// Where the entry's local header starts.
    pub header_offset: u64,
}

fn err<T>(msg: impl Into<String>) -> Result<T, FigError> {
    Err(FigError(msg.into()))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Finds the central directory from the last bytes of the file (`tail`, at most [`TAIL_LEN`]
/// of them, ending at `file_len`). Returns its offset and length.
pub fn central_directory(tail: &[u8], file_len: u64) -> Result<(u64, u64), FigError> {
    let tail_start = file_len
        .checked_sub(tail.len() as u64)
        .ok_or_else(|| FigError("tail longer than the file".into()))?;
    let bad = || FigError("not a readable .fig archive: no end of central directory".into());
    // The record is 22 bytes plus a comment; search backwards for a signature whose comment
    // length reaches exactly to the end.
    let at = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&i| {
            u32_at(tail, i) == Some(EOCD)
                && u16_at(tail, i + 20).map(|c| i + 22 + usize::from(c)) == Some(tail.len())
        })
        .ok_or_else(bad)?;
    let disk = u16_at(tail, at + 4).ok_or_else(bad)?;
    let cd_disk = u16_at(tail, at + 6).ok_or_else(bad)?;
    let mut size = u64::from(u32_at(tail, at + 12).ok_or_else(bad)?);
    let mut offset = u64::from(u32_at(tail, at + 16).ok_or_else(bad)?);
    if disk != 0 && disk != u16::MAX || cd_disk != 0 && cd_disk != u16::MAX {
        return err("split .fig archives are not supported");
    }
    if (size == u64::from(u32::MAX) || offset == u64::from(u32::MAX)) && at >= 20 {
        // zip64: the locator sits right before the record and points at the zip64 record.
        let loc = at - 20;
        if u32_at(tail, loc) == Some(ZIP64_LOCATOR) {
            let rec = u64_at(tail, loc + 8).ok_or_else(bad)?;
            let rel = rec
                .checked_sub(tail_start)
                .and_then(|r| usize::try_from(r).ok())
                .filter(|&r| r + 56 <= tail.len())
                .ok_or_else(|| FigError("zip64 record outside the end of the file".into()))?;
            if u32_at(tail, rel) != Some(ZIP64_EOCD) {
                return Err(bad());
            }
            size = u64_at(tail, rel + 40).ok_or_else(bad)?;
            offset = u64_at(tail, rel + 48).ok_or_else(bad)?;
        }
    }
    if offset.checked_add(size).is_none_or(|end| end > file_len) {
        return err("central directory outside the file");
    }
    Ok((offset, size))
}

/// Lists the entries of a central directory (the bytes [`central_directory`] located).
pub fn entries(cd: &[u8]) -> Result<Vec<Entry>, FigError> {
    let bad = |what: &str| FigError(format!("bad central directory: {what}"));
    let mut out = Vec::new();
    let mut at = 0;
    while at + 46 <= cd.len() {
        if u32_at(cd, at) != Some(CENTRAL) {
            break;
        }
        let flags = u16_at(cd, at + 8).ok_or_else(|| bad("flags"))?;
        let method = u16_at(cd, at + 10).ok_or_else(|| bad("method"))?;
        let mut compressed_size = u64::from(u32_at(cd, at + 20).ok_or_else(|| bad("size"))?);
        let mut size = u64::from(u32_at(cd, at + 24).ok_or_else(|| bad("size"))?);
        let name_len = usize::from(u16_at(cd, at + 28).ok_or_else(|| bad("name"))?);
        let extra_len = usize::from(u16_at(cd, at + 30).ok_or_else(|| bad("extra"))?);
        let comment_len = usize::from(u16_at(cd, at + 32).ok_or_else(|| bad("comment"))?);
        let mut header_offset = u64::from(u32_at(cd, at + 42).ok_or_else(|| bad("offset"))?);
        let name_at = at + 46;
        let name = cd
            .get(name_at..name_at + name_len)
            .ok_or_else(|| bad("truncated name"))?;
        let extra = cd
            .get(name_at + name_len..name_at + name_len + extra_len)
            .ok_or_else(|| bad("truncated extra field"))?;
        if flags & 1 != 0 {
            return err("encrypted .fig archives are not supported");
        }
        // zip64 extra field: 64-bit values for the fields set to 0xFFFFFFFF, in this order.
        let mut e = 0;
        while e + 4 <= extra.len() {
            let id = u16_at(extra, e).unwrap_or(0);
            let len = usize::from(u16_at(extra, e + 2).unwrap_or(0));
            let field = extra
                .get(e + 4..e + 4 + len)
                .ok_or_else(|| bad("truncated extra field"))?;
            if id == 1 {
                let mut f = 0;
                for v in [&mut size, &mut compressed_size, &mut header_offset] {
                    if *v == u64::from(u32::MAX) {
                        *v = u64_at(field, f).ok_or_else(|| bad("short zip64 field"))?;
                        f += 8;
                    }
                }
            }
            e += 4 + len;
        }
        out.push(Entry {
            // Figma writes ASCII names; anything else is decoded leniently.
            name: String::from_utf8_lossy(name).into_owned(),
            method,
            compressed_size,
            size,
            header_offset,
        });
        at = name_at + name_len + extra_len + comment_len;
    }
    Ok(out)
}

impl Entry {
    /// Where the entry's data starts, given its local header (at least the first
    /// [`LOCAL_HEADER_LEN`] bytes at `header_offset`).
    pub fn data_start(&self, local_header: &[u8]) -> Result<u64, FigError> {
        if u32_at(local_header, 0) != Some(LOCAL) {
            return err(format!("bad local header for {}", self.name));
        }
        let name = u64::from(u16_at(local_header, 26).unwrap_or(0));
        let extra = u64::from(u16_at(local_header, 28).unwrap_or(0));
        Ok(self.header_offset + LOCAL_HEADER_LEN + name + extra)
    }

    /// Decompresses the entry's data (`compressed_size` bytes from [`Entry::data_start`]).
    pub fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>, FigError> {
        match self.method {
            0 => Ok(data),
            8 => {
                let mut out = Vec::with_capacity(usize::try_from(self.size).unwrap_or(0));
                flate2::read::DeflateDecoder::new(data.as_slice())
                    .read_to_end(&mut out)
                    .map_err(|e| FigError(format!("cannot inflate {}: {e}", self.name)))?;
                Ok(out)
            }
            m => err(format!("{} uses unsupported compression {m}", self.name)),
        }
    }
}

fn read_range(r: &mut (impl Read + Seek), at: u64, len: u64) -> Result<Vec<u8>, FigError> {
    let io = |e: std::io::Error| FigError(format!("cannot read the file: {e}"));
    let len = usize::try_from(len).map_err(|_| FigError("entry too large".into()))?;
    r.seek(SeekFrom::Start(at)).map_err(io)?;
    let mut buf = vec![0; len];
    r.read_exact(&mut buf).map_err(io)?;
    Ok(buf)
}

/// Reads the central directory of a seekable archive of `len` bytes.
pub fn read_entries(r: &mut (impl Read + Seek), len: u64) -> Result<Vec<Entry>, FigError> {
    let tail_len = TAIL_LEN.min(len);
    let tail = read_range(r, len - tail_len, tail_len)?;
    let (offset, size) = central_directory(&tail, len)?;
    entries(&read_range(r, offset, size)?)
}

/// Reads and decompresses one entry of a seekable archive.
pub fn read_entry(r: &mut (impl Read + Seek), entry: &Entry) -> Result<Vec<u8>, FigError> {
    let header = read_range(r, entry.header_offset, LOCAL_HEADER_LEN)?;
    let start = entry.data_start(&header)?;
    entry.decode(read_range(r, start, entry.compressed_size)?)
}
