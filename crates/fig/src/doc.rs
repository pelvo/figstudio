//! Reading `.fig` files: container, Kiwi schema, node tree, blobs and images.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;

use kiwi_schema::{ByteBuffer, DefKind, Schema};

use crate::read::Reader;
use crate::val::{Guid, Val};

#[derive(Debug)]
pub struct FigError(pub String);

impl fmt::Display for FigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FigError {}

fn err<T>(msg: impl Into<String>) -> Result<T, FigError> {
    Err(FigError(msg.into()))
}

/// Largest file accepted. Decoding keeps the whole file in memory several times over.
pub const MAX_FILE_BYTES: usize = 1024 * 1024 * 1024;

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

pub struct Document {
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
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Document, FigError> {
        let path = path.as_ref();
        let meta =
            std::fs::metadata(path).map_err(|e| FigError(format!("{}: {e}", path.display())))?;
        if meta.len() as usize > MAX_FILE_BYTES {
            return err(format!(
                "{} is larger than {MAX_FILE_BYTES} bytes",
                path.display()
            ));
        }
        let bytes =
            std::fs::read(path).map_err(|e| FigError(format!("{}: {e}", path.display())))?;
        Document::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Document, FigError> {
        let mut file_name = None;
        let mut images = HashMap::new();
        let mut thumbnail = None;
        let mut archive_entries = None;
        let canvas = if bytes.starts_with(b"PK") {
            let mut entries = Vec::new();
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
                .map_err(|e| FigError(format!("not a readable .fig archive: {e}")))?;
            let mut canvas = None;
            for i in 0..zip.len() {
                let mut entry = zip
                    .by_index(i)
                    .map_err(|e| FigError(format!("bad archive entry: {e}")))?;
                let name = entry.name().to_string();
                let mut data = Vec::new();
                entry
                    .read_to_end(&mut data)
                    .map_err(|e| FigError(format!("cannot read {name}: {e}")))?;
                if name == "canvas.fig" {
                    canvas = Some(data);
                } else if name == "thumbnail.png" {
                    thumbnail = Some(data);
                } else if let Some(hash) = name.strip_prefix("images/") {
                    if !hash.is_empty() && !data.is_empty() {
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
        let (schema_bytes, schema_codec) = decompress(chunks[0])?;
        let schema = Schema::decode(&schema_bytes)
            .map_err(|()| FigError("cannot decode the file schema".into()))?;
        let message_def = schema
            .defs
            .iter()
            .position(|d| d.name == "Message" && d.kind == DefKind::Message)
            .ok_or_else(|| FigError("schema has no Message type".into()))?;
        let (message_bytes, message_codec) = decompress(chunks[1])?;
        let reader = Reader::new(&schema);
        let def = &schema.defs[message_def];
        let mut bb = ByteBuffer::new(&message_bytes);
        let decode_err = |e: String| FigError(format!("cannot decode the document: {e}"));

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
                    nodes.reserve(len);
                    index.reserve(len);
                    for _ in 0..len {
                        let data = reader
                            .value(&mut bb, field.type_id, 1)
                            .map_err(decode_err)?;
                        let Some(node) = Node::from_data(data) else {
                            continue;
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
                    blobs = match list {
                        Val::Arr(items) => items
                            .into_iter()
                            .map(|b| match b {
                                Val::Obj(mut o) => match o.remove("bytes") {
                                    Some(Val::Bytes(bytes)) => bytes,
                                    _ => Vec::new(),
                                },
                                _ => Vec::new(),
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
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

        let mut doc = Document {
            prelude,
            format_version,
            file_name,
            nodes,
            index,
            children: HashMap::new(),
            root: None,
            pages: Vec::new(),
            internal_pages: Vec::new(),
            blobs,
            images,
            thumbnail,
            source: Source {
                schema: schema_bytes,
                codecs: vec![schema_codec, message_codec],
                extra_chunks: chunks[2..].iter().map(|c| c.to_vec()).collect(),
                message_extra,
                had_blobs,
                archive_entries,
            },
        };
        doc.reindex();
        Ok(doc)
    }

    /// Replaces the node list (keeping order) and rebuilds the lookup tables.
    /// Later nodes with an id that already appeared replace the earlier one.
    pub fn set_nodes(&mut self, nodes: Vec<Node>) {
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
        self.nodes = out;
        self.index = index;
        self.reindex();
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
    pub fn page_of(&self, mut id: Guid) -> Option<Guid> {
        loop {
            let node = self.node(id)?;
            if node.kind == "CANVAS" {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    /// Number of nodes in the subtree below `id`, excluding `id`.
    pub fn descendant_count(&self, id: Guid) -> usize {
        self.children(id)
            .iter()
            .map(|c| 1 + self.descendant_count(*c))
            .sum()
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

fn decompress(data: &[u8]) -> Result<(Vec<u8>, Codec), FigError> {
    let mut out = Vec::new();
    if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(data)
            .map_err(|e| FigError(format!("bad zstd chunk: {e}")))?;
        decoder
            .read_to_end(&mut out)
            .map_err(|e| FigError(format!("bad zstd chunk: {e}")))?;
        return Ok((out, Codec::Zstd));
    }
    if flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .is_ok()
    {
        return Ok((out, Codec::Deflate));
    }
    out.clear();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| FigError(format!("cannot decompress chunk: {e}")))?;
    Ok((out, Codec::Zlib))
}
