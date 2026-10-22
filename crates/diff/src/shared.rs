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
        for (group, mut marked) in self.groups() {
            let id = shared.len();
            let count = screens(marked.iter().map(|(at, _)| at));
            let region = |o: &SharedOccurrence| {
                &report.pages[o.page_index].screens[o.screen_index].regions[o.region_index]
            };
            // An area kept without its crop (the page byte cap) has no picture to lead with.
            let cropped = |o: &SharedOccurrence| region(o).image_width.is_some();
            let area = |o: &SharedOccurrence| {
                let r = region(o);
                u64::from(r.width) * u64::from(r.height)
            };
            // The largest cropped region (first on ties) gives the most context and leads.
            let lead = (0..marked.len())
                .max_by_key(|i| {
                    let at = &marked[*i].0;
                    (cropped(at), area(at), std::cmp::Reverse(*i))
                })
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

/// Drops the shared cards left with no picture: no occurrence kept its crop (the page byte
/// cap) and no component view was drawn. Their screens then stay unfolded and show their own
/// areas. Card ids stay contiguous, in `report` and in the component pictures of `images`.
pub(crate) fn drop_unpictured(report: &mut DiffReport, images: Option<&mut crate::RenderImages>) {
    let mut ids = Vec::with_capacity(report.shared_changes.len());
    let mut kept = 0;
    report.shared_changes.retain_mut(|group| {
        let pictured = group.image_width.is_some();
        ids.push(pictured.then_some(kept));
        if pictured {
            group.id = kept;
            kept += 1;
        }
        pictured
    });
    if kept == ids.len() {
        return;
    }
    for region in report
        .pages
        .iter_mut()
        .flat_map(|p| &mut p.screens)
        .flat_map(|s| &mut s.regions)
    {
        region.shared = region.shared.and_then(|id| ids.get(id).copied().flatten());
    }
    if let Some(images) = images {
        let remap = |id: &mut usize| ids.get(*id).copied().flatten().map(|new| *id = new);
        images.shared.retain_mut(|(id, _)| remap(id).is_some());
        images.variants.retain_mut(|(id, ..)| remap(id).is_some());
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

#[cfg(test)]
mod tests {
    use super::{ComponentKey, RegionKey, SharedGroups, Signature};
    use figstudio_fig::{Document, Guid};

    /// One page whose screens each hold one area of the given side, cropped or not.
    fn report(doc: &Document, areas: &[(u32, bool)]) -> crate::DiffReport {
        let mut report = crate::diff_documents(doc, doc);
        let screens: Vec<_> = areas
            .iter()
            .map(|&(side, cropped)| {
                serde_json::json!({
                    "old_id": [0, 2], "new_id": [0, 2], "name": "", "kind": "FRAME",
                    "status": "Changed", "matched_by": "Id", "moved": false,
                    "width": 200, "height": 200, "pixels_changed": 1,
                    "thumb_old": null, "thumb_new": null,
                    "regions": [{
                        "x": 0, "y": 0, "width": side, "height": side, "pixels_changed": 1,
                        "old_png": null, "new_png": null,
                        "image_width": cropped.then_some(side * 2),
                    }],
                })
            })
            .collect();
        report.pages.push(
            serde_json::from_value(serde_json::json!({
                "id": [0, 1], "old_name": null, "new_name": null, "status": "Changed",
                "added": [], "removed": [], "changed": [], "render": null, "screens": screens,
            }))
            .unwrap(),
        );
        report
    }

    fn finish(doc: &Document, areas: &[(u32, bool)]) -> crate::DiffReport {
        let mut report = report(doc, areas);
        let mut groups = SharedGroups::default();
        // One component group across every screen, so sizes can differ.
        let key = RegionKey {
            signature: Signature(0, 0, 0),
            component: Some(ComponentKey {
                group: Guid(0, 9),
                master: Guid(0, 9),
            }),
        };
        groups.record(0, areas.iter().map(|_| vec![key]).collect());
        let page = report.pages.len() - 1;
        groups.0.iter_mut().for_each(|(at, _)| at.page_index = page);
        groups.finish(&mut report, (doc, doc));
        report
    }

    #[test]
    fn a_shared_card_leads_with_a_cropped_area_and_is_dropped_without_one() {
        let doc =
            Document::from_bytes(include_bytes!("../../../tests/fixtures/structure.fig")).unwrap();
        // The larger area was kept without its crop (the page byte cap), so the smaller,
        // cropped one leads.
        let report = finish(&doc, &[(20, true), (40, false)]);
        assert_eq!(report.shared_changes[0].occurrences[0].screen_index, 0);
        assert_eq!(report.shared_changes[0].image_width, Some(40));
        assert_eq!(
            finish(&doc, &[(20, false), (40, true)]).shared_changes[0].width,
            40
        );
        // No occurrence has a picture: the card waits for a component view, and without
        // one it is dropped and the screens stay unfolded.
        let mut report = finish(&doc, &[(20, false), (40, false)]);
        assert_eq!(report.shared_changes.len(), 1);
        // A later card with a component view stays and takes the first id, pictures too.
        let mut viewed = report.shared_changes[0].clone();
        viewed.id = 1;
        viewed.image_width = Some(10);
        report.shared_changes.push(viewed);
        let page = report.pages.len() - 1;
        report.pages[page].screens[1].regions[0].shared = Some(1);
        let mut images = crate::RenderImages {
            shared: vec![(1, crate::RegionImages::default())],
            variants: vec![(1, 1, crate::RegionImages::default())],
            ..crate::RenderImages::default()
        };
        super::drop_unpictured(&mut report, Some(&mut images));
        assert_eq!(report.shared_changes.len(), 1);
        assert_eq!(report.shared_changes[0].id, 0);
        let regions: Vec<_> = report.pages[page]
            .screens
            .iter()
            .map(|s| s.regions[0].shared)
            .collect();
        assert_eq!(regions, [None, Some(0)]);
        assert_eq!(images.shared.iter().map(|s| s.0).collect::<Vec<_>>(), [0]);
        assert_eq!(
            images
                .variants
                .iter()
                .map(|v| (v.0, v.1))
                .collect::<Vec<_>>(),
            [(0, 1)]
        );
    }
}
