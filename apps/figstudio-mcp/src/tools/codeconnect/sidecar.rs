use super::{MapInput, Mapping, Mappings, fault};
use crate::node_id::{NodeId, parse_node_id};
use figstudio_fig::Document;
use figstudio_mcp_core::codeconnect_context::{component_id, component_set};
use rmcp::model::CallToolResult;
use std::{
    io::{Read, Write},
    path::Path,
};

// Bound sidecar and user strings before allocating or persisting them.
const MAX_SIDECAR_BYTES: u64 = 10 * 1024 * 1024;

pub(super) fn map(
    doc: &Document,
    path: &Path,
    input: MapInput,
    add: Option<Mapping>,
) -> CallToolResult {
    let requested = match input.node_id.as_deref().map(parse_node_id).transpose() {
        Ok(id) => id.map(|id| match id {
            NodeId::Node(id) => id.to_string(),
            NodeId::InstancePath(path) => path.to_string(),
        }),
        Err(_) => return fault("Invalid node ID"),
    };
    let id = match input
        .node_id
        .as_deref()
        .map(|id| component_id(doc, id))
        .transpose()
    {
        Ok(id) => id,
        Err(message) => return fault(message),
    };
    // Resolve the registered file, not a client-supplied output path.
    let Ok(path) = path.canonicalize() else {
        return fault("Cannot locate registered file");
    };
    let Some(parent) = path.parent() else {
        return fault("File has no directory");
    };
    let Some(name) = path.file_name() else {
        return fault("File has no name");
    };
    let mut name = name.to_os_string();
    name.push(".codeconnect.json");
    let path = parent.join(name);
    // Held until this function returns, so the read below and the write are one step across
    // processes; the in-process mutex in `codeconnect.rs` covers threads of this server.
    let _lock = match lock_sidecar(&path) {
        Ok(lock) => lock,
        Err(_) => return fault("Cannot lock Code Connect sidecar"),
    };
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return fault("Code Connect sidecar must be a regular file, not a symlink");
        }
        Ok(meta) if meta.len() > MAX_SIDECAR_BYTES => {
            return fault("Code Connect sidecar exceeds 10 MiB");
        }
        Ok(meta) => Some(meta),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return fault("Cannot read Code Connect sidecar"),
    };
    let mut mappings = if meta.is_some() {
        match read(&path) {
            Ok(mappings) => mappings,
            Err(message) => return fault(message),
        }
    } else {
        Mappings::new()
    };
    let is_add = add.is_some();
    if let Some(mapping) = add {
        let Some(id) = id.as_ref() else {
            return fault("nodeId is required");
        };
        if let Err(message) = validate(&mapping) {
            return fault(message);
        }
        mappings.insert(id.to_string(), mapping);
        if write(
            &path,
            parent,
            &mappings,
            meta.map(|meta| meta.permissions()),
        )
        .is_err()
        {
            return fault("Cannot atomically write Code Connect sidecar");
        }
    }
    let output_id = if is_add {
        id.map(|id| id.to_string())
    } else {
        requested
    };
    if let (Some(requested), Some(id)) = (output_id, id) {
        let candidates = [
            Some(requested.clone()),
            Some(id.to_string()),
            component_set(doc, id).map(|set| set.id.to_string()),
        ];
        let selected = candidates
            .into_iter()
            .flatten()
            .find_map(|key| mappings.get(&key).cloned());
        mappings.clear();
        if let Some(mapping) = selected {
            mappings.insert(requested, mapping);
        }
    }
    mappings.retain(|_, mapping| {
        matches(input.client_frameworks.as_deref(), |filter| {
            mapping.label.eq_ignore_ascii_case(filter)
        }) && matches(input.client_languages.as_deref(), |filter| {
            language_matches(mapping, filter)
        })
    });
    match serde_json::to_value(mappings) {
        Ok(value) => CallToolResult::structured(value),
        Err(_) => fault("Cannot serialize Code Connect mappings"),
    }
}

/// An exclusive lock on the sibling `<sidecar>.lock` file, released when the handle drops.
/// `File::lock` is advisory: every writer of the sidecar takes it, which is the case here.
fn lock_sidecar(path: &Path) -> std::io::Result<std::fs::File> {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    let name = std::path::PathBuf::from(name);
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

fn validate(mapping: &Mapping) -> Result<(), &'static str> {
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
    crate::tools::persist_file(temp, path, permissions)
}

fn matches(filters: Option<&[String]>, predicate: impl Fn(&str) -> bool) -> bool {
    filters
        .is_none_or(|filters| filters.is_empty() || filters.iter().any(|filter| predicate(filter)))
}

fn language_matches(mapping: &Mapping, filter: &str) -> bool {
    if mapping.label.eq_ignore_ascii_case(filter) {
        return true;
    }
    let source = mapping.source.split(['?', '#']).next().unwrap_or("");
    let ext = Path::new(source)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match filter.to_ascii_lowercase().as_str() {
        "typescript" | "ts" => matches!(ext.as_str(), "ts" | "tsx"),
        "javascript" | "js" => matches!(ext.as_str(), "js" | "jsx" | "mjs"),
        "python" => ext == "py",
        "rust" => ext == "rs",
        language => ext == language,
    }
}
