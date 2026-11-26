//! Writing a `Document` back to `.fig` bytes.
//!
//! The schema from the original file is reused, so every field the decoder kept is written
//! back with its original Kiwi type. Values are encoded straight from `Val` without building
//! a second tree, which matters for large files.

use std::io::{Cursor, Write};

use kiwi_schema::{
    ByteBufferMut, DefKind, Field, Schema, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64,
    TYPE_STRING, TYPE_UINT, TYPE_UINT64,
};

use std::sync::Arc;

use crate::doc::{Codec, Document, FigError};
use crate::read::Decoder;
use crate::val::Val;

fn fail<T>(msg: String) -> Result<T, FigError> {
    Err(FigError::malformed(msg))
}

struct Encoder<'a> {
    schema: &'a Schema,
    /// The decoder the document was read with; its unedited lazy fields are copied as bytes.
    decoder: Option<&'a Arc<Decoder>>,
    bb: ByteBufferMut,
}

impl Encoder<'_> {
    fn value(&mut self, type_id: i32, v: &Val, at: &str) -> Result<(), FigError> {
        let v = v.view();
        let bb = &mut self.bb;
        let int = |v: &Val| match v {
            Val::Int(i) => Ok(*i),
            other => fail(format!("{at}: expected an integer, found {}", kind(other))),
        };
        match type_id {
            TYPE_BOOL => match v {
                Val::Bool(b) => bb.write_bool(*b),
                other => return fail(format!("{at}: expected a bool, found {}", kind(other))),
            },
            // Narrowing is checked: an out-of-range value is an error, never a wrapped value.
            TYPE_BYTE => bb.write_byte(narrow::<u8>(int(v)?, at, "byte")?),
            TYPE_INT => bb.write_var_int(narrow::<i32>(int(v)?, at, "int")?),
            TYPE_UINT => bb.write_var_uint(narrow::<u32>(int(v)?, at, "uint")?),
            TYPE_INT64 => bb.write_var_int64(int(v)?),
            // Intentional: the decoder stores a uint64 as its 64 bits in an i64, so the same bit
            // pattern is written back. Every i64 has exactly one u64 with those bits.
            TYPE_UINT64 => bb.write_var_uint64(int(v)? as u64),
            TYPE_FLOAT => match v {
                Val::Float(f) => bb.write_var_float(*f as f32),
                Val::Int(i) => bb.write_var_float(*i as f32),
                other => return fail(format!("{at}: expected a number, found {}", kind(other))),
            },
            TYPE_STRING => match v {
                Val::Str(s) => bb.write_string(s),
                other => return fail(format!("{at}: expected a string, found {}", kind(other))),
            },
            _ => {
                let schema = self.schema;
                let Some((idx, def)) = usize::try_from(type_id)
                    .ok()
                    .and_then(|i| Some((i, schema.defs.get(i)?)))
                else {
                    return fail(format!("{at}: unknown type {type_id}"));
                };
                match def.kind {
                    DefKind::Enum => {
                        let Val::Enum(name) = v else {
                            return fail(format!("{at}: expected {}, found {}", def.name, kind(v)));
                        };
                        let Some(field) = def.field(name) else {
                            return fail(format!("{at}: {} has no value {name}", def.name));
                        };
                        bb.write_var_uint(field.value);
                    }
                    DefKind::Struct if let Val::Guid(g) = v => {
                        if def.fields.len() != 2 {
                            return fail(format!("{at}: {} is not a GUID", def.name));
                        }
                        for field in &def.fields {
                            let part = match field.name.as_str() {
                                "sessionID" => g.0,
                                "localID" => g.1,
                                _ => return fail(format!("{at}: {} is not a GUID", def.name)),
                            };
                            self.field(field, &Val::Int(i64::from(part)), at)?;
                        }
                    }
                    DefKind::Struct => {
                        for field in &def.fields {
                            let Some(fv) = v.get(&field.name) else {
                                return fail(format!("{at}: {} needs {}", def.name, field.name));
                            };
                            self.field(field, fv, at)?;
                        }
                    }
                    DefKind::Message => {
                        let Val::Obj(map) = v else {
                            return fail(format!("{at}: expected {}, found {}", def.name, kind(v)));
                        };
                        if let Some(unknown) = map.keys().find(|k| def.field(k).is_none()) {
                            return fail(format!("{at}: {} has no field {unknown}", def.name));
                        }
                        let decoder = self.decoder;
                        for (i, field) in def.fields.iter().enumerate() {
                            // The document's own schema: its decoder has the interned name.
                            let fv = match decoder.and_then(|d| d.name(idx, i)) {
                                Some(name) => map.get_raw_named(name),
                                None => map.get_raw(&field.name),
                            };
                            if let Some(fv) = fv {
                                self.bb.write_var_uint(field.value);
                                self.field(field, fv, at)?;
                            }
                        }
                        self.bb.write_byte(0);
                    }
                }
            }
        }
        Ok(())
    }

    fn field(&mut self, field: &Field, v: &Val, at: &str) -> Result<(), FigError> {
        // Still encoded, by this document's schema: the original bytes, not re-encoded.
        if let Val::Lazy(l) = v
            && let Some(bytes) = l.bytes_for(self.decoder)
        {
            self.bb.write_bytes(bytes);
            return Ok(());
        }
        let v = v.view();
        if !field.is_array {
            return self.value(field.type_id, v, at);
        }
        match v {
            Val::Bytes(bytes) if field.type_id == TYPE_BYTE => {
                self.bb.write_var_uint(len32(bytes.len(), at)?);
                self.bb.write_bytes(bytes);
            }
            Val::Arr(items) => {
                self.bb.write_var_uint(len32(items.len(), at)?);
                for item in items {
                    self.value(field.type_id, item, at)?;
                }
            }
            other => {
                return fail(format!(
                    "{at}: {} must be a list, found {}",
                    field.name,
                    kind(other)
                ));
            }
        }
        Ok(())
    }
}

/// Converts `v` to `T`, or fails with the field's path and the value.
fn narrow<T: TryFrom<i64>>(v: i64, at: &str, what: &str) -> Result<T, FigError> {
    T::try_from(v).or_else(|_| fail(format!("{at}: {v} does not fit a {what}")))
}

fn len32(n: usize, at: &str) -> Result<u32, FigError> {
    u32::try_from(n).map_err(|_| FigError::malformed(format!("{at}: list too long")))
}

fn kind(v: &Val) -> &'static str {
    match v {
        Val::Bool(_) => "bool",
        Val::Int(_) => "integer",
        Val::Float(_) => "number",
        Val::Str(_) => "string",
        Val::Enum(_) => "enum",
        Val::Bytes(_) => "bytes",
        Val::Arr(_) => "list",
        Val::Obj(_) | Val::Guid(_) => "object",
        Val::Lazy(l) => kind(l.get()),
        Val::Shared(v) => kind(v),
    }
}

fn compress(data: &[u8], codec: Codec) -> Result<Vec<u8>, FigError> {
    let io = |e: std::io::Error| FigError::io("compression failed", e);
    Ok(match codec {
        Codec::Deflate => {
            let mut enc =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data).map_err(io)?;
            enc.finish().map_err(io)?
        }
        Codec::Zlib => {
            let mut enc =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data).map_err(io)?;
            enc.finish().map_err(io)?
        }
        Codec::Zstd => {
            ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
        }
    })
}

impl Document {
    /// Encodes the Kiwi message (nodes, blobs and the other top-level fields), uncompressed.
    pub fn encode_message(&self) -> Result<Vec<u8>, FigError> {
        let schema = Schema::decode(&self.source.schema)
            .map_err(|()| FigError::malformed("cannot decode the file schema"))?;
        let Some(message) = schema.def("Message") else {
            return fail("schema has no Message type".into());
        };
        let mut enc = Encoder {
            schema: &schema,
            decoder: self.source.decoder.as_ref(),
            bb: ByteBufferMut::new(),
        };
        // Figma writes the other fields in schema order and the two big lists last; doing the
        // same makes an unedited document encode to the original bytes.
        for field in &message.fields {
            if field.name == "nodeChanges" || field.name == "blobs" {
                continue;
            }
            if let Some(v) = self.source.message_extra.get(&field.name) {
                enc.bb.write_var_uint(field.value);
                enc.field(field, v, &field.name)?;
            }
        }
        if let Some(field) = message.field("nodeChanges") {
            enc.bb.write_var_uint(field.value);
            enc.bb
                .write_var_uint(len32(self.nodes.len(), "nodeChanges")?);
            for node in &self.nodes {
                enc.value(field.type_id, &node.data, &format!("node {}", node.id))?;
            }
        }
        if let Some(field) = message.field("blobs")
            && (self.source.had_blobs || !self.blobs.is_empty())
        {
            let blob_def = usize::try_from(field.type_id)
                .ok()
                .and_then(|i| schema.defs.get(i));
            if blob_def.is_none_or(|d| d.fields.len() != 1 || d.fields[0].name != "bytes") {
                return fail("unexpected Blob layout in schema".into());
            }
            enc.bb.write_var_uint(field.value);
            enc.bb.write_var_uint(len32(self.blobs.len(), "blobs")?);
            let blob_def = blob_def.unwrap();
            let is_message = blob_def.kind == DefKind::Message;
            for blob in &self.blobs {
                // Blob has one byte[] field; a message also needs its tag and terminator.
                if is_message {
                    enc.bb.write_var_uint(blob_def.fields[0].value);
                }
                enc.bb.write_var_uint(len32(blob.len(), "blob")?);
                enc.bb.write_bytes(blob);
                if is_message {
                    enc.bb.write_byte(0);
                }
            }
        }
        enc.bb.write_byte(0);
        Ok(enc.bb.data())
    }

    /// The `canvas.fig` part: prelude, version and compressed chunks.
    pub fn encode_canvas(&self) -> Result<Vec<u8>, FigError> {
        let codec = |i: usize| self.source.codecs.get(i).copied().unwrap_or(Codec::Deflate);
        let chunks = [
            compress(&self.source.schema, codec(0))?,
            compress(&self.encode_message()?, codec(1))?,
        ];
        let mut prelude = self.prelude.as_bytes().to_vec();
        prelude.resize(8, b' ');
        let mut out = prelude;
        out.extend_from_slice(&self.format_version.to_le_bytes());
        for chunk in chunks.iter().chain(&self.source.extra_chunks) {
            out.extend_from_slice(&len32(chunk.len(), "chunk")?.to_le_bytes());
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }

    /// A complete `.fig` file. Archives keep their other entries; images are written from
    /// `images`, so removing an entry there drops it from the file.
    pub fn to_bytes(&self) -> Result<Vec<u8>, FigError> {
        if self.source.lazy {
            return Err(FigError::malformed(
                "opened without its images (open_lazy), so it cannot be written",
            ));
        }
        let canvas = self.encode_canvas()?;
        let Some(entries) = &self.source.archive_entries else {
            return Ok(canvas);
        };
        let zip_err = |e: zip::result::ZipError| FigError::archive("cannot write archive", e);
        let io = |e: std::io::Error| FigError::io("cannot write archive", e);
        // Figma stores entries uncompressed; the canvas chunks are compressed already.
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(canvas.len() > u32::MAX as usize / 2);
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("canvas.fig", opts).map_err(zip_err)?;
        zip.write_all(&canvas).map_err(io)?;
        if let Some(thumb) = &self.thumbnail {
            zip.start_file("thumbnail.png", opts).map_err(zip_err)?;
            zip.write_all(thumb).map_err(io)?;
        }
        for (name, data) in entries {
            zip.start_file(name.as_str(), opts).map_err(zip_err)?;
            zip.write_all(data).map_err(io)?;
        }
        zip.add_directory("images/", opts).map_err(zip_err)?;
        let mut hashes: Vec<&String> = self.images.keys().collect();
        hashes.sort();
        for hash in hashes {
            zip.start_file(format!("images/{hash}"), opts)
                .map_err(zip_err)?;
            zip.write_all(&self.images[hash]).map_err(io)?;
        }
        Ok(zip.finish().map_err(zip_err)?.into_inner())
    }

    /// Writes a `.fig` copy to `raw_dst` with path-safety checks:
    /// - rejects any `..` (`ParentDir`) traversal component,
    /// - requires a `.fig` extension,
    /// - resolves relative paths against `source_path`'s parent directory when available,
    /// - optionally confines the canonical destination directory inside `source_path`'s directory
    ///   (and fails when there is no source file on disk to confine to),
    /// - unconditionally refuses to overwrite `source_path` (even when `overwrite` is `true`),
    ///   comparing file identity so a differently cased name on a case-insensitive disk is caught,
    /// - refuses to replace an existing file unless `overwrite` is `true`. The final no-overwrite
    ///   commit is atomic (`RENAME_NOREPLACE` / `RENAME_EXCL` on Linux/macOS, or `hard_link` +
    ///   `remove_file`) and fails closed without touching `dst` when atomic publication is
    ///   unavailable; the earlier source and symlink checks are not atomic.
    pub fn save_copy_checked(
        &self,
        source_path: Option<&std::path::Path>,
        raw_dst: &std::path::Path,
        overwrite: bool,
        confine_to_source_dir: bool,
    ) -> Result<std::path::PathBuf, FigError> {
        use std::path::Component;

        if raw_dst
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(FigError::malformed(
                "save_copy outPath must not contain '..' path traversal",
            ));
        }
        let has_fig_ext = raw_dst
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("fig"));
        if !has_fig_ext {
            return Err(FigError::malformed(
                "save_copy outPath must end with a .fig extension",
            ));
        }
        let file_name = raw_dst
            .file_name()
            .ok_or_else(|| FigError::malformed("save_copy outPath has no file name"))?;

        let src_dir = source_path
            .and_then(|p| p.parent())
            .filter(|d| !d.as_os_str().is_empty());
        let resolved = if raw_dst.is_relative() {
            match src_dir {
                Some(base) => base.join(raw_dst),
                None => raw_dst.to_path_buf(),
            }
        } else {
            raw_dst.to_path_buf()
        };

        let dst_parent = match resolved.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => std::path::Path::new("."),
        };
        let canon_dst_parent = std::fs::canonicalize(dst_parent).map_err(|e| {
            FigError::io(format!("destination directory {}", dst_parent.display()), e)
        })?;

        let existing_source = source_path.filter(|p| p.exists());
        if confine_to_source_dir && existing_source.is_none() {
            return Err(FigError::malformed(
                "save_copy needs the source .fig on disk to keep outPath inside its folder",
            ));
        }
        if let Some(sp) = existing_source {
            let canon_src = std::fs::canonicalize(sp)
                .map_err(|e| FigError::io(format!("source file {}", sp.display()), e))?;
            if confine_to_source_dir
                && let Some(canon_src_dir) = canon_src.parent()
                && !canon_dst_parent.starts_with(canon_src_dir)
            {
                return Err(FigError::malformed(format!(
                    "save_copy outPath must stay inside {}",
                    canon_src_dir.display()
                )));
            }
            let target_in_canon_dir = canon_dst_parent.join(file_name);
            if target_in_canon_dir == canon_src
                || same_file::is_same_file(sp, &resolved).unwrap_or(false)
            {
                return Err(FigError::malformed(
                    "save_copy refuses to overwrite the original source file",
                ));
            }
            if let Ok(canon_existing) = std::fs::canonicalize(&resolved)
                && canon_existing == canon_src
            {
                return Err(FigError::malformed(
                    "save_copy refuses to overwrite the original source file",
                ));
            }
        }

        if let Ok(meta) = std::fs::symlink_metadata(&resolved) {
            if meta.file_type().is_symlink() {
                return Err(FigError::malformed(
                    "save_copy refuses to write through a symlink destination",
                ));
            }
            if !overwrite {
                return Err(FigError::malformed(format!(
                    "{} already exists; pass overwrite: true to replace it",
                    resolved.display()
                )));
            }
        }

        let final_dst = canon_dst_parent.join(file_name);
        self.write_archive_to_temp_and_commit(source_path, &final_dst, overwrite)?;
        Ok(final_dst)
    }

    /// Writes a `.fig` file to `dst`, streaming untouched archive entries directly from
    /// `lazy_source` when `self` was opened with [`Document::open_lazy`].
    pub fn save_with_lazy_source(
        &self,
        lazy_source: Option<&std::path::Path>,
        dst: impl AsRef<std::path::Path>,
    ) -> Result<(), FigError> {
        let dst = dst.as_ref();
        if let Some(src) = lazy_source
            && src.exists()
            && let Ok(canon_src) = std::fs::canonicalize(src)
        {
            let dst_parent = match dst.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => std::path::Path::new("."),
            };
            if let (Ok(cp), Some(fname)) = (std::fs::canonicalize(dst_parent), dst.file_name())
                && cp.join(fname) == canon_src
            {
                return Err(FigError::malformed(
                    "refusing to overwrite source file in save_with_lazy_source",
                ));
            }
        }
        self.write_archive_to_temp_and_commit(lazy_source, dst, true)
    }

    fn write_archive_to_temp_and_commit(
        &self,
        lazy_source: Option<&std::path::Path>,
        dst: &std::path::Path,
        overwrite: bool,
    ) -> Result<(), FigError> {
        let commit_temp = |tmp: &std::path::Path| commit_temp_file(tmp, dst, overwrite);

        if !self.source.lazy {
            let bytes = self.to_bytes()?;
            let (tmp, mut file) = create_temp(dst)?;
            let written = std::io::Write::write_all(&mut file, &bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| FigError::io(tmp.display().to_string(), e));
            #[cfg_attr(
                target_arch = "wasm32",
                expect(clippy::drop_non_drop, reason = "wasm File has no Drop")
            )]
            drop(file);
            let res = written.and_then(|()| commit_temp(&tmp));
            if res.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            return res;
        }
        let Some(src_path) = lazy_source else {
            return Err(FigError::malformed(
                "lazy document requires source path to stream unread archive entries",
            ));
        };
        let src_file = std::fs::File::open(src_path)
            .map_err(|e| FigError::io("cannot read lazy source file", e))?;

        let (tmp, file) = create_temp(dst)?;
        let write_res = (|| -> Result<(), FigError> {
            let io = |e: std::io::Error| FigError::io("cannot write archive", e);
            let mut buf = self.write_lazy_archive(src_file, std::io::BufWriter::new(file))?;
            buf.flush().map_err(io)?;
            let inner = buf.into_inner().map_err(|e| io(e.into_error()))?;
            inner.sync_all().map_err(io)?;
            Ok(())
        })();

        let res = write_res.and_then(|()| commit_temp(&tmp));
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    /// Writes the file next to `path` first and then renames it over `path`, so a failed save
    /// never leaves a half-written file. The temp name is unique and created with `create_new`,
    /// so an existing file or symlink is never written through. It is removed on every failure;
    /// the destination is not touched until the rename.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), FigError> {
        let path = path.as_ref();
        let bytes = self.to_bytes()?;
        let (tmp, mut file) = create_temp(path)?;
        let written = std::io::Write::write_all(&mut file, &bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| FigError::io(tmp.display().to_string(), e));
        // Closed before the rename (Windows cannot rename an open file). On wasm `File` has no
        // `Drop`, which clippy flags there.
        #[cfg_attr(
            target_arch = "wasm32",
            expect(clippy::drop_non_drop, reason = "wasm File has no Drop")
        )]
        drop(file);
        let result = written.and_then(|()| {
            std::fs::rename(&tmp, path).map_err(|e| FigError::io(path.display().to_string(), e))
        });
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

/// Atomically publishes `tmp` to `dst`. When `overwrite` is `false`, never replaces an
/// existing destination and fails closed without creating or modifying `dst` if atomic
/// publication (`RENAME_NOREPLACE` / `RENAME_EXCL` or `hard_link`) is unavailable.
fn commit_temp_file(
    tmp: &std::path::Path,
    dst: &std::path::Path,
    overwrite: bool,
) -> Result<(), FigError> {
    let exists_err = || {
        FigError::malformed(format!(
            "{} already exists; pass overwrite: true to replace it",
            dst.display()
        ))
    };
    if overwrite {
        // Keep the replaced file's permissions without following a raced symlink.
        // shortcut: xattrs/ACLs are not carried over.
        if let Ok(meta) = std::fs::symlink_metadata(dst)
            && !meta.file_type().is_symlink()
        {
            let _ = std::fs::set_permissions(tmp, meta.permissions());
        }
        return std::fs::rename(tmp, dst).map_err(|e| FigError::io(dst.display().to_string(), e));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        use rustix::io::Errno;
        match renameat_with(CWD, tmp, CWD, dst, RenameFlags::NOREPLACE) {
            Ok(()) => return Ok(()),
            Err(Errno::EXIST) => return Err(exists_err()),
            // Kernel or filesystem lacks atomic no-replace rename: try hard_link next.
            Err(e)
                if [Errno::NOSYS, Errno::INVAL, Errno::NOTSUP, Errno::OPNOTSUPP].contains(&e) => {}
            Err(e) => {
                return Err(FigError::io(
                    format!("atomic no-overwrite save to {}", dst.display()),
                    e.into(),
                ));
            }
        }
    }
    // Fallback where RENAME_NOREPLACE / RENAME_EXCL is unavailable (and on other OSes):
    // hard_link atomically publishes the synced temp inode without clobbering `dst`.
    // If hard_link is also unavailable, fail closed rather than creating `dst` and copying.
    match std::fs::hard_link(tmp, dst) {
        Ok(()) => {
            let _ = std::fs::remove_file(tmp);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(exists_err()),
        Err(e) => Err(FigError::io(
            format!(
                "atomic no-overwrite save to {} is unavailable on this filesystem",
                dst.display()
            ),
            e,
        )),
    }
}

/// Creates a new, uniquely named file in the same directory as `path` (so the final rename stays
/// on one filesystem). `create_new` makes it fail rather than reuse an existing name.
fn create_temp(path: &std::path::Path) -> Result<(std::path::PathBuf, std::fs::File), FigError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let base = path
        .file_name()
        .map_or_else(|| "figstudio".into(), |n| n.to_os_string());
    let mut last_err = None;
    for _ in 0..16 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut name = base.clone();
        name.push(format!(".{}.{n}.saving", std::process::id()));
        let tmp = path.with_file_name(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last_err = Some(e),
            Err(e) => return Err(FigError::io(tmp.display().to_string(), e)),
        }
    }
    let what = format!("{}: cannot create a temporary file", path.display());
    Err(match last_err {
        Some(e) => FigError::io(what, e),
        None => FigError::malformed(format!("{what}: name clash")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_clobber_commit_publishes_new_file_and_preserves_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let tmp1 = dir.path().join("a.saving");
        std::fs::write(&tmp1, b"first-payload").unwrap();
        let dst = dir.path().join("out.fig");

        commit_temp_file(&tmp1, &dst, false).expect("atomic no-clobber publish to new path");
        assert!(!tmp1.exists(), "temp file must be gone after commit");
        assert_eq!(std::fs::read(&dst).unwrap(), b"first-payload");

        let tmp2 = dir.path().join("b.saving");
        std::fs::write(&tmp2, b"second-payload").unwrap();
        let err = commit_temp_file(&tmp2, &dst, false)
            .expect_err("no-clobber commit must refuse to overwrite existing destination");
        assert!(
            err.to_string().contains("already exists"),
            "unexpected error: {err}"
        );
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            b"first-payload",
            "existing destination must be untouched"
        );
        assert!(tmp2.exists(), "caller cleans up temp on failure");

        let occupied_dir = dir.path().join("occupied.fig");
        std::fs::create_dir(&occupied_dir).unwrap();
        assert!(
            commit_temp_file(&tmp2, &occupied_dir, false).is_err(),
            "no-clobber commit must refuse an existing directory"
        );
        assert!(occupied_dir.is_dir());

        #[cfg(unix)]
        {
            let target = dir.path().join("dangling-target.fig");
            let link = dir.path().join("link.fig");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(
                commit_temp_file(&tmp2, &link, false).is_err(),
                "no-clobber commit must refuse an existing dangling symlink"
            );
            assert!(!target.exists(), "must not follow dangling symlink");
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn no_clobber_commit_fails_closed_when_atomic_publish_is_unavailable() {
        use std::os::unix::fs::MetadataExt as _;

        // Pick two temp roots on different filesystems so neither renameat2(RENAME_NOREPLACE)
        // nor hard_link can publish atomically (EXDEV). A non-atomic copy fallback would succeed
        // and create `dst`; failing closed must return Err and never create `dst`.
        let candidates = [
            std::env::temp_dir(),
            std::path::PathBuf::from("/tmp"),
            std::path::PathBuf::from("/dev/shm"),
            std::path::PathBuf::from("/var/tmp"),
        ];
        let mut pair = None;
        'outer: for (i, a) in candidates.iter().enumerate() {
            let Ok(ma) = std::fs::metadata(a) else {
                continue;
            };
            for b in &candidates[i + 1..] {
                let Ok(mb) = std::fs::metadata(b) else {
                    continue;
                };
                if ma.dev() != mb.dev()
                    && let (Ok(da), Ok(db)) = (tempfile::tempdir_in(a), tempfile::tempdir_in(b))
                {
                    pair = Some((da, db));
                    break 'outer;
                }
            }
        }
        let Some((src_dir, dst_dir)) = pair else {
            eprintln!("SKIPPED: no two writable temp directories on distinct filesystems");
            return;
        };

        let tmp = src_dir.path().join("part.saving");
        std::fs::write(&tmp, b"staged-fig-bytes").unwrap();
        let dst = dst_dir.path().join("copy.fig");

        let res = commit_temp_file(&tmp, &dst, false);
        assert!(
            res.is_err(),
            "no-clobber commit must fail closed when atomic publication fails (not copy into dst)"
        );
        assert!(
            !dst.exists(),
            "failed atomic publication must not create a partial or copied destination"
        );
        assert_eq!(
            std::fs::read(&tmp).unwrap(),
            b"staged-fig-bytes",
            "staged temp file must remain intact for caller cleanup"
        );
    }
}
