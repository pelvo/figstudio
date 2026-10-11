use super::{MapInput, Mapping, Mappings, context::component_id, fault};
use figstudio_fig::Document;
use rmcp::model::CallToolResult;
use std::{io::Write, path::Path};

pub(super) fn map(
    doc: &Document,
    path: &Path,
    input: MapInput,
    add: Option<Mapping>,
) -> CallToolResult {
    let id = match input
        .node_id
        .as_deref()
        .map(|id| component_id(doc, id))
        .transpose()
    {
        Ok(id) => id.map(|id| id.to_string()),
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
    let mut mappings: Mappings = match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return fault("Code Connect sidecar must be a regular file, not a symlink");
        }
        Ok(_) => match std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(mappings) => mappings,
            None => return fault("Cannot read Code Connect sidecar: invalid JSON or mapping"),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Mappings::new(),
        Err(_) => return fault("Cannot read Code Connect sidecar"),
    };
    if let Some(mapping) = add {
        let Some(id) = id.as_ref() else {
            return fault("nodeId is required");
        };
        if mapping.component_name.trim().is_empty()
            || mapping.source.trim().is_empty()
            || mapping.label.trim().is_empty()
        {
            return fault("componentName, source and label must not be empty");
        }
        mappings.insert(id.clone(), mapping);
        let write = (|| -> Result<(), Box<dyn std::error::Error>> {
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            serde_json::to_writer_pretty(&mut temp, &mappings)?;
            temp.flush()?;
            temp.as_file().sync_all()?;
            temp.persist(&path)?;
            Ok(())
        })();
        if write.is_err() {
            return fault("Cannot atomically write Code Connect sidecar");
        }
    }
    mappings.retain(|key, mapping| {
        id.as_ref().is_none_or(|id| id == key)
            && matches(input.client_frameworks.as_deref(), |filter| {
                mapping.label.eq_ignore_ascii_case(filter)
            })
            && matches(input.client_languages.as_deref(), |filter| {
                language_matches(mapping, filter)
            })
    });
    match serde_json::to_value(mappings) {
        Ok(value) => CallToolResult::structured(value),
        Err(_) => fault("Cannot serialize Code Connect mappings"),
    }
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
