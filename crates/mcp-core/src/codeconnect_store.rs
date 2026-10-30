//! The local Code Connect store: `FILE.fig.codeconnect.json` next to a registered `.fig`, shared
//! by the MCP tools and the desktop app (feature `fs`).
//!
//! Every writer holds an exclusive lock on the sibling `FILE.fig.codeconnect.json.lock` (an
//! advisory `File::lock`, kept between runs) from before it reads the sidecar until it has
//! written it, so writers in several processes do not lose each other's mappings. A symlink
//! at either path is refused, and the sidecar is replaced atomically, so a reader ([`mapped`])
//! takes no lock: it never waits for a writer and creates no `.lock` file.
use crate::codeconnect_context::{component_id, component_set};
use figstudio_fig::Document;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Largest sidecar read or written.
pub const MAX_SIDECAR_BYTES: u64 = 10 * 1024 * 1024;
/// Most mappings one [`add_many`] call may hold.
pub const MAX_BATCH: usize = 100;

/// A local source binding for a component.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub component_name: String,
    pub source: String,
    pub label: String,
    pub snippet: Option<String>,
    #[serde(default, alias = "imports")]
    pub snippet_imports: Vec<String>,
}

/// Mappings by component (or component set) node ID.
pub type Mappings = BTreeMap<String, Mapping>;

/// The sidecar of the `.fig` at `fig`, and the directory holding both.
pub fn sidecar_path(fig: &Path) -> Result<(PathBuf, PathBuf), &'static str> {
    // Resolve the registered file, not a client-supplied output path.
    let fig = fig
        .canonicalize()
        .map_err(|_| "Cannot locate registered file")?;
    let parent = fig.parent().ok_or("File has no directory")?;
    let mut name = fig.file_name().ok_or("File has no name")?.to_os_string();
    name.push(".codeconnect.json");
    Ok((parent.join(name), parent.to_path_buf()))
}

/// The sidecar, locked for this value's lifetime, with its mappings loaded.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    parent: PathBuf,
    meta: Option<std::fs::Metadata>,
    pub mappings: Mappings,
    _lock: std::fs::File,
}

impl Store {
    /// Locks and reads the sidecar of `fig` (empty when it does not exist yet).
    pub fn open(fig: &Path) -> Result<Store, &'static str> {
        let (path, parent) = sidecar_path(fig)?;
        let lock = lock_sidecar(&path).map_err(|_| "Cannot lock Code Connect sidecar")?;
        let (meta, mappings) = load(&path)?;
        Ok(Store {
            path,
            parent,
            meta,
            mappings,
            _lock: lock,
        })
    }

    /// Writes the mappings back atomically, keeping the sidecar's permissions.
    pub fn save(&self) -> Result<(), &'static str> {
        write(
            &self.path,
            &self.parent,
            &self.mappings,
            self.meta.as_ref().map(std::fs::Metadata::permissions),
        )
        .map_err(|_| "Cannot atomically write Code Connect sidecar")
    }
}

/// Every mapping in the sidecar of `fig` (empty when there is none). Lock-free: writers
/// replace the sidecar by rename, so this reads either the old file or the new one whole.
pub fn mapped(fig: &Path) -> Result<Mappings, &'static str> {
    let (path, _) = sidecar_path(fig)?;
    Ok(load(&path)?.1)
}

/// When the sidecar of `fig` last changed; `None` when it does not exist.
pub fn modified(fig: &Path) -> Option<std::time::SystemTime> {
    let (path, _) = sidecar_path(fig).ok()?;
    std::fs::symlink_metadata(path).ok()?.modified().ok()
}

/// Sets (or with `None` removes) the mapping of the component behind `node_id` (a component,
/// component set or instance). Returns the component ID the mapping is stored under.
pub fn set(
    doc: &Document,
    fig: &Path,
    node_id: &str,
    mapping: Option<Mapping>,
) -> Result<String, &'static str> {
    let key = component_id(doc, node_id)?.to_string();
    if let Some(m) = &mapping {
        validate(m)?;
    }
    let mut store = Store::open(fig)?;
    match mapping {
        Some(m) => {
            store.mappings.insert(key.clone(), m);
        }
        None => {
            store.mappings.remove(&key);
        }
    }
    store.save()?;
    Ok(key)
}

/// Adds `items` (node ID, mapping) in one locked write. A mapping whose node is not a local
/// component (or instance of one), that fails validation, or whose component is already
/// mapped is listed in `failed` with the reason and left out; the others are written together.
/// Returns `{created, failed}`; `Err` only when the call itself is invalid or the sidecar
/// cannot be read or written.
pub fn add_many(
    doc: &Document,
    fig: &Path,
    items: Vec<(String, Mapping)>,
) -> Result<Value, &'static str> {
    if items.is_empty() || items.len() > MAX_BATCH {
        return Err("mappings must hold 1 to 100 entries");
    }
    let mut store = Store::open(fig)?;
    let mut created = Vec::new();
    let mut failed = Vec::new();
    for (node_id, mapping) in items {
        let result = component_id(doc, &node_id).and_then(|id| {
            validate(&mapping)?;
            let key = id.to_string();
            let set = component_set(doc, id).map(|s| s.id.to_string());
            if store.mappings.contains_key(&key)
                || set.is_some_and(|s| store.mappings.contains_key(&s))
            {
                return Err("Component is already mapped to code");
            }
            Ok(key)
        });
        match result {
            Ok(key) => {
                created.push(json!({
                    "nodeId": node_id,
                    "componentId": key,
                    "componentName": mapping.component_name,
                    "source": mapping.source,
                    "label": mapping.label,
                }));
                store.mappings.insert(key, mapping);
            }
            Err(reason) => failed.push(json!({"nodeId": node_id, "error": reason})),
        }
    }
    if !created.is_empty() {
        store.save()?;
    }
    Ok(json!({"created": created, "failed": failed}))
}

/// Checks a mapping's sizes and that its required fields are not blank.
pub fn validate(mapping: &Mapping) -> Result<(), &'static str> {
    if mapping.source.len() > 4096
        || mapping.component_name.len() > 4096
        || mapping.label.len() > 4096
        || mapping.snippet.as_ref().is_some_and(|s| s.len() > 100_000)
        || mapping.snippet_imports.len() > 100
        || mapping.snippet_imports.iter().any(|s| s.len() > 4096)
    {
        return Err("Code Connect mapping exceeds string or import limits");
    }
    if mapping.component_name.trim().is_empty()
        || mapping.source.trim().is_empty()
        || mapping.label.trim().is_empty()
    {
        return Err("componentName, source and label must not be empty");
    }
    Ok(())
}

/// The sidecar's metadata (`None` when it does not exist yet) and mappings. A writer calls it
/// with the lock held; a reader without.
fn load(path: &Path) -> Result<(Option<std::fs::Metadata>, Mappings), &'static str> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err("Code Connect sidecar must be a regular file, not a symlink");
        }
        Ok(meta) if meta.len() > MAX_SIDECAR_BYTES => {
            return Err("Code Connect sidecar exceeds 10 MiB");
        }
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((None, Mappings::new()));
        }
        Err(_) => return Err("Cannot read Code Connect sidecar"),
    };
    Ok((Some(meta), read(path)?))
}

/// An exclusive lock on the sibling `<sidecar>.lock` file, released when the handle drops.
/// `File::lock` is advisory: every writer of the sidecar takes it.
fn lock_sidecar(path: &Path) -> std::io::Result<std::fs::File> {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    let name = PathBuf::from(name);
    // A symlink at the lock path would make the lock land on another file: refuse it.
    if std::fs::symlink_metadata(&name).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(std::io::Error::other("Code Connect lock is a symlink"));
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&name)?;
    file.lock()?;
    Ok(file)
}

fn read(path: &Path) -> Result<Mappings, &'static str> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_SIDECAR_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| "Cannot read Code Connect sidecar")?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Err("Code Connect sidecar exceeds 10 MiB");
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Cannot read Code Connect sidecar: invalid JSON or mapping")
}

fn write(
    path: &Path,
    parent: &Path,
    mappings: &Mappings,
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, mappings)?;
    temp.flush()?;
    if temp.as_file().metadata()?.len() > MAX_SIDECAR_BYTES {
        return Err(std::io::Error::other("Code Connect sidecar exceeds 10 MiB"));
    }
    crate::fs::persist_file(temp, path, permissions)
}
