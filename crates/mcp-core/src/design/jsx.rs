//! Writes the collected layers as nested JSX elements.
use super::walk::{Item, Layers};
use super::{Assets, Classes, Notes, Options, assets, layout, paint, style, text};
use figstudio_fig::{Document, Val, tokens::Tokens};
use html_escape::encode_double_quoted_attribute as attr;

/// Kinds drawn as a rendered picture: their outlines have no CSS equivalent.
const PICTURE_KINDS: [&str; 5] = [
    "VECTOR",
    "BOOLEAN_OPERATION",
    "STAR",
    "REGULAR_POLYGON",
    "LINE",
];

/// The written code and what was left out.
#[derive(Debug, Default)]
pub(super) struct Output {
    pub(super) code: String,
    pub(super) assets: Vec<assets::Asset>,
    pub(super) notes: Notes,
    /// The size cap cut the code.
    pub(super) truncated: bool,
}

pub(super) struct Writer<'a> {
    pub(super) doc: &'a Document,
    pub(super) options: &'a Options,
    tokens: Tokens,
    pub(super) out: Output,
    /// Code length the element being written must stay within (cap minus pending closings).
    limit: usize,
    /// Base64 bytes of the inline images so far.
    pub(super) inline_bytes: usize,
}

pub(super) fn write(
    doc: &Document,
    layers: &Layers<'_>,
    options: &Options,
    max_code: usize,
) -> Output {
    let mut w = Writer {
        doc,
        options,
        tokens: Tokens::new(doc),
        out: Output::default(),
        limit: max_code,
        inline_bytes: 0,
    };
    // Open elements: (level, index into items, closing tag).
    let mut open: Vec<(usize, usize, &str)> = Vec::new();
    // Bytes of the closing tags of `open`, reserved so the cap holds once they are written.
    let mut closing = 0;
    let mut skip_below: Option<usize> = None;
    let items = &layers.items;
    for (index, item) in items.iter().enumerate() {
        if skip_below.is_some_and(|level| item.level > level) {
            continue;
        }
        skip_below = None;
        while let Some((_, _, tag)) = open.pop_if(|(level, _, _)| *level >= item.level) {
            closing -= close_len(open.len(), tag);
            w.close(open.len(), tag);
        }
        if index > 0 && item.data.flag("visible") == Some(false) {
            skip_below = Some(item.level);
            continue;
        }
        let parent = open.last().map(|(_, i, _)| &*items[*i].data);
        let children = items.get(index + 1).is_some_and(|n| n.level > item.level);
        let before = w.out.code.len();
        w.limit = max_code.saturating_sub(closing);
        let tag = w.element(item, parent, children, open.len());
        let close = tag.map_or(0, |tag| close_len(open.len(), tag));
        // The element (escaping included) and every closing tag it leaves pending must fit.
        if w.out.code.len() + close > w.limit {
            w.out.code.truncate(before);
            w.out.truncated = true;
            break;
        }
        match tag {
            Some(tag) => {
                closing += close;
                open.push((item.level, index, tag));
            }
            None => skip_below = Some(item.level),
        }
    }
    while let Some((_, _, tag)) = open.pop() {
        w.close(open.len(), tag);
    }
    w.out
}

impl Writer<'_> {
    fn line(&mut self, depth: usize, text: &str) {
        self.out.code.push_str(&"  ".repeat(depth));
        self.out.code.push_str(text);
        self.out.code.push('\n');
    }

    fn close(&mut self, depth: usize, tag: &str) {
        debug_assert_eq!(close_len(depth, tag), 2 * depth + tag.len() + 4);
        self.line(depth, &format!("</{tag}>"));
    }

    /// Writes `item`; returns the tag left open for its children, or `None` when the element
    /// is complete and its subtree must be skipped.
    fn element(
        &mut self,
        item: &Item<'_>,
        parent: Option<&Val>,
        children: bool,
        depth: usize,
    ) -> Option<&'static str> {
        let data = &*item.data;
        if item.scale != 1.0 && layout::keeps_stored_lengths(data) {
            self.out
                .notes
                .add("lengths kept unscaled in scaled instances");
        }
        let fills = paint::paints(data, "fillPaints", &self.tokens);
        let image = fills.iter().any(|p| p.text("type") == Some("IMAGE"));
        let head = format!(
            "data-node-id=\"{}\" data-name=\"{}\"",
            item.target.id(),
            attr(data.text("name").unwrap_or(""))
        );
        if PICTURE_KINDS.contains(&item.kind) || (image && !children) {
            self.picture(item, parent, &head, depth);
            return None;
        }
        let mut classes = Classes::default();
        layout::place(
            &mut classes,
            item.kind,
            data,
            parent,
            false,
            &mut self.out.notes,
        );
        if item.kind == "TEXT" {
            return self.text(item, classes, &head, depth);
        }
        layout::container(&mut classes, item.kind, data, children, &mut self.out.notes);
        if item.kind == "ELLIPSE" {
            classes.push("rounded-[50%]");
            if data.get("arcData").is_some() {
                self.out.notes.add("ellipse arcs drawn whole");
            }
        }
        let size = layout_size(data);
        let mut layers = Vec::new();
        for paint in &fills {
            match paint::layer(paint, size, &self.tokens, &mut self.out.notes) {
                Some(layer) => layers.push(layer),
                None if paint.text("type") == Some("IMAGE") => {
                    self.out.notes.add("image fills behind children dropped")
                }
                None => self.out.notes.add("paints skipped"),
            }
        }
        if let Some(bg) = paint::background(&layers) {
            classes.push(bg);
        }
        style::decorate(
            &mut classes,
            item.kind,
            data,
            &self.tokens,
            &mut self.out.notes,
        );
        let open = format!("<div {head} className=\"{}\"", attr(&classes.join()));
        if children {
            self.line(depth, &format!("{open}>"));
            Some("div")
        } else {
            self.line(depth, &format!("{open} />"));
            None
        }
    }

    fn text(
        &mut self,
        item: &Item<'_>,
        mut classes: Classes,
        head: &str,
        depth: usize,
    ) -> Option<&'static str> {
        let data = &*item.data;
        text::classes(&mut classes, data, &self.tokens, &mut self.out.notes);
        style::decorate(
            &mut classes,
            "TEXT",
            data,
            &self.tokens,
            &mut self.out.notes,
        );
        let mut body = String::new();
        for (id, run) in text::runs(data) {
            let mut span = Classes::default();
            if let Some(fragment) = (id != 0).then(|| text::fragment(data, id)).flatten() {
                text::classes(&mut span, fragment, &self.tokens, &mut self.out.notes);
            }
            if span.0.is_empty() {
                body.push_str(&text::jsx(&run));
            } else {
                body.push_str(&format!(
                    "<span className=\"{}\">{}</span>",
                    attr(&span.join()),
                    text::jsx(&run)
                ));
            }
        }
        self.line(
            depth,
            &format!(
                "<p {head} className=\"{}\">{body}</p>",
                attr(&classes.join())
            ),
        );
        None
    }

    fn picture(&mut self, item: &Item<'_>, parent: Option<&Val>, head: &str, depth: usize) {
        let mut classes = Classes::default();
        layout::place(
            &mut classes,
            item.kind,
            &item.data,
            parent,
            true,
            &mut self.out.notes,
        );
        style::common(&mut classes, &item.data, &mut self.out.notes);
        let classes = attr(&classes.join()).into_owned();
        let name = assets::name(&self.options.file_key, &item.target.id());
        let src = match &self.options.assets {
            Assets::Inline => name.clone(),
            Assets::Dir(dir) => dir.join(&name).display().to_string(),
        };
        let alt = attr(item.data.text("name").unwrap_or("")).into_owned();
        let img = format!(
            "<img {head} alt=\"{alt}\" src=\"{}\" className=\"{classes}\" />",
            attr(&src)
        );
        // Render only what the size cap lets into the code; the caller cuts an oversized line.
        let fits = self.out.code.len() + 2 * depth + img.len() < self.limit;
        if !fits || assets::render(self, &item.target, name) {
            self.line(depth, &img);
        } else {
            self.line(depth, &format!("<div {head} className=\"{classes}\" />"));
        }
    }
}

/// Bytes [`Writer::close`] writes: indent, `</tag>` and the newline.
fn close_len(depth: usize, tag: &str) -> usize {
    2 * depth + tag.len() + 4
}

fn layout_size(data: &Val) -> (f64, f64) {
    let axis = |a| {
        data.path(&["size", a])
            .and_then(Val::as_f32)
            .map_or(0.0, f64::from)
    };
    (axis("x"), axis("y"))
}
