//! Writing resolved instance content into the metadata body.
use super::node::{tag_name, write_node};
use figstudio_fig::{
    Document, Guid,
    resolve::{self, Definitions, Resolved, walk_instance},
};
use quick_xml::{
    Writer,
    events::{BytesStart, BytesText, Event},
};
use std::sync::Arc;

/// The XML body being written and its bounds.
pub(super) struct Out {
    pub(super) body: Writer<Vec<u8>>,
    pub(super) count: usize,
    pub(super) truncated: bool,
    pub(super) skipped: usize,
    /// Component property definitions, built on the first instance expanded.
    pub(super) definitions: Option<Arc<Definitions>>,
}

impl Out {
    /// The resolved content of `instance` down to `max_level`, at most `max_nodes` nodes.
    pub(super) fn walk(
        &mut self,
        doc: &Document,
        instance: Guid,
        max_level: Option<usize>,
        max_nodes: usize,
    ) -> Option<resolve::Walk> {
        let definitions = self
            .definitions
            .get_or_insert_with(|| Arc::new(resolve::definitions(doc)));
        let Ok(walk) = walk_instance(doc, definitions, instance, max_level, max_nodes) else {
            self.skipped += 1;
            return None;
        };
        self.truncated |= walk.truncated;
        self.skipped += walk.skipped_instances;
        Some(walk)
    }

    /// Writes resolved nodes in walk order (pre-order with levels), nesting them by level.
    pub(super) fn resolved(&mut self, doc: &Document, nodes: &[Resolved]) -> std::io::Result<()> {
        let mut open: Vec<(usize, String)> = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            while let Some((_, tag)) = open.pop_if(|(level, _)| *level >= node.level) {
                self.body
                    .write_event(Event::End(BytesStart::new(tag).to_end()))?;
            }
            let kind = doc.node(node.node).map_or("UNKNOWN", |n| n.kind.as_str());
            let children = nodes.get(index + 1).is_some_and(|n| n.level > node.level);
            let id = node.path.to_string();
            if write_node(&mut self.body, &id, kind, &node.data, children)? {
                open.push((node.level, tag_name(kind)));
            }
            self.count += 1;
        }
        while let Some((_, tag)) = open.pop() {
            self.body
                .write_event(Event::End(BytesStart::new(tag).to_end()))?;
        }
        Ok(())
    }

    pub(super) fn finish(self, writer: &mut Writer<Vec<u8>>) -> std::io::Result<()> {
        let mut wrapper = BytesStart::new("metadata");
        if self.truncated {
            wrapper.push_attribute(("truncated", "true"));
        }
        writer.write_event(Event::Start(wrapper))?;
        writer.get_mut().extend(self.body.into_inner());
        writer.write_event(Event::End(BytesStart::new("metadata").to_end()))?;
        if self.skipped > 0 {
            let note = format!(
                "approximations: {} instances not expanded (missing component, cycle or nesting cap)",
                self.skipped
            );
            writer.write_event(Event::Comment(BytesText::new(&note)))?;
        }
        Ok(())
    }
}
