//! The page list (`get_metadata` with no nodeId).
use super::{ELEMENT_MAX_BYTES, MAX_ELEMENTS, node::clean_xml};
use figstudio_fig::Document;
use quick_xml::{
    Writer,
    events::{BytesStart, BytesText, Event},
};

pub(super) fn write(
    doc: &Document,
    writer: &mut Writer<Vec<u8>>,
    max_bytes: usize,
) -> std::io::Result<()> {
    let visible: Vec<_> = doc
        .pages
        .iter()
        .filter_map(|id| doc.node(*id))
        .filter(|node| {
            node.data.flag("visible") != Some(false) && node.data.flag("internalOnly") != Some(true)
        })
        .take(MAX_ELEMENTS + 1)
        .collect();
    // Page names are not capped by the writer, so the budget is checked per element.
    let mut pages = BytesStart::new("pages");
    let mut body = Writer::new(Vec::new());
    let mut written = 0;
    for node in visible.iter().take(MAX_ELEMENTS) {
        if body.get_ref().len() + ELEMENT_MAX_BYTES + 32 > max_bytes {
            break;
        }
        let mut tag = BytesStart::new("canvas");
        let id = node.id.to_string();
        tag.push_attribute(("id", id.as_str()));
        let name = clean_xml(&node.name);
        tag.push_attribute(("name", name.as_str()));
        body.write_event(Event::Empty(tag))?;
        written += 1;
    }
    let cut = visible.len() > written;
    if cut {
        pages.push_attribute(("truncated", "true"));
    }
    writer.write_event(Event::Start(pages))?;
    writer.get_mut().extend(body.into_inner());
    writer.write_event(Event::End(BytesStart::new("pages").to_end()))?;
    if cut {
        let note = "truncated: call get_metadata with a page nodeId to list its children";
        writer.write_event(Event::Comment(BytesText::new(note)))?;
    }
    Ok(())
}
