//! Reading `.fig` files: container, Kiwi schema, node tree, blobs and images.

#[cfg(test)]
use kiwi_schema::ByteBufferMut;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;

use kiwi_schema::{ByteBuffer, DefKind, Schema};

use crate::archive::{self, Entry};
use crate::read::Reader;
use crate::val::{Guid, Val};

pub use crate::error::FigError;

fn err<T>(msg: impl Into<String>) -> Result<T, FigError> {
    Err(FigError::malformed(msg))
}

/// Reads one file using the decoder's input cap without exposing its path in the error.
pub fn read_limited_file(path: &Path, out: &mut Vec<u8>) -> Result<(), FigError> {
    let file = std::fs::File::open(path).map_err(|e| FigError::io("cannot read file", e))?;
    read_limited(file, MAX_FILE_BYTES, out, "file").map_err(FigError::from)
}

/// Largest file [`Document::open`] reads into memory (it keeps the file several times over).
/// Bigger archives open with [`Document::open_lazy`].
pub const MAX_FILE_BYTES: usize = 1024 * 1024 * 1024;

/// Largest decompressed size of one canvas chunk or one archive member. Set from a real
/// 54 MB document whose message chunk inflates to about 169 MB; about 3x headroom above that.
pub const MAX_CHUNK_BYTES: usize = 512 * 1024 * 1024;

/// Largest total decompressed size of all archive members together, duplicates included.
pub const MAX_ARCHIVE_INFLATED_BYTES: usize = 1024 * 1024 * 1024;
const MAX_DECODED_NODES: usize = 1_000_000;

/// Byte limits used while reading a file. The public API uses the `MAX_*` constants above;
/// the limits are a parameter so boundary behaviour can be tested with small numbers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub chunk: usize,
    pub archive: usize,
}

const PRODUCTION_LIMITS: Limits = Limits {
    chunk: MAX_CHUNK_BYTES,
    archive: MAX_ARCHIVE_INFLATED_BYTES,
};

/// Why a bounded read stopped: the stream was bad, or it was longer than the limit.
enum ReadFail {
    Limit(FigError),
    Decode(FigError),
}

impl From<ReadFail> for FigError {
    fn from(e: ReadFail) -> FigError {
        match e {
            ReadFail::Limit(e) | ReadFail::Decode(e) => e,
        }
    }
}

/// Reads at most `limit` bytes into `out` and reports `ReadFail::Limit` when there are more. It
/// reads one byte past the limit to tell "exactly at the limit" from "over", and never allocates
/// beyond that.
fn read_limited(r: impl Read, limit: usize, out: &mut Vec<u8>, what: &str) -> Result<(), ReadFail> {
    // Saturating: a limit of usize::MAX must not wrap the take() length to zero.
    let mut taken = r.take((limit as u64).saturating_add(1));
    taken
        .read_to_end(out)
        .map_err(|e| ReadFail::Decode(FigError::io(format!("cannot read {what}"), e)))?;
    if out.len() > limit {
        return Err(ReadFail::Limit(FigError::limit(format!(
            "{what} inflates past the {limit}-byte limit"
        ))));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: Guid,
    pub parent: Option<Guid>,
    /// Fractional index among siblings; sorts byte-wise.
    pub position: String,
    pub kind: String,
    pub name: String,
    /// Every decoded field, including `guid` and `parentIndex`.
    pub data: Val,
}

impl Node {
    /// Builds a node from its decoded fields; `None` when it has no `guid`.
    pub fn from_data(data: Val) -> Option<Node> {
        let id = data.get("guid").and_then(Val::as_guid)?;
        let parent_index = data.get("parentIndex");
        Some(Node {
            id,
            parent: parent_index
                .and_then(|p| p.get("guid"))
                .and_then(Val::as_guid),
            position: parent_index
                .and_then(|p| p.text("position"))
                .unwrap_or("")
                .to_string(),
            kind: data.text("type").unwrap_or("UNKNOWN").to_string(),
            name: data.text("name").unwrap_or("").to_string(),
            data,
        })
    }

    /// Key used by instance overrides: `overrideKey` when present, otherwise the guid.
    pub fn override_key(&self) -> Guid {
        self.data
            .get("overrideKey")
            .and_then(Val::as_guid)
            .unwrap_or(self.id)
    }
}

/// A process-unique [`Document::instance`].
fn next_instance() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Document {
    /// Unique per opened document in this process. Caches shared between documents (decoded
    /// images) key on it, so two files using the same image hash for different bytes never
    /// share an entry. [`Document::set_nodes`] assigns a new one, so per-document indexes
    /// (the renderer's style and property tables) are rebuilt after an edit.
    pub instance: u64,
    pub prelude: String,
    pub format_version: u32,
    pub file_name: Option<String>,
    pub nodes: Vec<Node>,
    index: HashMap<Guid, usize>,
    children: HashMap<Guid, Vec<Guid>>,
    pub root: Option<Guid>,
    /// Visible pages in document order.
    pub pages: Vec<Guid>,
    /// Figma's internal-only canvases (component storage etc.).
    pub internal_pages: Vec<Guid>,
    pub blobs: Vec<Vec<u8>>,
    /// Embedded images keyed by lowercase hex SHA-1, as named inside the archive.
    pub images: HashMap<String, Vec<u8>>,
    /// Images not read yet, by hash: their archive entries (documents opened with
    /// [`Document::open_lazy`] / [`Document::from_lazy`]; empty otherwise). The caller reads
    /// an entry when the image is needed ([`archive::read_entry`] or its own byte ranges).
    pub lazy_images: HashMap<String, Entry>,
    pub thumbnail: Option<Vec<u8>>,
    /// What is needed to write the file back (see `write.rs`).
    pub(crate) source: Source,
}

/// Compression used by a canvas chunk, reused when writing the chunk back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Deflate,
    Zlib,
    Zstd,
}

/// Parts of the original file that are not part of the node tree.
#[derive(Clone, Debug, Default)]
pub(crate) struct Source {
    /// Decompressed Kiwi schema, written back unchanged.
    pub schema: Vec<u8>,
    pub codecs: Vec<Codec>,
    /// Chunks after the message, kept byte for byte.
    pub extra_chunks: Vec<Vec<u8>>,
    /// Top-level message fields other than `nodeChanges` and `blobs`.
    pub message_extra: BTreeMap<String, Val>,
    /// Whether the message had a `blobs` field (kept even when empty).
    pub had_blobs: bool,
    /// `None` for a bare `fig-kiwi` file; otherwise archive entries other than
    /// `canvas.fig`, `thumbnail.png` and `images/*`, in their original order.
    pub archive_entries: Option<Vec<(String, Vec<u8>)>>,
    /// Opened lazily: images and other entries were not read, so the file cannot be written.
    pub lazy: bool,
}

/// What an archive holds besides the canvas.
#[derive(Default)]
struct Parts {
    file_name: Option<String>,
    images: HashMap<String, Vec<u8>>,
    lazy_images: HashMap<String, Entry>,
    thumbnail: Option<Vec<u8>>,
    archive_entries: Option<Vec<(String, Vec<u8>)>>,
    lazy: bool,
}

/// Counts only: names, text, image hashes and paths stay private (see `doctor`).
impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("format_version", &self.format_version)
            .field("nodes", &self.nodes.len())
            .field("pages", &self.pages.len())
            .field("blobs", &self.blobs.len())
            .field("images", &self.images.len())
            .field("lazy_images", &self.lazy_images.len())
            .finish_non_exhaustive()
    }
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Document, FigError> {
        let mut bytes = Vec::new();
        read_limited_file(path.as_ref(), &mut bytes)?;
        Document::from_bytes(&bytes)
    }

    /// Opens an archive without reading its images: they are listed in
    /// [`Document::lazy_images`] and read when needed. Only the central directory, the canvas,
    /// `meta.json` and the thumbnail are read, so files of any size open in little memory.
    /// A bare canvas file (not an archive) is read whole.
    pub fn open_lazy(path: impl AsRef<Path>) -> Result<Document, FigError> {
        let path = path.as_ref();
        let io = |e: std::io::Error| FigError::io("cannot read file", e);
        let mut file = std::fs::File::open(path).map_err(io)?;
        let len = file.metadata().map_err(io)?.len();
        let mut magic = [0u8; 2];
        file.read_exact(&mut magic).map_err(io)?;
        if &magic != b"PK" {
            return Document::open(path);
        }
        let entries = archive::read_entries(&mut file, len)?;
        let mut eager = HashMap::new();
        for e in entries
            .iter()
            .filter(|e| archive::EAGER_ENTRIES.contains(&e.name.as_str()))
        {
            eager.insert(e.name.clone(), archive::read_entry(&mut file, e)?);
        }
        Document::from_lazy(entries, eager)
    }

    /// Builds a document from an archive's entry list ([`archive::entries`]) and the contents
    /// of its [`archive::EAGER_ENTRIES`] (`canvas.fig` is required). Images stay unread, in
    /// [`Document::lazy_images`]; other entries are dropped, so the result cannot be saved.
    pub fn from_lazy(
        entries: Vec<Entry>,
        mut eager: HashMap<String, Vec<u8>>,
    ) -> Result<Document, FigError> {
        let Some(canvas) = eager.remove("canvas.fig") else {
            return err("archive has no canvas.fig");
        };
        let mut parts = Parts {
            file_name: eager.get("meta.json").and_then(|m| meta_file_name(m)),
            thumbnail: eager.remove("thumbnail.png"),
            lazy: true,
            ..Parts::default()
        };
        for e in entries {
            if let Some(hash) = e.name.strip_prefix("images/")
                && !hash.is_empty()
                && e.size > 0
            {
                parts.lazy_images.insert(hash.to_ascii_lowercase(), e);
            }
        }
        Document::from_canvas(canvas, parts, PRODUCTION_LIMITS)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Document, FigError> {
        Document::from_bytes_with(bytes, PRODUCTION_LIMITS)
    }

    pub(crate) fn from_bytes_with(bytes: &[u8], limits: Limits) -> Result<Document, FigError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(FigError::limit(format!(
                "input is larger than {MAX_FILE_BYTES} bytes"
            )));
        }
        let mut file_name = None;
        let mut images = HashMap::new();
        let mut thumbnail = None;
        let mut archive_entries = None;
        let canvas = if bytes.starts_with(b"PK") {
            let mut entries = Vec::new();
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
                .map_err(|e| FigError::archive("not a readable .fig archive", e))?;
            let mut canvas = None;
            let mut total = 0usize;
            for i in 0..zip.len() {
                let mut entry = zip
                    .by_index(i)
                    .map_err(|e| FigError::archive("bad archive entry", e))?;
                let name = entry.name().to_string();
                let mut data = Vec::new();
                let remaining = limits.archive.saturating_sub(total);
                read_limited(&mut entry, limits.chunk.min(remaining), &mut data, &name)?;
                // Duplicate entries count too: the total is over everything inflated.
                total = total.saturating_add(data.len());
                if total > limits.archive {
                    return Err(FigError::limit(format!(
                        "archive inflates past the {}-byte limit",
                        limits.archive
                    )));
                }
                if name == "canvas.fig" {
                    canvas = Some(data);
                } else if name == "thumbnail.png" {
                    thumbnail = Some(data);
                } else if let Some(hash) = name.strip_prefix("images/") {
                    // An empty name is the directory entry itself. An empty image is kept: it is
                    // present in the file, and dropping it would change the file on save.
                    if !hash.is_empty() {
                        images.insert(hash.to_ascii_lowercase(), data);
                    }
                } else if !name.ends_with('/') {
                    if name == "meta.json" {
                        file_name = meta_file_name(&data);
                    }
                    entries.push((name, data));
                }
            }
            archive_entries = Some(entries);
            match canvas {
                Some(c) => c,
                None => return err("archive has no canvas.fig"),
            }
        } else {
            bytes.to_vec()
        };
        Document::from_canvas(
            canvas,
            Parts {
                file_name,
                images,
                thumbnail,
                archive_entries,
                ..Parts::default()
            },
            limits,
        )
    }

    fn from_canvas(canvas: Vec<u8>, parts: Parts, limits: Limits) -> Result<Document, FigError> {
        if canvas.len() < 12 {
            return err("file too short to be a .fig");
        }
        let prelude = String::from_utf8_lossy(&canvas[..8]).to_string();
        if !prelude.starts_with("fig-") {
            return err("not a .fig file (missing fig- header)");
        }
        let format_version = u32::from_le_bytes(canvas[8..12].try_into().unwrap());
        let mut chunks = Vec::new();
        let mut at = 12;
        while at + 4 <= canvas.len() {
            let len = u32::from_le_bytes(canvas[at..at + 4].try_into().unwrap()) as usize;
            at += 4;
            let Some(chunk) = canvas.get(at..at + len) else {
                return err("truncated chunk");
            };
            chunks.push(chunk);
            at += len;
        }
        if chunks.len() < 2 {
            return err("missing schema or message chunk");
        }
        let (schema_bytes, schema_codec) = decompress(chunks[0], limits.chunk)?;
        let schema = Schema::decode(&schema_bytes)
            .map_err(|()| FigError::malformed("cannot decode the file schema"))?;
        let message_def = schema
            .defs
            .iter()
            .position(|d| d.name == "Message" && d.kind == DefKind::Message)
            .ok_or_else(|| FigError::malformed("schema has no Message type"))?;
        if let Some(field) = schema.defs[message_def].field("blobs") {
            validate_blobs_field(&schema, field)?;
        }
        let (message_bytes, message_codec) = decompress(chunks[1], limits.chunk)?;
        let decode_err =
            |e: String| FigError::malformed(format!("cannot decode the document: {e}"));
        let reader = Reader::new(&schema).map_err(decode_err)?;
        let def = &schema.defs[message_def];
        let mut bb = ByteBuffer::new(&message_bytes);

        let mut message_extra = BTreeMap::new();
        let mut had_blobs = false;
        let mut blobs = Vec::new();
        let mut nodes: Vec<Node> = Vec::new();
        let mut index = HashMap::new();
        let mut saw_nodes = false;
        loop {
            let tag = bb
                .read_var_uint()
                .map_err(|()| decode_err("truncated".into()))?;
            if tag == 0 {
                break;
            }
            let Some(&fi) = def.field_value_to_index.get(&tag) else {
                return Err(decode_err(format!("Message has no field {tag}")));
            };
            let field = &def.fields[fi];
            match field.name.as_str() {
                "nodeChanges" if field.is_array => {
                    saw_nodes = true;
                    // Nodes are decoded one at a time so only one tree exists in memory.
                    let len = reader.len(&mut bb).map_err(decode_err)?;
                    if len > MAX_DECODED_NODES.saturating_sub(nodes.len()) {
                        return Err(decode_err("too many node records".into()));
                    }
                    nodes.reserve(len);
                    index.reserve(len);
                    for _ in 0..len {
                        let data = reader
                            .value(&mut bb, field.type_id, 1)
                            .map_err(decode_err)?;
                        let Some(node) = Node::from_data(data) else {
                            return Err(decode_err("node record has no valid guid".into()));
                        };
                        if let Some(&existing) = index.get(&node.id) {
                            // Later changes win, as Figma applies them in order.
                            nodes[existing] = node;
                        } else {
                            index.insert(node.id, nodes.len());
                            nodes.push(node);
                        }
                    }
                }
                "blobs" if field.is_array => {
                    had_blobs = true;
                    let list = reader.field(&mut bb, field, 1).map_err(decode_err)?;
                    let Val::Arr(items) = list else {
                        return Err(decode_err("blobs is not an array".into()));
                    };
                    blobs = items
                        .into_iter()
                        .map(blob_bytes)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(decode_err)?;
                }
                name => {
                    let v = reader.field(&mut bb, field, 1).map_err(decode_err)?;
                    message_extra.insert(name.to_string(), v);
                }
            }
        }
        drop(message_bytes);
        if !saw_nodes {
            return err("document has no nodes");
        }
        nodes.shrink_to_fit();
        check_parent_links(&nodes, &index)?;

        let mut doc = Document {
            instance: next_instance(),
            prelude,
            format_version,
            file_name: parts.file_name,
            nodes,
            index,
            children: HashMap::new(),
            root: None,
            pages: Vec::new(),
            internal_pages: Vec::new(),
            blobs,
            images: parts.images,
            lazy_images: parts.lazy_images,
            thumbnail: parts.thumbnail,
            source: Source {
                schema: schema_bytes,
                codecs: vec![schema_codec, message_codec],
                extra_chunks: chunks[2..].iter().map(|c| c.to_vec()).collect(),
                message_extra,
                had_blobs,
                archive_entries: parts.archive_entries,
                lazy: parts.lazy,
            },
        };
        doc.reindex();
        Ok(doc)
    }

    /// Replaces the node list (keeping order) and rebuilds the lookup tables.
    /// Later nodes with an id that already appeared replace the earlier one.
    /// Fails, leaving the document unchanged, when the parent links form a cycle.
    pub fn set_nodes(&mut self, nodes: Vec<Node>) -> Result<(), FigError> {
        let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
        let mut index = HashMap::with_capacity(nodes.len());
        for node in nodes {
            if let Some(&existing) = index.get(&node.id) {
                out[existing] = node;
            } else {
                index.insert(node.id, out.len());
                out.push(node);
            }
        }
        check_parent_links(&out, &index)?;
        self.nodes = out;
        self.index = index;
        self.instance = next_instance();
        self.reindex();
        Ok(())
    }

    /// Rebuilds children lists, root and pages from `nodes` and `index`.
    fn reindex(&mut self) {
        let (nodes, index) = (&self.nodes, &self.index);
        let mut children: HashMap<Guid, Vec<Guid>> = HashMap::new();
        for node in nodes {
            if let Some(parent) = node.parent {
                children.entry(parent).or_default().push(node.id);
            }
        }
        for list in children.values_mut() {
            list.sort_by(|a, b| {
                let (na, nb) = (&nodes[index[a]], &nodes[index[b]]);
                na.position
                    .as_bytes()
                    .cmp(nb.position.as_bytes())
                    .then(a.cmp(b))
            });
        }

        let root = nodes.iter().find(|n| n.kind == "DOCUMENT").map(|n| n.id);
        let mut pages = Vec::new();
        let mut internal_pages = Vec::new();
        if let Some(root) = root {
            for id in children.get(&root).into_iter().flatten() {
                let node = &nodes[index[id]];
                if node.kind != "CANVAS" {
                    continue;
                }
                if node.data.flag("internalOnly") == Some(true) {
                    internal_pages.push(*id);
                } else {
                    pages.push(*id);
                }
            }
        }
        self.children = children;
        self.root = root;
        self.pages = pages;
        self.internal_pages = internal_pages;
    }

    pub fn node(&self, id: Guid) -> Option<&Node> {
        self.index.get(&id).map(|&i| &self.nodes[i])
    }

    pub fn children(&self, id: Guid) -> &[Guid] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The page (canvas) a node lives on.
    pub fn page_of(&self, id: Guid) -> Option<Guid> {
        let mut cur = Some(id);
        let mut seen = std::collections::HashSet::new();
        while let Some(current) = cur {
            if !seen.insert(current) {
                return None;
            }
            let node = self.node(current)?;
            if node.kind == "CANVAS" {
                return Some(current);
            }
            cur = node.parent;
        }
        None
    }

    /// Number of nodes in the subtree below `id`, excluding `id`.
    pub fn descendant_count(&self, id: Guid) -> usize {
        let mut stack: Vec<Guid> = self.children(id).to_vec();
        let mut seen = std::collections::HashSet::new();
        let mut count = 0;
        while let Some(c) = stack.pop() {
            if !seen.insert(c) {
                continue;
            }
            count += 1;
            stack.extend_from_slice(self.children(c));
        }
        count
    }
}

fn validate_blobs_field(schema: &Schema, field: &kiwi_schema::Field) -> Result<(), FigError> {
    let blob = usize::try_from(field.type_id)
        .ok()
        .and_then(|i| schema.defs.get(i));
    let supported = field.is_array
        && blob.is_some_and(|d| {
            d.kind == DefKind::Struct
                && d.fields.len() == 1
                && d.fields[0].name == "bytes"
                && d.fields[0].is_array
                && d.fields[0].type_id == kiwi_schema::TYPE_BYTE
        });
    if supported {
        Ok(())
    } else {
        err("unsupported blobs schema layout")
    }
}

/// The bytes of one blob. An empty `bytes` is a valid empty blob. A blob with no `bytes` field,
/// or one whose field is not bytes, is refused rather than replaced by empty data on save.
fn blob_bytes(blob: Val) -> Result<Vec<u8>, String> {
    let Val::Obj(mut o) = blob else {
        return Err("a blob is not an object".into());
    };
    match o.remove("bytes") {
        Some(Val::Bytes(bytes)) => Ok(bytes),
        Some(_) => Err("a blob's bytes field is not a byte array".into()),
        None => Err("a blob has no bytes field".into()),
    }
}

/// Rejects parent links that form a cycle, including cycles no page reaches. Iterative, so a
/// long chain cannot overflow the stack. Each node is walked at most once per start point.
fn check_parent_links(nodes: &[Node], index: &HashMap<Guid, usize>) -> Result<(), FigError> {
    // 0 = unvisited, 1 = on the current path, 2 = known to reach no cycle.
    let mut state = vec![0u8; nodes.len()];
    let mut path: Vec<usize> = Vec::new();
    for start in 0..nodes.len() {
        if state[start] != 0 {
            continue;
        }
        let mut cur = Some(start);
        while let Some(i) = cur {
            if state[i] == 2 {
                break;
            }
            if state[i] == 1 {
                return err(format!(
                    "node {} is its own ancestor (parent cycle)",
                    nodes[i].id
                ));
            }
            state[i] = 1;
            path.push(i);
            cur = nodes[i].parent.and_then(|p| index.get(&p).copied());
        }
        for i in path.drain(..) {
            state[i] = 2;
        }
    }
    Ok(())
}

#[cfg(test)]
fn synthetic_schema(blob_fields: Vec<kiwi_schema::Field>, has_blobs: bool) -> Schema {
    use kiwi_schema::{Def, DefKind, Field};
    Schema::new(vec![
        Def::new("Blob".into(), DefKind::Struct, blob_fields),
        Def::new(
            "Guid".into(),
            DefKind::Struct,
            vec![
                Field {
                    name: "sessionID".into(),
                    type_id: kiwi_schema::TYPE_UINT,
                    is_array: false,
                    value: 0,
                },
                Field {
                    name: "localID".into(),
                    type_id: kiwi_schema::TYPE_UINT,
                    is_array: false,
                    value: 0,
                },
            ],
        ),
        Def::new(
            "Node".into(),
            DefKind::Message,
            vec![
                Field {
                    name: "guid".into(),
                    type_id: 1,
                    is_array: false,
                    value: 1,
                },
                Field {
                    name: "wide".into(),
                    type_id: kiwi_schema::TYPE_UINT64,
                    is_array: false,
                    value: 2,
                },
                Field {
                    name: "future".into(),
                    type_id: kiwi_schema::TYPE_STRING,
                    is_array: false,
                    value: 3,
                },
            ],
        ),
        Def::new(
            "Message".into(),
            DefKind::Message,
            vec![
                Field {
                    name: "nodeChanges".into(),
                    type_id: 2,
                    is_array: true,
                    value: 1,
                },
                if has_blobs {
                    Field {
                        name: "blobs".into(),
                        type_id: 0,
                        is_array: true,
                        value: 2,
                    }
                } else {
                    Field {
                        name: "placeholder".into(),
                        type_id: kiwi_schema::TYPE_STRING,
                        is_array: false,
                        value: 2,
                    }
                },
                Field {
                    name: "extra".into(),
                    type_id: kiwi_schema::TYPE_STRING,
                    is_array: false,
                    value: 3,
                },
            ],
        ),
    ])
}

#[cfg(test)]
fn synthetic_file(schema: &Schema, blob_mode: u8) -> Vec<u8> {
    synthetic_file_with_node(schema, blob_mode, true)
}

#[cfg(test)]
fn synthetic_file_with_node(schema: &Schema, blob_mode: u8, include_guid: bool) -> Vec<u8> {
    let mut msg = ByteBufferMut::new();
    msg.write_var_uint(1);
    msg.write_var_uint(1);
    if include_guid {
        msg.write_var_uint(1);
        msg.write_var_uint(7);
        msg.write_var_uint(9);
    }
    msg.write_var_uint(2);
    msg.write_var_uint64(u64::MAX);
    if schema
        .def("Node")
        .is_some_and(|d| d.field("future").is_some())
    {
        msg.write_var_uint(3);
        msg.write_string("future");
    }
    msg.write_byte(0);
    if schema
        .def("Message")
        .is_some_and(|d| d.field("extra").is_some())
    {
        msg.write_var_uint(3);
        msg.write_string("top");
    }
    if schema
        .def("Message")
        .is_some_and(|d| d.field("blobs").is_some())
    {
        msg.write_var_uint(2);
        msg.write_var_uint(1);
        msg.write_var_uint(if blob_mode == 1 {
            0
        } else if blob_mode == 2 {
            2
        } else {
            1
        });
        if blob_mode == 0 {
            msg.write_byte(77);
        }
    }
    msg.write_byte(0);
    let s = compressed(&schema.encode());
    let m = compressed(&msg.data());
    let mut file = b"fig-kiwi".to_vec();
    file.extend_from_slice(&1u32.to_le_bytes());
    for chunk in [s, m] {
        file.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        file.extend_from_slice(&chunk);
    }
    file
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use kiwi_schema::{Def, DefKind, Field, TYPE_BYTE, TYPE_UINT64};

    #[test]
    fn rejects_oversized_schemas_before_interning_identifiers() {
        let schema = Schema::new(vec![Def::new(
            "Message".into(),
            DefKind::Message,
            (0..4097)
                .map(|i| Field {
                    name: format!("oversized-schema-field-{i}"),
                    type_id: kiwi_schema::TYPE_STRING,
                    is_array: false,
                    value: i,
                })
                .collect(),
        )]);
        let bytes = synthetic_file_with_node(&schema, 1, true);
        assert!(Document::from_bytes(&bytes).is_err());
    }

    #[test]
    fn rejects_node_records_without_ids_instead_of_dropping_them() {
        let schema = synthetic_schema(Vec::new(), false);
        let result = Document::from_bytes(&synthetic_file_with_node(&schema, 1, false));
        assert!(
            matches!(result, Err(e) if e.to_string().contains("node record has no valid guid"))
        );
    }

    #[test]
    fn rejects_node_lists_over_the_decoded_item_limit_before_reserving() {
        let schema = synthetic_schema(Vec::new(), false);
        let mut msg = ByteBufferMut::new();
        msg.write_var_uint(1);
        msg.write_var_uint(1_000_001);
        for _ in 0..1_000_001 {
            msg.write_byte(0);
        }
        let schema_chunk = compressed(&schema.encode());
        let message_chunk = compressed(&msg.data());
        let mut file = b"fig-kiwi".to_vec();
        file.extend_from_slice(&1u32.to_le_bytes());
        for chunk in [schema_chunk, message_chunk] {
            file.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            file.extend_from_slice(&chunk);
        }
        assert!(matches!(
            Document::from_bytes(&file),
            Err(error) if error.to_string().contains("too many node records")
        ));
    }

    #[test]
    fn validates_blob_shapes_and_roundtrips_uint64_through_the_codec() {
        let valid = synthetic_schema(
            vec![Field {
                name: "bytes".into(),
                type_id: TYPE_BYTE,
                is_array: true,
                value: 0,
            }],
            true,
        );
        let bytes = synthetic_file(&valid, 0);
        let doc = Document::from_bytes(&bytes).unwrap();
        let roundtrip = Document::from_bytes(&doc.to_bytes().unwrap()).unwrap();
        assert_eq!(roundtrip.blobs, vec![vec![77]]);
        assert_eq!(roundtrip.nodes[0].data.text("future"), Some("future"));
        assert_eq!(roundtrip.nodes[0].data.get("wide"), Some(&Val::Int(-1)));
        assert_eq!(
            roundtrip.source.message_extra.get("extra"),
            Some(&Val::Str("top".into()))
        );
        let empty = Document::from_bytes(&synthetic_file(&valid, 1)).unwrap();
        assert_eq!(empty.blobs, vec![Vec::<u8>::new()]);
        assert!(Document::from_bytes(&synthetic_file(&valid, 2)).is_err());
        let absent_schema = synthetic_schema(Vec::new(), false);
        let absent_result = Document::from_bytes(&synthetic_file(&absent_schema, 0));
        assert!(
            absent_result.is_ok(),
            "absent blobs schema should decode: {:?}",
            absent_result.err()
        );
        let mut scalar = valid
            .defs
            .iter()
            .map(|d| {
                Def::new(
                    d.name.clone(),
                    match d.kind {
                        DefKind::Struct => DefKind::Struct,
                        DefKind::Message => DefKind::Message,
                        DefKind::Enum => DefKind::Enum,
                    },
                    d.fields
                        .iter()
                        .map(|f| Field {
                            name: f.name.clone(),
                            type_id: f.type_id,
                            is_array: f.is_array,
                            value: f.value,
                        })
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        scalar[3].fields[1].is_array = false;
        assert!(Document::from_bytes(&synthetic_file(&Schema::new(scalar), 0)).is_err());
        let mut malformed = valid
            .defs
            .iter()
            .map(|d| {
                Def::new(
                    d.name.clone(),
                    match d.kind {
                        DefKind::Struct => DefKind::Struct,
                        DefKind::Message => DefKind::Message,
                        DefKind::Enum => DefKind::Enum,
                    },
                    d.fields
                        .iter()
                        .map(|f| Field {
                            name: f.name.clone(),
                            type_id: f.type_id,
                            is_array: f.is_array,
                            value: f.value,
                        })
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        malformed[0].fields[0].is_array = false;
        assert!(Document::from_bytes(&synthetic_file(&Schema::new(malformed), 0)).is_err());
        let mut uint64 = valid
            .defs
            .iter()
            .map(|d| {
                Def::new(
                    d.name.clone(),
                    match d.kind {
                        DefKind::Struct => DefKind::Struct,
                        DefKind::Message => DefKind::Message,
                        DefKind::Enum => DefKind::Enum,
                    },
                    d.fields
                        .iter()
                        .map(|f| Field {
                            name: f.name.clone(),
                            type_id: f.type_id,
                            is_array: f.is_array,
                            value: f.value,
                        })
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        uint64[2].fields[1].type_id = TYPE_UINT64;
        let schema = Schema::new(uint64);
        let mut msg = ByteBufferMut::new();
        msg.write_var_uint(1);
        msg.write_var_uint(1);
        msg.write_var_uint(1);
        msg.write_var_uint(7);
        msg.write_var_uint(9);
        msg.write_var_uint(2);
        msg.write_var_uint64(u64::MAX);
        msg.write_var_uint(3);
        msg.write_string("future");
        msg.write_byte(0);
        msg.write_var_uint(3);
        msg.write_string("top");
        msg.write_byte(0);
        msg.write_byte(0);
        let mut file = b"fig-kiwi".to_vec();
        file.extend_from_slice(&1u32.to_le_bytes());
        let s = compressed(&schema.encode());
        let m = compressed(&msg.data());
        for chunk in [s, m] {
            file.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            file.extend_from_slice(&chunk);
        }
        let decoded = Document::from_bytes(&file).unwrap();
        assert_eq!(decoded.nodes[0].data.get("wide"), Some(&Val::Int(-1)));
        assert_eq!(
            Document::from_bytes(&decoded.to_bytes().unwrap())
                .unwrap()
                .nodes[0]
                .data
                .get("wide"),
            Some(&Val::Int(-1))
        );
    }
}

fn meta_file_name(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    // meta.json is small and flat; avoid a JSON dependency for one string field.
    let start = text.find("\"file_name\"")?;
    let rest = &text[start + "\"file_name\"".len()..];
    let rest = rest
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let code: String = chars.by_ref().take(4).collect();
                    out.push(
                        char::from_u32(u32::from_str_radix(&code, 16).ok()?).unwrap_or('\u{fffd}'),
                    );
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
fn compressed(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn decompress(data: &[u8], limit: usize) -> Result<(Vec<u8>, Codec), FigError> {
    let mut out = Vec::new();
    if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let decoder = ruzstd::decoding::StreamingDecoder::new(data)
            .map_err(|e| FigError::codec("bad zstd chunk", e))?;
        read_limited(decoder, limit, &mut out, "zstd chunk")?;
        return Ok((out, Codec::Zstd));
    }
    // Deflate is tried first. A stream that is not deflate fails with a decode error, and only
    // then is zlib tried. Hitting the size limit is final and never falls through.
    let mut deflate = flate2::read::DeflateDecoder::new(data);
    match read_limited(&mut deflate, limit, &mut out, "deflate chunk") {
        Ok(()) => return Ok((out, Codec::Deflate)),
        Err(ReadFail::Limit(e)) => return Err(e),
        Err(ReadFail::Decode(_)) => out.clear(),
    }
    read_limited(
        flate2::read::ZlibDecoder::new(data),
        limit,
        &mut out,
        "zlib chunk",
    )?;
    Ok((out, Codec::Zlib))
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    fn structure_bytes() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/structure.fig"
        );
        std::fs::read(path).unwrap()
    }

    /// The largest single inflated item: a zip member, or a canvas chunk inside `canvas.fig`.
    /// The per-item limit applies to both, so the boundary is the largest of either.
    fn largest_inflated_item(bytes: &[u8]) -> usize {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut largest = 0;
        let mut canvas = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).unwrap();
            let is_canvas = entry.name() == "canvas.fig";
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            largest = largest.max(buf.len());
            if is_canvas {
                canvas = buf;
            }
        }
        let mut at = 12;
        while at + 4 <= canvas.len() {
            let len = u32::from_le_bytes(canvas[at..at + 4].try_into().unwrap()) as usize;
            let (inflated, _) = decompress(&canvas[at + 4..at + 4 + len], usize::MAX).unwrap();
            largest = largest.max(inflated.len());
            at += 4 + len;
        }
        largest
    }

    /// Sum of every archive member's inflated size, duplicates included, as `from_bytes` counts.
    fn archive_total(bytes: &[u8]) -> usize {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut total = 0;
        for i in 0..zip.len() {
            let mut buf = Vec::new();
            zip.by_index(i).unwrap().read_to_end(&mut buf).unwrap();
            total += buf.len();
        }
        total
    }

    #[test]
    fn a_chunk_exactly_at_the_limit_is_accepted_and_one_more_byte_is_refused() {
        let bytes = structure_bytes();
        let size = largest_inflated_item(&bytes);
        let at = Limits {
            chunk: size,
            archive: usize::MAX,
        };
        let r = Document::from_bytes_with(&bytes, at);
        assert!(
            r.is_ok(),
            "at limit size={size}: {}",
            r.err().map(|e| e.to_string()).unwrap_or_default()
        );
        let under = Limits {
            chunk: size - 1,
            archive: usize::MAX,
        };
        let err = Document::from_bytes_with(&bytes, under).expect_err("over limit");
        assert!(err.to_string().contains("limit"), "{err}");
    }

    #[test]
    fn bounded_file_reader_stops_after_limit_plus_one_even_if_file_is_larger() {
        let mut out = Vec::new();
        let result = read_limited(&b"123456789"[..], 4, &mut out, "test");
        assert!(matches!(result, Err(ReadFail::Limit(_))));
        assert_eq!(out.len(), 5);
        let file =
            std::env::temp_dir().join(format!("figstudio-open-limit-{}", std::process::id()));
        std::fs::write(&file, b"123456789").unwrap();
        assert!(Document::open(&file).is_err());
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn archive_total_counts_duplicate_entries() {
        let bytes = structure_bytes();
        let total = archive_total(&bytes);
        let exact = Limits {
            chunk: usize::MAX,
            archive: total,
        };
        assert!(
            Document::from_bytes_with(&bytes, exact).is_ok(),
            "total at limit"
        );
        let short = Limits {
            chunk: usize::MAX,
            archive: total - 1,
        };
        assert!(
            Document::from_bytes_with(&bytes, short).is_err(),
            "total over limit"
        );
    }

    // Measured on a real 54 MB file: one chunk inflates to 169,350,032 bytes and the archive
    // members total 54,397,544. A compile-time check, so lowering a limit fails the build.
    const _: () = assert!(MAX_CHUNK_BYTES >= 169_350_032 + 1024);
    const _: () = assert!(MAX_ARCHIVE_INFLATED_BYTES >= 54_397_544);
}
