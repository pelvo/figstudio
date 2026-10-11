//! Writes the collected layers as nested JSX elements.
use super::walk::{Item, Layers, Target};
use super::{
    Assets, Classes, MAX_ASSETS, MAX_CODE_BYTES, Notes, Options, assets, layout, paint, style, text,
};
use crate::tools::screenshot::{Root, layer_png};
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
/// Asset render scale: sharp on high-density screens.
const ASSET_SCALE: f64 = 2.0;

/// One rendered asset.
#[derive(Debug)]
pub(super) struct Asset {
    pub(super) name: String,
    pub(super) node: String,
    /// What the code's `src` holds: the written path, or the name for inline images.
    pub(super) reference: String,
    pub(super) inline: Option<Vec<u8>>,
}

/// The written code and what was left out.
#[derive(Debug, Default)]
pub(super) struct Output {
    pub(super) code: String,
    pub(super) assets: Vec<Asset>,
    pub(super) notes: Notes,
    /// The size cap cut the code.
    pub(super) truncated: bool,
}

struct Writer<'a> {
    doc: &'a Document,
    options: &'a Options,
    tokens: Tokens,
    out: Output,
}

pub(super) fn write(doc: &Document, layers: &Layers<'_>, options: &Options) -> Output {
    let mut w = Writer {
        doc,
        options,
        tokens: Tokens::new(doc),
        out: Output::default(),
    };
    // Open elements: (level, index into items, closing tag).
    let mut open: Vec<(usize, usize, &str)> = Vec::new();
    let mut skip_below: Option<usize> = None;
    let items = &layers.items;
    for (index, item) in items.iter().enumerate() {
        if skip_below.is_some_and(|level| item.level > level) {
            continue;
        }
        skip_below = None;
        while let Some((_, _, tag)) = open.pop_if(|(level, _, _)| *level >= item.level) {
            w.close(open.len(), tag);
        }
        if index > 0 && item.data.flag("visible") == Some(false) {
            skip_below = Some(item.level);
            continue;
        }
        if w.out.code.len() > MAX_CODE_BYTES {
            w.out.truncated = true;
            break;
        }
        let parent = open.last().map(|(_, i, _)| &*items[*i].data);
        let children = items.get(index + 1).is_some_and(|n| n.level > item.level);
        match w.element(item, parent, children, open.len()) {
            Some(tag) => open.push((item.level, index, tag)),
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
        let src = self.asset(&item.target);
        match src {
            Some(src) => {
                let alt = attr(item.data.text("name").unwrap_or("")).into_owned();
                self.line(
                    depth,
                    &format!(
                        "<img {head} alt=\"{alt}\" src=\"{}\" className=\"{classes}\" />",
                        attr(&src)
                    ),
                );
            }
            None => self.line(depth, &format!("<div {head} className=\"{classes}\" />")),
        }
    }

    /// Renders `target` to a PNG asset and returns its `src`; `None` (counted) when it could
    /// not be rendered or stored.
    fn asset(&mut self, target: &Target) -> Option<String> {
        if self.out.assets.len() >= MAX_ASSETS {
            self.out.notes.add("assets not rendered (asset cap)");
            return None;
        }
        let root = match target {
            Target::Node(id) => Root::Node(*id),
            Target::Path(path) => Root::Path(path),
        };
        let Ok(png) = layer_png(self.doc, root, ASSET_SCALE, self.options.system_fonts) else {
            self.out.notes.add("assets not rendered (empty or hidden)");
            return None;
        };
        if !crate::tools::screenshot::approximations(&png.stats).is_empty() {
            self.out
                .notes
                .add("assets with render approximations (see get_screenshot)");
        }
        let name = assets::name(&self.options.file_key, &target.id());
        let (reference, inline) = match &self.options.assets {
            Assets::Inline => (name.clone(), Some(png.bytes)),
            Assets::Dir(dir) => match assets::write(dir, &name, &png.bytes) {
                Ok(path) => (path.display().to_string(), None),
                Err(_) => {
                    self.out
                        .notes
                        .add("assets not written (assets directory error)");
                    return None;
                }
            },
        };
        self.out.assets.push(Asset {
            name,
            node: target.id(),
            reference: reference.clone(),
            inline,
        });
        Some(reference)
    }
}

fn layout_size(data: &Val) -> (f64, f64) {
    let axis = |a| {
        data.path(&["size", a])
            .and_then(Val::as_f32)
            .map_or(0.0, f64::from)
    };
    (axis("x"), axis("y"))
}
