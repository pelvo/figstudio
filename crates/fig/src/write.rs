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

use crate::doc::{Codec, Document, FigError};
use crate::val::Val;

fn fail<T>(msg: String) -> Result<T, FigError> {
    Err(FigError(msg))
}

struct Encoder<'a> {
    schema: &'a Schema,
    bb: ByteBufferMut,
}

impl Encoder<'_> {
    fn value(&mut self, type_id: i32, v: &Val, at: &str) -> Result<(), FigError> {
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
                let Some(def) = usize::try_from(type_id)
                    .ok()
                    .and_then(|i| schema.defs.get(i))
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
                        for field in &def.fields {
                            if let Some(fv) = map.get(&field.name) {
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
    u32::try_from(n).map_err(|_| FigError(format!("{at}: list too long")))
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
        Val::Obj(_) => "object",
    }
}

fn compress(data: &[u8], codec: Codec) -> Result<Vec<u8>, FigError> {
    let io = |e: std::io::Error| FigError(format!("compression failed: {e}"));
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
            .map_err(|()| FigError("cannot decode the file schema".into()))?;
        let Some(message) = schema.def("Message") else {
            return fail("schema has no Message type".into());
        };
        let mut enc = Encoder {
            schema: &schema,
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
            return Err(FigError(
                "opened without its images (open_lazy), so it cannot be written".into(),
            ));
        }
        let canvas = self.encode_canvas()?;
        let Some(entries) = &self.source.archive_entries else {
            return Ok(canvas);
        };
        let zip_err = |e: zip::result::ZipError| FigError(format!("cannot write archive: {e}"));
        let io = |e: std::io::Error| FigError(format!("cannot write archive: {e}"));
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
            .map_err(|e| FigError(format!("{}: {e}", tmp.display())));
        // Closed before the rename (Windows cannot rename an open file). On wasm `File` has no
        // `Drop`, which clippy flags there.
        #[cfg_attr(target_arch = "wasm32", allow(clippy::drop_non_drop))]
        drop(file);
        let result = written.and_then(|()| {
            std::fs::rename(&tmp, path).map_err(|e| FigError(format!("{}: {e}", path.display())))
        });
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
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
            Err(e) => return fail(format!("{}: {e}", tmp.display())),
        }
    }
    fail(format!(
        "{}: cannot create a temporary file: {}",
        path.display(),
        last_err.map_or_else(|| "name clash".into(), |e| e.to_string())
    ))
}
