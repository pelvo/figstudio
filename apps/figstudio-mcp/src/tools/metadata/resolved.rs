//! Writing resolved instance content into the metadata body.
use super::{
    ELEMENT_MAX_BYTES, MAX_XML_BYTES,
    figjam::Extras,
    node::{tag_name, write_node},
};
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
    /// Bytes of the closing tags of open elements, reserved so the byte budget holds.
    pub(super) close_reserve: usize,
    pub(super) skipped: usize,
    /// Component property definitions, built on the first instance expanded.
    pub(super) definitions: Option<Arc<Definitions>>,
    /// FigJam attributes, text and counters; `None` for plain `get_metadata`.
    pub(super) figjam: Option<Extras>,
}

impl Out {
    /// Whether another element (and the closing tags still owed) fits in the byte budget.
    /// An element can be up to [`ELEMENT_MAX_BYTES`], so the check leaves that much room.
    pub(super) fn has_room(&self) -> bool {
        self.body.get_ref().len() + self.close_reserve + ELEMENT_MAX_BYTES <= MAX_XML_BYTES
    }

    /// The resolved content of `instance` down to `max_level`, at most `max_nodes` nodes.
    /// Component property definitions, built on the first instance expanded.
    pub(super) fn definitions(&mut self, doc: &Document) -> Arc<Definitions> {
        self.definitions
            .get_or_insert_with(|| Arc::new(resolve::definitions(doc)))
            .clone()
    }

    pub(super) fn walk(
        &mut self,
        doc: &Document,
        instance: Guid,
        max_level: Option<usize>,
        max_nodes: usize,
    ) -> Option<resolve::Walk> {
        let definitions = self.definitions(doc);
        let Ok(walk) = walk_instance(doc, &definitions, instance, max_level, max_nodes) else {
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
                self.close_reserve -= tag.len() + 3;
                self.body
                    .write_event(Event::End(BytesStart::new(tag).to_end()))?;
            }
            let kind = doc.node(node.node).map_or("UNKNOWN", |n| n.kind.as_str());
            let children = nodes.get(index + 1).is_some_and(|n| n.level > node.level);
            let id = node.path.to_string();
            if let Some(extras) = &mut self.figjam {
                extras.note(&id, kind);
            }
            if !self.has_room() {
                self.truncated = true;
                break;
            }
            let figjam = self.figjam.is_some();
            if write_node(&mut self.body, &id, kind, &node.data, children, figjam)? {
                open.push((node.level, tag_name(kind)));
                self.close_reserve += tag_name(kind).len() + 3;
            }
            self.count += 1;
        }
        while let Some((_, tag)) = open.pop() {
            self.close_reserve -= tag.len() + 3;
            self.body
                .write_event(Event::End(BytesStart::new(tag).to_end()))?;
        }
        Ok(())
    }

    pub(super) fn finish(self, writer: &mut Writer<Vec<u8>>) -> std::io::Result<Option<Extras>> {
        let mut wrapper = BytesStart::new("metadata");
        if self.truncated {
            wrapper.push_attribute(("truncated", "true"));
        }
        writer.write_event(Event::Start(wrapper))?;
        writer.get_mut().extend(self.body.into_inner());
        writer.write_event(Event::End(BytesStart::new("metadata").to_end()))?;
        if self.truncated {
            let note = "truncated: call get_metadata again with a child nodeId or a smaller depth";
            writer.write_event(Event::Comment(BytesText::new(note)))?;
        }
        if self.skipped > 0 {
            let note = format!(
                "approximations: {} instances not expanded (missing component, cycle or nesting cap)",
                self.skipped
            );
            writer.write_event(Event::Comment(BytesText::new(&note)))?;
        }
        if let Some(note) = self.figjam.as_ref().and_then(Extras::approximations) {
            writer.write_event(Event::Comment(BytesText::new(&note)))?;
        }
        Ok(self.figjam)
    }
}
