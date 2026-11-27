//! Streaming and zero-copy lazy `.fig` archive rewriting.
//!
//! Large `.fig` ZIP archives opened via [`Document::open_lazy`] / [`Document::from_lazy`] keep
//! untouched images unread in the source file. Rewriting them reuses `zip::ZipArchive` and
//! `zip::ZipWriter::raw_copy_file` so untouched entries are neither decompressed nor buffered
//! in full:
//! - Desktop streams directly from the source `File` to the destination temp `File`
//!   ([`Document::write_lazy_archive`]).
//! - Browser reads only the tail, central directory, and 30-byte local headers asynchronously
//!   via `File.slice()`, builds a list of [`ArchivePart`]s via [`LazyArchiveMeta::plan`], and
//!   emits untouched entries as zero-copy `File.slice()` `Blob` parts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

use zip::read::{ArchiveOffset, Config};

use crate::archive::LOCAL_HEADER_LEN;
use crate::doc::{Document, FigError};

/// One piece of a rebuilt `.fig` archive produced without reading or decompressing untouched
/// source entries into memory.
///
/// Write every [`ArchivePart::Bytes`] verbatim and copy or slice every
/// [`ArchivePart::SourceRange`] directly from the original `.fig` archive to produce a
/// complete `.fig` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchivePart {
    /// Newly generated archive bytes (local headers, `canvas.fig`, `thumbnail.png`, newly
    /// imported images, central directory, and end-of-central-directory records).
    Bytes(Vec<u8>),
    /// Byte range `[offset .. offset + len)` of the original `.fig` file, copied verbatim.
    SourceRange { offset: u64, len: u64 },
}

/// Central-directory metadata of a source `.fig` ZIP archive, parsed without reading any
/// entry payloads.
#[derive(Clone)]
pub struct LazyArchiveMeta {
    file_len: u64,
    tail: Vec<u8>,
    cd_offset: u64,
    cd: Vec<u8>,
}

/// Counts and offsets only; never prints entry names or hashes.
impl fmt::Debug for LazyArchiveMeta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LazyArchiveMeta")
            .field("file_len", &self.file_len)
            .field("tail_len", &self.tail.len())
            .field("cd_offset", &self.cd_offset)
            .field("cd_len", &self.cd.len())
            .finish()
    }
}

fn zip_err(e: zip::result::ZipError) -> FigError {
    FigError::archive("cannot write archive", e)
}

fn zip_read_err(e: zip::result::ZipError) -> FigError {
    FigError::archive("cannot read source archive", e)
}

fn io_err(e: io::Error) -> FigError {
    FigError::io("cannot write archive", e)
}

fn io_read_err(e: io::Error) -> FigError {
    FigError::io("cannot read source archive", e)
}

fn stored_opts(len: usize) -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(len > u32::MAX as usize / 2)
}

/// Whether an entry from the original archive should be raw-copied into the saved copy.
///
/// `canvas.fig` and `thumbnail.png` are rewritten from `doc`, `"images/"` is emitted once
/// explicitly, and any `images/<hash>` overridden in `doc.images` is replaced by `doc.images`.
/// Every other entry (`meta.json`, untouched images including 0-byte ones, custom files and
/// directories) is preserved verbatim.
fn should_copy_source_entry(doc: &Document, name: &str) -> bool {
    if name == "canvas.fig" || name == "thumbnail.png" || name == "images/" {
        return false;
    }
    if let Some(hash) = name.strip_prefix("images/")
        && !hash.is_empty()
        && doc.images.contains_key(&hash.to_ascii_lowercase())
    {
        return false;
    }
    true
}

fn write_canvas_thumb_and_images_dir<W: Write + Seek>(
    doc: &Document,
    dst: &mut zip::ZipWriter<W>,
) -> Result<(), FigError> {
    let canvas = doc.encode_canvas()?;
    dst.start_file("canvas.fig", stored_opts(canvas.len()))
        .map_err(zip_err)?;
    dst.write_all(&canvas).map_err(io_err)?;
    if let Some(thumb) = &doc.thumbnail {
        dst.start_file("thumbnail.png", stored_opts(thumb.len()))
            .map_err(zip_err)?;
        dst.write_all(thumb).map_err(io_err)?;
    }
    dst.add_directory("images/", stored_opts(0))
        .map_err(zip_err)?;
    Ok(())
}

fn write_new_images<W: Write + Seek>(
    doc: &Document,
    dst: &mut zip::ZipWriter<W>,
) -> Result<(), FigError> {
    let mut new_hashes: Vec<&String> = doc.images.keys().collect();
    new_hashes.sort();
    for hash in new_hashes {
        let data = &doc.images[hash];
        dst.start_file(format!("images/{hash}"), stored_opts(data.len()))
            .map_err(zip_err)?;
        dst.write_all(data).map_err(io_err)?;
    }
    Ok(())
}

impl Document {
    /// Verifies that `src` still matches the archive identity captured when `self` was lazily
    /// opened, reading only tail/central-directory metadata (never entry payloads).
    pub(crate) fn verify_lazy_source<R: Read + Seek>(
        &self,
        src: &mut R,
    ) -> Result<usize, FigError> {
        if !self.source.lazy {
            return Err(FigError::malformed(
                "write_lazy_archive requires a document opened lazily",
            ));
        }
        let Some(expected) = self.source.lazy_identity else {
            return Err(FigError::malformed(
                "lazy document has no source archive identity",
            ));
        };
        let len = src.seek(SeekFrom::End(0)).map_err(io_read_err)?;
        let (entries, fp) = crate::archive::read_entries_with_fingerprint(src, len)?;
        if entries.len() != expected.entry_count
            || crate::archive::entries_sha1(&entries) != expected.entries_sha1
        {
            return Err(FigError::malformed(
                "source .fig archive entries do not match the opened document",
            ));
        }
        if let Some(expected_fp) = expected.container
            && fp != expected_fp
        {
            return Err(FigError::malformed(
                "source .fig archive changed since it was opened",
            ));
        }
        src.seek(SeekFrom::Start(0)).map_err(io_read_err)?;
        Ok(expected.entry_count)
    }

    /// Writes a `.fig` archive to `dst`, replacing `canvas.fig` (and `thumbnail.png` when
    /// present), raw-copying untouched entries from `src` without decompressing or buffering
    /// them in full, and writing newly added or overridden `self.images`.
    ///
    /// Verifies `src` against the opened document's archive identity both before and after
    /// streaming so path replacement or in-place modification around streaming cannot mix an old
    /// canvas with a changed archive.
    pub fn write_lazy_archive<R: Read + Seek, W: Write + Seek>(
        &self,
        mut src: R,
        dst: W,
    ) -> Result<W, FigError> {
        let expected_count = self.verify_lazy_source(&mut src)?;
        let mut src_zip = zip::ZipArchive::new(src).map_err(zip_read_err)?;
        if src_zip.offset() != 0 || src_zip.len() != expected_count {
            return Err(FigError::malformed(
                "source .fig archive layout does not match the opened document",
            ));
        }
        let mut dst_zip = zip::ZipWriter::new(dst);
        write_canvas_thumb_and_images_dir(self, &mut dst_zip)?;

        for i in 0..src_zip.len() {
            {
                let meta = src_zip.by_index_data(i).map_err(zip_read_err)?;
                if meta.encrypted() {
                    return Err(FigError::malformed(
                        "encrypted .fig archives are not supported",
                    ));
                }
                let name = meta.name().map_err(zip_read_err)?;
                if !should_copy_source_entry(self, &name) {
                    continue;
                }
            }
            let entry = src_zip.by_index_raw(i).map_err(zip_read_err)?;
            dst_zip.raw_copy_file(entry).map_err(zip_err)?;
        }

        write_new_images(self, &mut dst_zip)?;
        let dst = dst_zip.finish().map_err(zip_err)?;
        let mut src = src_zip.into_inner();
        self.verify_lazy_source(&mut src)?;
        Ok(dst)
    }
}

impl LazyArchiveMeta {
    fn verify_doc(&self, doc: &Document) -> Result<(), FigError> {
        if !doc.source.lazy {
            return Err(FigError::malformed(
                "lazy archive plan requires a document opened lazily",
            ));
        }
        let Some(expected) = doc.source.lazy_identity else {
            return Err(FigError::malformed(
                "lazy document has no source archive identity",
            ));
        };
        let entries = crate::archive::entries(&self.cd)?;
        if entries.len() != expected.entry_count
            || crate::archive::entries_sha1(&entries) != expected.entries_sha1
        {
            return Err(FigError::malformed(
                "source .fig archive entries do not match the opened document",
            ));
        }
        if let Some(expected_fp) = expected.container {
            let fp = crate::archive::ContainerFingerprint::from_parts(
                self.file_len,
                &self.tail,
                self.cd_offset,
                &self.cd,
            );
            if fp != expected_fp {
                return Err(FigError::malformed(
                    "source .fig archive changed since it was opened",
                ));
            }
        }
        Ok(())
    }

    /// Validates and stores the central-directory metadata of a source `.fig` archive from
    /// its `tail` (ending at `file_len`) and central directory `cd` (starting at `cd_offset`).
    pub fn from_directory(
        file_len: u64,
        tail: Vec<u8>,
        cd_offset: u64,
        cd: Vec<u8>,
    ) -> Result<Self, FigError> {
        if (tail.len() as u64) > file_len {
            return Err(FigError::malformed("archive tail is longer than file"));
        }
        if cd_offset
            .checked_add(cd.len() as u64)
            .is_none_or(|end| end > file_len)
        {
            return Err(FigError::malformed("central directory outside the file"));
        }
        let meta = Self {
            file_len,
            tail,
            cd_offset,
            cd,
        };
        // Validate the central directory immediately so malformed archives fail early.
        let _ = meta.open_archive(
            HashMap::new(),
            Rc::new(RefCell::new(PayloadBridge::default())),
        )?;
        Ok(meta)
    }

    /// Unique local-header byte offsets (each [`LOCAL_HEADER_LEN`] bytes) needed from the
    /// source file to plan a lazy save of `doc`.
    pub fn needed_local_headers(&self, doc: &Document) -> Result<Vec<u64>, FigError> {
        self.verify_doc(doc)?;
        let archive = self.open_archive(
            HashMap::new(),
            Rc::new(RefCell::new(PayloadBridge::default())),
        )?;
        let mut seen = HashSet::new();
        let mut offsets = Vec::new();
        for i in 0..archive.len() {
            let meta = archive.by_index_data(i).map_err(zip_read_err)?;
            if meta.encrypted() {
                return Err(FigError::malformed(
                    "encrypted .fig archives are not supported",
                ));
            }
            let name = meta.name().map_err(zip_read_err)?;
            if !should_copy_source_entry(doc, &name) {
                continue;
            }
            let start = meta.header_start();
            if start
                .checked_add(LOCAL_HEADER_LEN)
                .is_none_or(|end| end > self.file_len)
            {
                return Err(FigError::malformed(
                    "archive entry local header lies outside the file",
                ));
            }
            if seen.insert(start) {
                offsets.push(start);
            }
        }
        Ok(offsets)
    }

    /// Rebuilds the archive structure for `doc` using `zip::ZipWriter`, emitting untouched
    /// entries as [`ArchivePart::SourceRange`] references without reading or decompressing
    /// their payloads.
    pub fn plan(
        &self,
        doc: &Document,
        local_headers: &[(u64, [u8; 30])],
    ) -> Result<Vec<ArchivePart>, FigError> {
        self.verify_doc(doc)?;
        let mut headers = HashMap::with_capacity(local_headers.len());
        for &(offset, bytes) in local_headers {
            headers.insert(offset, bytes);
        }
        let bridge = Rc::new(RefCell::new(PayloadBridge::default()));
        let mut src_zip = self.open_archive(headers, Rc::clone(&bridge))?;
        bridge.borrow_mut().cd_parsed = true;
        let mut dst_zip = zip::ZipWriter::new(PartWriter::new(Rc::clone(&bridge)));

        write_canvas_thumb_and_images_dir(doc, &mut dst_zip)?;

        for i in 0..src_zip.len() {
            {
                let meta = src_zip.by_index_data(i).map_err(zip_read_err)?;
                if meta.encrypted() {
                    return Err(FigError::malformed(
                        "encrypted .fig archives are not supported",
                    ));
                }
                let name = meta.name().map_err(zip_read_err)?;
                if !should_copy_source_entry(doc, &name) {
                    continue;
                }
            }
            let entry = src_zip.by_index_raw(i).map_err(zip_read_err)?;
            let data_start = entry.data_start().ok_or_else(|| {
                FigError::malformed("could not determine archive entry data offset")
            })?;
            let compressed_size = entry.compressed_size();
            if data_start
                .checked_add(compressed_size)
                .is_none_or(|end| end > self.file_len)
            {
                return Err(FigError::malformed(
                    "archive entry payload lies outside the source file",
                ));
            }
            bridge.borrow_mut().arm(data_start, compressed_size);
            let copied = dst_zip.raw_copy_file(entry).map_err(zip_err);
            let verified = bridge.borrow_mut().disarm_and_verify(compressed_size);
            copied?;
            verified?;
        }

        write_new_images(doc, &mut dst_zip)?;
        let writer = dst_zip.finish().map_err(zip_err)?;
        Ok(writer.into_parts())
    }

    fn open_archive(
        &self,
        local_headers: HashMap<u64, [u8; 30]>,
        bridge: Rc<RefCell<PayloadBridge>>,
    ) -> Result<zip::ZipArchive<MetadataOnlyReader<'_>>, FigError> {
        let reader = MetadataOnlyReader {
            file_len: self.file_len,
            tail_start: self.file_len - self.tail.len() as u64,
            tail: &self.tail,
            cd_offset: self.cd_offset,
            cd: &self.cd,
            local_headers,
            pos: 0,
            bridge,
        };
        let config = Config {
            archive_offset: ArchiveOffset::Known(0),
        };
        zip::ZipArchive::with_config(config, reader).map_err(zip_read_err)
    }
}

/// Coordinates [`MetadataOnlyReader`] and [`PartWriter`] during a single `raw_copy_file` call
/// so local headers go into [`ArchivePart::Bytes`] and raw payloads become a zero-copy
/// [`ArchivePart::SourceRange`].
#[derive(Default)]
struct PayloadBridge {
    /// Set once `ZipArchive` finishes parsing the central directory; afterwards only fetched
    /// `local_headers` and armed payload ranges are readable (never `tail`/`cd`).
    cd_parsed: bool,
    armed: Option<(u64, u64)>,
    in_payload: bool,
    emitted: bool,
    skipped: u64,
}

impl PayloadBridge {
    fn arm(&mut self, data_start: u64, compressed_size: u64) {
        self.armed = Some((data_start, compressed_size));
        self.in_payload = false;
        self.emitted = false;
        self.skipped = 0;
    }

    fn disarm_and_verify(&mut self, expected: u64) -> Result<(), FigError> {
        let skipped = self.skipped;
        self.armed = None;
        self.in_payload = false;
        self.emitted = false;
        self.skipped = 0;
        if skipped != expected {
            return Err(FigError::malformed(format!(
                "archive entry copy length mismatch: expected {expected} bytes, planned {skipped}"
            )));
        }
        Ok(())
    }
}

/// Seekable reader that serves only ZIP metadata (tail, central directory, and 30-byte local
/// headers) and synthetic zeroes for the currently armed raw-copy payload range, refusing any
/// read outside metadata so payload bytes are never needed or materialized.
struct MetadataOnlyReader<'a> {
    file_len: u64,
    tail_start: u64,
    tail: &'a [u8],
    cd_offset: u64,
    cd: &'a [u8],
    local_headers: HashMap<u64, [u8; 30]>,
    pos: u64,
    bridge: Rc<RefCell<PayloadBridge>>,
}

impl Read for MetadataOnlyReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let (cd_parsed, armed) = {
            let b = self.bridge.borrow();
            (b.cd_parsed, b.armed)
        };
        if let Some((start, len)) = armed {
            let end = start
                .checked_add(len)
                .ok_or_else(|| io::Error::other("archive payload range overflow"))?;
            if self.pos >= start && self.pos < end {
                self.bridge.borrow_mut().in_payload = true;
                let n =
                    usize::try_from((end - self.pos).min(buf.len() as u64)).unwrap_or(buf.len());
                buf[..n].fill(0);
                self.pos += n as u64;
                return Ok(n);
            }
        }
        if !cd_parsed {
            if let Some(n) = copy_from_slice(self.pos, self.tail_start, self.tail, buf) {
                self.pos += n as u64;
                return Ok(n);
            }
            if let Some(n) = copy_from_slice(self.pos, self.cd_offset, self.cd, buf) {
                self.pos += n as u64;
                return Ok(n);
            }
        }
        if let Some(bytes) = self.local_headers.get(&self.pos) {
            let n = buf.len().min(bytes.len());
            buf[..n].copy_from_slice(&bytes[..n]);
            self.pos += n as u64;
            return Ok(n);
        }
        for (&offset, bytes) in &self.local_headers {
            if let Some(n) = copy_from_slice(self.pos, offset, bytes, buf) {
                self.pos += n as u64;
                return Ok(n);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "missing source archive metadata range",
        ))
    }
}

fn copy_from_slice(pos: u64, slice_start: u64, slice: &[u8], out: &mut [u8]) -> Option<usize> {
    let rel = pos.checked_sub(slice_start)?;
    let rel = usize::try_from(rel).ok()?;
    if rel >= slice.len() {
        return None;
    }
    let n = out.len().min(slice.len() - rel);
    out[..n].copy_from_slice(&slice[rel..rel + n]);
    Some(n)
}

impl Seek for MetadataOnlyReader<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(x) => x,
            SeekFrom::End(off) => seek_offset(self.file_len, off)?,
            SeekFrom::Current(off) => seek_offset(self.pos, off)?,
        };
        if next > self.file_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek past end of archive",
            ));
        }
        self.pos = next;
        Ok(self.pos)
    }
}

/// Writer backing `zip::ZipWriter` that buffers headers and rewritten entries in memory and
/// records untouched `raw_copy_file` payloads as zero-copy [`ArchivePart::SourceRange`] entries
/// while keeping the virtual byte position exact for ZIP32 and ZIP64 offsets.
struct PartWriter {
    pos: u64,
    buf_start: u64,
    buf: Vec<u8>,
    parts: Vec<ArchivePart>,
    bridge: Rc<RefCell<PayloadBridge>>,
}

impl PartWriter {
    fn new(bridge: Rc<RefCell<PayloadBridge>>) -> Self {
        Self {
            pos: 0,
            buf_start: 0,
            buf: Vec::new(),
            parts: Vec::new(),
            bridge,
        }
    }

    fn end_pos(&self) -> u64 {
        self.buf_start + self.buf.len() as u64
    }

    fn into_parts(mut self) -> Vec<ArchivePart> {
        if !self.buf.is_empty() {
            self.parts.push(ArchivePart::Bytes(self.buf));
        }
        self.parts
    }
}

impl Write for PartWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        let in_payload = self.bridge.borrow().in_payload;
        if in_payload {
            if self.pos != self.end_pos() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "cannot emit source range while positioned inside buffered bytes",
                ));
            }
            let mut bridge = self.bridge.borrow_mut();
            let Some((src_offset, expected_len)) = bridge.armed else {
                return Err(io::Error::other("payload write without armed source range"));
            };
            if !bridge.emitted {
                if !self.buf.is_empty() {
                    self.parts
                        .push(ArchivePart::Bytes(std::mem::take(&mut self.buf)));
                }
                if expected_len > 0 {
                    self.parts.push(ArchivePart::SourceRange {
                        offset: src_offset,
                        len: expected_len,
                    });
                }
                bridge.emitted = true;
            }
            let n = data.len() as u64;
            let next_skipped = bridge
                .skipped
                .checked_add(n)
                .ok_or_else(|| io::Error::other("archive payload length overflow"))?;
            if next_skipped > expected_len {
                return Err(io::Error::other(
                    "archive payload write exceeded declared compressed size",
                ));
            }
            bridge.skipped = next_skipped;
            self.pos = self
                .pos
                .checked_add(n)
                .ok_or_else(|| io::Error::other("archive output size overflow"))?;
            self.buf_start = self.pos;
            return Ok(data.len());
        }

        let rel = self
            .pos
            .checked_sub(self.buf_start)
            .and_then(|d| usize::try_from(d).ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "cannot write before already-flushed archive part",
                )
            })?;
        if rel == self.buf.len() {
            self.buf.extend_from_slice(data);
        } else {
            let end = rel
                .checked_add(data.len())
                .ok_or_else(|| io::Error::other("archive buffer overflow"))?;
            if end > self.buf.len() {
                self.buf.resize(end, 0);
            }
            self.buf[rel..end].copy_from_slice(data);
        }
        self.pos = self
            .pos
            .checked_add(data.len() as u64)
            .ok_or_else(|| io::Error::other("archive output size overflow"))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for PartWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let end = self.end_pos();
        let next = match pos {
            SeekFrom::Start(x) => x,
            SeekFrom::End(off) => seek_offset(end, off)?,
            SeekFrom::Current(off) => seek_offset(self.pos, off)?,
        };
        if next < self.buf_start || next > end {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cannot seek outside current archive buffer",
            ));
        }
        self.pos = next;
        Ok(self.pos)
    }
}

fn seek_offset(base: u64, delta: i64) -> io::Result<u64> {
    let target = i128::from(base) + i128::from(delta);
    u64::try_from(target)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "seek offset out of range"))
}
