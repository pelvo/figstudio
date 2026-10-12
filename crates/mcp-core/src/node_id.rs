use crate::{CoreError, link::parse_link};
use figstudio_fig::{Guid, resolve::InstancePath};

/// A document node or a node inside an instance (`I<instance>;<key>...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeId {
    /// One node GUID.
    Node(Guid),
    /// A node inside an instance; resolve it with `figstudio_fig::resolve`.
    InstancePath(InstancePath),
}

/// Parse colon/dash IDs, instance paths, and the node-id of figma.com and `figstudio://` links.
///
/// A link that names no node (a region link, or a figma.com link without `node-id`) is invalid.
pub fn parse_node_id(input: &str) -> Result<NodeId, CoreError> {
    if ["https://", "http://", "figstudio://"]
        .iter()
        .any(|scheme| input.starts_with(scheme))
    {
        return parse_link(input)
            .ok()
            .and_then(|link| link.node)
            .ok_or_else(CoreError::invalid_node_id);
    }
    parse_bare(input)
}

/// A bare ID: `1:2`, `1-2` or `I1:2;3:4`.
pub(crate) fn parse_bare(input: &str) -> Result<NodeId, CoreError> {
    if let Some(path) = input.strip_prefix('I') {
        let nodes = path
            .split(';')
            .map(parse_guid)
            .collect::<Result<Vec<_>, _>>()?;
        InstancePath::new(nodes)
            .map(NodeId::InstancePath)
            .ok_or_else(CoreError::invalid_node_id)
    } else {
        parse_guid(input).map(NodeId::Node)
    }
}

/// `session:local` or `session-local`, both numbers unsigned.
pub(crate) fn parse_guid(input: &str) -> Result<Guid, CoreError> {
    let (session, local) = input
        .split_once([':', '-'])
        .ok_or_else(CoreError::invalid_node_id)?;
    if [session, local]
        .iter()
        .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(CoreError::invalid_node_id());
    }
    Ok(Guid(
        session.parse().map_err(|_| CoreError::invalid_node_id())?,
        local.parse().map_err(|_| CoreError::invalid_node_id())?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nodes_urls_and_paths() {
        for input in [
            "1:2",
            "1-2",
            "https://www.figma.com/design/example?node-id=1-2",
            "https://figma.com/design/example?node-id=1%3A2",
        ] {
            assert_eq!(parse_node_id(input).unwrap(), NodeId::Node(Guid(1, 2)));
        }
        assert_eq!(
            parse_node_id("I1:2;3:4").unwrap(),
            NodeId::InstancePath(InstancePath::new(vec![Guid(1, 2), Guid(3, 4)]).unwrap())
        );
    }

    #[test]
    fn rejects_invalid_ids() {
        for input in [
            "",
            "1",
            "1:",
            "-1:2",
            "+1:2",
            "1:2:3",
            "4294967296:2",
            "I1:2",
            "I1:2;",
            "1:2;3:4",
            "https://example.com/?node-id=1-2",
            "https://figma.com/",
            "https://figma.com/?node-id=1-2&node-id=3-4",
            "https://figma.com/?node-id=1%ZZ2",
        ] {
            assert!(parse_node_id(input).is_err(), "accepted {input}");
        }
    }
}
