//! Selection state shared by canvas gestures and keyboard commands.
use crate::tab::Tab;
use figstudio_fig::{
    Guid, Val,
    resolve::{self, InstancePath},
};
use figstudio_render::{Renderer, absolute_transform, kurbo::Affine};

impl Tab {
    /// Transient text data and its node-local to page transform, exactly as rendered.
    pub fn text_layout(&self) -> Option<(Guid, Affine, Val)> {
        if let Some(path) = &self.inner {
            let resolved = resolve::resolve_path(&self.doc, path).ok()?;
            if self.doc.node(resolved.node)?.kind != "TEXT" {
                return None;
            }
            let (tf, data) = Renderer::new().path_layout(&self.doc, path)?;
            return Some((resolved.node, tf, data));
        }
        let id = self.selection?;
        let node = self.doc.node(id)?;
        (node.kind == "TEXT").then(|| (id, absolute_transform(&self.doc, id), node.data.clone()))
    }

    pub fn set_inner(&mut self, path: Option<InstancePath>) {
        if self.inner != path {
            self.text_select = None;
        }
        self.inner = path;
    }

    /// Copy the active range, or the resolved layer's characters outside text mode.
    pub fn selected_text(&self) -> Option<String> {
        let (id, _, data) = self.text_layout()?;
        let text = crate::text_select::characters(&data);
        let text = self.text_select.filter(|s| s.node == id).map_or(text, |s| {
            crate::text_select::range_text(text, s.anchor, s.head)
        });
        (!text.is_empty()).then(|| text.to_owned())
    }

    pub fn select_all_text(&mut self) {
        if let Some((_, _, data)) = self.text_layout()
            && let Some(selection) = &mut self.text_select
        {
            selection.anchor = 0;
            selection.head = crate::text_select::characters(&data).len();
        }
    }

    pub fn select_relative(&mut self, down: bool) {
        let Some(selected) = self.selection else {
            return;
        };
        if self.inner.is_some()
            || self
                .doc
                .node(selected)
                .is_some_and(|n| n.kind == "INSTANCE")
        {
            if down {
                if let Some(path) =
                    crate::instance_pick::first_child(&self.doc, selected, self.inner.as_ref())
                {
                    self.set_inner(Some(path));
                }
            } else if let Some(path) = &self.inner {
                self.set_inner(crate::instance_pick::parent(&self.doc, path));
            } else if let Some(parent) = crate::pick::parent(&self.doc, self.page, selected) {
                self.select(Some(parent));
            }
            return;
        }
        let next = if down {
            crate::pick::child(&self.doc, selected)
        } else {
            crate::pick::parent(&self.doc, self.page, selected)
        };
        if let Some(next) = next {
            self.select(Some(next));
        }
    }

    /// Selects a node, switching page if needed and expanding its ancestors in the Layers tree.
    pub fn select(&mut self, id: Option<Guid>) {
        if self.selection != id {
            self.text_select = None;
            self.inner = None;
        }
        self.selection = id;
        let Some(id) = id else { return };
        if let Some(page) = self.doc.page_of(id)
            && page != self.page
        {
            self.page = page;
            self.fit_pending = true;
        }
        let mut cur = self.doc.node(id).and_then(|n| n.parent);
        while let Some(p) = cur {
            self.expanded.insert(p);
            cur = self.doc.node(p).and_then(|n| n.parent);
        }
        self.reveal = Some(id);
    }
}
