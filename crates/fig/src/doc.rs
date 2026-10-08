//! Reading `.fig` files: container, Kiwi schema, node tree, blobs and images.

use std::collections::HashMap;
use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;

use kiwi_schema::{Schema, Value};

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
    /// Key used by instance overrides: `overrideKey` when present, otherwise the guid.
    pub fn override_key(&self) -> Guid {
        self.data.get("overrideKey").and_then(Val::as_guid).unwrap_or(self.id)
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
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Document, FigError> {
        let path = path.as_ref();
        let meta = std::fs::metadata(path).map_err(|e| FigError(format!("{}: {e}", path.display())))?;
        if meta.len() as usize > MAX_FILE_BYTES {
            return err(format!("{} is larger than {MAX_FILE_BYTES} bytes", path.display()));
        }
        let bytes = std::fs::read(path).map_err(|e| FigError(format!("{}: {e}", path.display())))?;
        Document::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Document, FigError> {
        let mut file_name = None;
        let mut images = HashMap::new();
        let mut thumbnail = None;
        let canvas = if bytes.starts_with(b"PK") {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
                .map_err(|e| FigError(format!("not a readable .fig archive: {e}")))?;
            let mut canvas = None;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| FigError(format!("bad archive entry: {e}")))?;
                let name = entry.name().to_string();
                let mut data = Vec::new();
                entry
                    .read_to_end(&mut data)
                    .map_err(|e| FigError(format!("cannot read {name}: {e}")))?;
                if name == "canvas.fig" {
                    canvas = Some(data);
                } else if name == "thumbnail.png" {
                    thumbnail = Some(data);
                } else if name == "meta.json" {
                    file_name = meta_file_name(&data);
                } else if let Some(hash) = name.strip_prefix("images/") {
                    if !hash.is_empty() && !data.is_empty() {
                        images.insert(hash.to_ascii_lowercase(), data);
                    }
                }
            }
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
        let schema_bytes = decompress(chunks[0])?;
        let schema = Schema::decode(&schema_bytes).map_err(|()| FigError("cannot decode the file schema".into()))?;
        let message_type = schema
            .defs
            .iter()
            .position(|d| d.name == "Message")
            .ok_or_else(|| FigError("schema has no Message type".into()))? as i32;
        let message_bytes = decompress(chunks[1])?;
        let message = Value::decode(&schema, message_type, &message_bytes)
            .map_err(|()| FigError("cannot decode the document message".into()))?;

        let blobs = match message.get("blobs") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|b| match b.get("bytes") {
                    Some(Value::Array(bytes)) => bytes.iter().map(Value::as_byte).collect(),
                    _ => Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        };
        let Some(Value::Array(changes)) = message.get("nodeChanges") else {
            return err("document has no nodes");
        };

        let mut nodes = Vec::with_capacity(changes.len());
        let mut index = HashMap::with_capacity(changes.len());
        for change in changes {
            let data = Val::from_kiwi(change);
            let Some(id) = data.get("guid").and_then(Val::as_guid) else {
                continue;
            };
            let parent_index = data.get("parentIndex");
            let node = Node {
                id,
                parent: parent_index.and_then(|p| p.get("guid")).and_then(Val::as_guid),
                position: parent_index.and_then(|p| p.text("position")).unwrap_or("").to_string(),
                kind: data.text("type").unwrap_or("UNKNOWN").to_string(),
                name: data.text("name").unwrap_or("").to_string(),
                data,
            };
            if let Some(&existing) = index.get(&id) {
                // Later changes win, as Figma applies them in order.
                nodes[existing] = node;
            } else {
                index.insert(id, nodes.len());
                nodes.push(node);
            }
        }

        let mut children: HashMap<Guid, Vec<Guid>> = HashMap::new();
        for node in &nodes {
            if let Some(parent) = node.parent {
                children.entry(parent).or_default().push(node.id);
            }
        }
        for list in children.values_mut() {
            list.sort_by(|a, b| {
                let (na, nb) = (&nodes[index[a]], &nodes[index[b]]);
                na.position.as_bytes().cmp(nb.position.as_bytes()).then(a.cmp(b))
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

        Ok(Document {
            prelude,
            format_version,
            file_name,
            nodes,
            index,
            children,
            root,
            pages,
            internal_pages,
            blobs,
            images,
            thumbnail,
        })
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
        self.children(id).iter().map(|c| 1 + self.descendant_count(*c)).sum()
    }
}

fn meta_file_name(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    // meta.json is small and flat; avoid a JSON dependency for one string field.
    let start = text.find("\"file_name\"")?;
    let rest = &text[start + "\"file_name\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
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
                    out.push(char::from_u32(u32::from_str_radix(&code, 16).ok()?).unwrap_or('\u{fffd}'));
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, FigError> {
    let mut out = Vec::new();
    if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(data)
            .map_err(|e| FigError(format!("bad zstd chunk: {e}")))?;
        decoder
            .read_to_end(&mut out)
            .map_err(|e| FigError(format!("bad zstd chunk: {e}")))?;
        return Ok(out);
    }
    if flate2::read::DeflateDecoder::new(data).read_to_end(&mut out).is_ok() {
        return Ok(out);
    }
    out.clear();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| FigError(format!("cannot decompress chunk: {e}")))?;
    Ok(out)
}
