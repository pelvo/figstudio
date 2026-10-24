//! Region fingerprints collected while pixels are alive; no images retained or re-rendered.
use crate::{DiffReport, Region, SharedChange, SharedComponent, SharedOccurrence};
use crate::{SharedSet, SharedVariant};
use figstudio_fig::{Document, Guid};
use figstudio_render::Frame;
use std::collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher};
use std::hash::Hasher;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Signature(u32, u32, u64);

pub(crate) fn signatures(
    regions: &[Region],
    mask: &[bool],
    width: u32,
    before: &Option<Frame>,
    after: &Option<Frame>,
) -> Vec<Signature> {
    regions
        .iter()
        .map(|region| {
            let mut hash = DefaultHasher::new();
            // Batch hash writes by row; keep only the mask and four packed RGBA nibbles per side.
            let mut row = Vec::with_capacity(region.width as usize * 5);
            for y in region.y..region.y + region.height {
                row.clear();
                for x in region.x..region.x + region.width {
                    let index = y as usize * width as usize + x as usize;
                    let changed = mask.get(index).copied().unwrap_or(false);
                    row.push(u8::from(changed));
                    if changed {
                        for frame in [before, after] {
                            let rgba = frame.as_ref().map_or([0; 4], |f| {
                                crate::render_pixels::pixel4(f.pixmap.data_as_u8_slice(), index * 4)
                            });
                            row.push((rgba[0] & 0xf0) | (rgba[1] >> 4));
                            row.push((rgba[2] & 0xf0) | (rgba[3] >> 4));
                        }
                    }
                }
                hash.write(&row);
            }
            Signature(region.width, region.height, hash.finish())
        })
        .collect()
}

/// How one region joins a group: by the changed component it belongs to, else by pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegionKey {
    pub signature: Signature,
    pub component: Option<ComponentKey>,
}

/// `group` is the component set (when the master is a variant) or the master itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ComponentKey {
    pub group: Guid,
    pub master: Guid,
}

type Group = (Option<Guid>, Vec<(SharedOccurrence, Option<Guid>)>);

#[derive(Debug, Default)]
pub(crate) struct SharedGroups(Vec<(SharedOccurrence, RegionKey)>);

fn screens<'a>(occurrences: impl Iterator<Item = &'a SharedOccurrence>) -> usize {
    occurrences
        .map(|o| (o.page_index, o.screen_index))
        .collect::<BTreeSet<_>>()
        .len()
}

impl SharedGroups {
    pub(crate) fn record(&mut self, page_index: usize, screens: Vec<Vec<RegionKey>>) {
        for (screen_index, regions) in screens.into_iter().enumerate() {
            for (region_index, key) in regions.into_iter().enumerate() {
                let at = SharedOccurrence {
                    page_index,
                    screen_index,
                    region_index,
                };
                self.0.push((at, key));
            }
        }
    }

    /// Component groups first (two or more screens); every other region falls back to its
    /// pixel signature. Each occurrence keeps its master (the variant, within a set).
    fn groups(self) -> Vec<Group> {
        let mut by_component: BTreeMap<Guid, Vec<(SharedOccurrence, Option<Guid>)>> =
            BTreeMap::new();
        for (at, key) in &self.0 {
            if let Some(c) = key.component {
                by_component
                    .entry(c.group)
                    .or_default()
                    .push((*at, Some(c.master)));
            }
        }
        by_component.retain(|_, o| screens(o.iter().map(|(at, _)| at)) >= 2);
        let mut by_signature: BTreeMap<Signature, Vec<(SharedOccurrence, Option<Guid>)>> =
            BTreeMap::new();
        for (at, key) in self.0 {
            if key
                .component
                .is_none_or(|c| !by_component.contains_key(&c.group))
            {
                by_signature
                    .entry(key.signature)
                    .or_default()
                    .push((at, None));
            }
        }
        let mut out: Vec<Group> = by_component
            .into_iter()
            .map(|(id, o)| (Some(id), o))
            .chain(
                by_signature
                    .into_iter()
                    .filter(|(_, o)| screens(o.iter().map(|(at, _)| at)) >= 2)
                    .map(|(_, o)| (None, o)),
            )
            .collect();
        // Most distinct screens first, then most occurrences, then first occurrence.
        out.sort_by_key(|(_, o)| {
            let distinct = screens(o.iter().map(|(at, _)| at));
            (
                std::cmp::Reverse(distinct),
                std::cmp::Reverse(o.len()),
                o[0].0,
            )
        });
        out
    }

    pub(crate) fn finish(self, report: &mut DiffReport, docs: (&Document, &Document)) {
        let name_of = |id: Guid| {
            [docs.1, docs.0]
                .iter()
                .find_map(|d| d.node(id))
                .map_or_else(String::new, |n| n.name.clone())
        };
        let mut shared = Vec::new();
        for (id, (group, mut marked)) in self.groups().into_iter().enumerate() {
            let count = screens(marked.iter().map(|(at, _)| at));
            let area = |o: &SharedOccurrence| {
                let r = &report.pages[o.page_index].screens[o.screen_index].regions[o.region_index];
                u64::from(r.width) * u64::from(r.height)
            };
            // The largest region (first on ties) gives the most context and leads the group.
            let lead = (0..marked.len())
                .max_by_key(|i| (area(&marked[*i].0), std::cmp::Reverse(*i)))
                .unwrap_or(0);
            let first = marked.remove(lead);
            marked.insert(0, first);
            let occurrences: Vec<_> = marked.iter().map(|(at, _)| *at).collect();
            for o in &occurrences {
                report.pages[o.page_index].screens[o.screen_index].regions[o.region_index].shared =
                    Some(id);
            }
            let region = &report.pages[first.0.page_index].screens[first.0.screen_index].regions
                [first.0.region_index];
            let component = group.map(|group| component(group, &marked, &name_of));
            shared.push(SharedChange {
                id,
                width: region.width,
                height: region.height,
                count,
                old_png: region.old_png.clone(),
                new_png: region.new_png.clone(),
                occurrences,
                component,
                component_view: false,
                image_width: region.image_width,
                image_height: region.image_height,
            });
        }
        report.shared_changes = shared;
    }
}

/// The variant seen on most screens is shown (ties: first in document order); the others
/// follow in document order. A group that is not a set has no variants.
fn component(
    group: Guid,
    marked: &[(SharedOccurrence, Option<Guid>)],
    name_of: &dyn Fn(Guid) -> String,
) -> SharedComponent {
    let mut masters: Vec<Guid> = Vec::new();
    let mut seen: BTreeMap<Guid, BTreeSet<(usize, usize)>> = BTreeMap::new();
    let mut ordered: Vec<_> = marked.iter().collect();
    ordered.sort_by_key(|(at, _)| *at);
    for (at, master) in ordered {
        let Some(master) = master else { continue };
        if !masters.contains(master) {
            masters.push(*master);
        }
        seen.entry(*master)
            .or_default()
            .insert((at.page_index, at.screen_index));
    }
    let shown = masters
        .iter()
        .copied()
        .max_by_key(|m| {
            (
                seen[m].len(),
                std::cmp::Reverse(masters.iter().position(|x| x == m)),
            )
        })
        .unwrap_or(group);
    let is_set = shown != group;
    let variants = if is_set {
        std::iter::once(shown)
            .chain(masters.iter().copied().filter(|m| *m != shown))
            .map(|id| SharedVariant {
                id,
                name: name_of(id),
                count: seen[&id].len(),
                old_png: None,
                new_png: None,
                image_width: None,
                image_height: None,
            })
            .collect()
    } else {
        Vec::new()
    };
    SharedComponent {
        id: shown,
        name: name_of(shown),
        set: is_set.then(|| SharedSet {
            id: group,
            name: name_of(group),
        }),
        variants,
    }
}
