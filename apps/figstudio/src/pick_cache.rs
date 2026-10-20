//! Document-scoped instance picking and tiny per-frame result caches.
use std::{
    cell::{RefCell, RefMut},
    sync::Arc,
};

use figstudio_fig::{
    Document, Guid,
    resolve::{Definitions, InstancePath},
};
use figstudio_render::{
    Renderer,
    kurbo::{Point, Rect},
};

#[derive(Debug, PartialEq)]
struct HoverKey {
    instance: Guid,
    selection: Option<Guid>,
    inner: Option<InstancePath>,
    pixel: Point,
    zoom: f64,
    deep: bool,
}

#[derive(Debug)]
pub struct PickCache(RefCell<State>);

#[derive(Debug)]
struct State {
    document: u64,
    renderer: Renderer,
    definitions: Arc<Definitions>,
    hover: Option<(HoverKey, Option<InstancePath>)>,
    // The canvas draws at most two inner outlines: selected and hovered.
    outlines: [Option<(InstancePath, Option<Rect>)>; 2],
}

impl State {
    fn new(doc: &Document) -> Self {
        let mut renderer = Renderer::new();
        let definitions = renderer.definitions(doc);
        Self {
            document: doc.instance,
            renderer,
            definitions,
            hover: None,
            outlines: [None, None],
        }
    }
}

impl PickCache {
    pub fn new(doc: &Document) -> Self {
        Self(RefCell::new(State::new(doc)))
    }

    pub fn with<T>(
        &self,
        doc: &Document,
        f: impl FnOnce(&mut Renderer, &Arc<Definitions>) -> T,
    ) -> T {
        let mut state = self.state(doc);
        let State {
            renderer,
            definitions,
            ..
        } = &mut *state;
        f(renderer, definitions)
    }

    fn state(&self, doc: &Document) -> RefMut<'_, State> {
        let mut state = self.0.borrow_mut();
        if state.document != doc.instance {
            *state = State::new(doc);
        }
        state
    }

    #[cfg(test)]
    pub fn work(&self, doc: &Document) -> figstudio_render::InspectionWork {
        self.with(doc, |renderer, _| renderer.inspection_work())
    }

    pub fn bounds(&self, doc: &Document, path: &InstancePath) -> Option<Rect> {
        let mut state = self.state(doc);
        if let Some((_, bounds)) = state
            .outlines
            .iter()
            .flatten()
            .find(|(cached, _)| cached == path)
        {
            return *bounds;
        }
        let bounds = state.renderer.path_bounds(doc, path);
        state.outlines[0] = state.outlines[1].take();
        state.outlines[1] = Some((path.clone(), bounds));
        bounds
    }

    pub fn hover(&self, tab: &crate::tab::Tab, point: Point, deep: bool) -> Option<InstancePath> {
        let instance = tab.hover?;
        let zoom = f64::from(tab.view.zoom);
        let key = HoverKey {
            instance,
            selection: tab.selection,
            inner: tab.inner.clone(),
            pixel: Point::new((point.x * zoom).round(), (point.y * zoom).round()),
            zoom,
            deep,
        };
        let mut state = self.state(&tab.doc);
        if let Some((old, result)) = &state.hover
            && *old == key
        {
            return result.clone();
        }
        let State {
            renderer,
            definitions,
            ..
        } = &mut *state;
        let hits = crate::instance_pick::hit_chain_with(&tab.doc, renderer, instance, point);
        let result = crate::instance_pick::click(
            &tab.doc,
            definitions,
            &hits,
            tab.inner.as_ref(),
            deep,
            false,
        );
        state.hover = Some((key, result.clone()));
        result
    }
}
