//! Document-scoped instance picking and tiny per-frame result caches.
use std::{
    cell::{RefCell, RefMut},
    sync::Arc,
};

use figstudio_fig::{
    Document, Guid,
    resolve::{Definitions, InstancePath},
};
use figstudio_render::{Renderer, geom, kurbo::Point};

#[derive(Debug, PartialEq, Eq)]
struct PageHitKey {
    page: Guid,
    x: u64,
    y: u64,
    zoom: u32,
}

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
    page_hit: Option<(PageHitKey, Vec<Guid>)>,
    page_walks: usize,
    hover: Option<(HoverKey, Option<InstancePath>)>,
    // The canvas draws at most two inner outlines: selected and hovered.
    outlines: [Option<(InstancePath, Option<[Point; 4]>)>; 2],
}

impl State {
    fn new(doc: &Document) -> Self {
        let mut renderer = Renderer::new();
        let definitions = renderer.definitions(doc);
        Self {
            document: doc.instance,
            renderer,
            definitions,
            page_hit: None,
            page_walks: 0,
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

    #[cfg(test)]
    pub fn page_walks(&self, doc: &Document) -> usize {
        self.state(doc).page_walks
    }

    /// Cached outer-page hit chain so stationary hover frames reuse the prior walk without
    /// re-scanning connectors, vectors, or styles. Invalidated automatically on `doc.instance`.
    pub fn page_chain(&self, tab: &crate::tab::Tab, point: Point) -> Vec<Guid> {
        let key = PageHitKey {
            page: tab.page,
            x: point.x.to_bits(),
            y: point.y.to_bits(),
            zoom: tab.view.zoom.to_bits(),
        };
        let mut state = self.state(&tab.doc);
        if let Some((old, result)) = &state.page_hit
            && *old == key
        {
            return result.clone();
        }
        state.page_walks += 1;
        let result = figstudio_render::hit_chain_with_options(
            &tab.doc,
            tab.page,
            point,
            crate::canvas::hit_options(tab.view.zoom),
        );
        state.page_hit = Some((key, result.clone()));
        result
    }

    pub fn corners(&self, doc: &Document, path: &InstancePath) -> Option<[Point; 4]> {
        let mut state = self.state(doc);
        if let Some((_, corners)) = state
            .outlines
            .iter()
            .flatten()
            .find(|(cached, _)| cached == path)
        {
            return *corners;
        }
        let corners = state.renderer.path_layout(doc, path).map(|(tf, data)| {
            let (w, h) = geom::size(&data);
            [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|p| tf * Point::from(p))
        });
        state.outlines[0] = state.outlines[1].take();
        state.outlines[1] = Some((path.clone(), corners));
        corners
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
        let hits = crate::instance_pick::hit_chain_with_options(
            &tab.doc,
            renderer,
            instance,
            point,
            crate::canvas::hit_options(tab.view.zoom),
        );
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
