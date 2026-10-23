//! The CLI and MCP share deterministic image names and report-link assignment.
use crate::{DiffReport, ImageReader, RenderImages, RenderOpts, attach_render_diffs_with_sink};
use figstudio_fig::{Document, Guid};
use std::{io, path::Path};

#[derive(Debug)]
struct ScreenLinks {
    before: Option<String>,
    after: Option<String>,
    regions: Vec<(String, String)>,
}

/// Successfully written local links for one page, including its screens.
#[derive(Debug)]
pub struct RenderLinks {
    page: [Option<String>; 3],
    screens: Vec<ScreenLinks>,
    /// `(shared-change id, before, after)` component pictures.
    shared: Vec<(usize, String, String)>,
    /// `(shared-change id, variant index, before, after)`.
    variants: Vec<(usize, usize, String, String)>,
}

impl RenderLinks {
    /// Sets links only after every image for this page has been written successfully.
    pub fn attach(self, report: &mut DiffReport, id: Guid) {
        for (k, before, after) in self.shared {
            if let Some(group) = report
                .shared_changes
                .get_mut(k)
                .filter(|g| g.component_view)
            {
                group.old_png = Some(before);
                group.new_png = Some(after);
            }
        }
        for (k, j, before, after) in self.variants {
            if let Some(v) = report
                .shared_changes
                .get_mut(k)
                .and_then(|g| g.component.as_mut())
                .and_then(|c| c.variants.get_mut(j))
            {
                v.old_png = Some(before);
                v.new_png = Some(after);
            }
        }
        let Some((page_index, page)) = report
            .pages
            .iter_mut()
            .enumerate()
            .find(|(_, p)| p.id == id)
        else {
            return;
        };
        let [before, after, diff] = self.page;
        if let Some(render) = &mut page.render {
            render.old_png = before;
            render.new_png = after;
            render.diff_png = diff;
        }
        for (screen, links) in page.screens.iter_mut().zip(self.screens) {
            screen.thumb_old = links.before;
            screen.thumb_new = links.after;
            // A screen has one crop per area, in order, or none (see `finish_screen`).
            debug_assert!(links.regions.is_empty() || links.regions.len() == screen.regions.len());
            for (region, (before, after)) in screen.regions.iter_mut().zip(links.regions) {
                region.old_png = Some(before);
                region.new_png = Some(after);
            }
        }
        for group in report
            .shared_changes
            .iter_mut()
            .filter(|g| !g.component_view)
        {
            if let Some(first) = group
                .occurrences
                .first()
                .filter(|o| o.page_index == page_index)
                && let Some(region) = page
                    .screens
                    .get(first.screen_index)
                    .and_then(|s| s.regions.get(first.region_index))
            {
                group.old_png.clone_from(&region.old_png);
                group.new_png.clone_from(&region.new_png);
            }
        }
    }
}

/// Writes a page's images under `out/render/`, without changing either source document.
/// The caller creates the render directory and chooses a fresh artifact folder.
pub fn write_render_images(out: &Path, id: Guid, images: &RenderImages) -> io::Result<RenderLinks> {
    let base = format!("render/{}-{}", id.0, id.1);
    let ext = images.format.extension();
    let mut shared = Vec::new();
    for (k, pictures) in &images.shared {
        let name = |side: &str| format!("render/shared-{k}-{side}.{ext}");
        std::fs::write(out.join(name("before")), &pictures.before)?;
        std::fs::write(out.join(name("after")), &pictures.after)?;
        shared.push((*k, name("before"), name("after")));
    }
    let mut variants = Vec::new();
    for (k, j, pictures) in &images.variants {
        let name = |side: &str| format!("render/shared-{k}-v{j}-{side}.{ext}");
        std::fs::write(out.join(name("before")), &pictures.before)?;
        std::fs::write(out.join(name("after")), &pictures.after)?;
        variants.push((*k, *j, name("before"), name("after")));
    }
    let put = |suffix: &str, bytes: Option<&[u8]>| -> io::Result<Option<String>> {
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let name = format!("{base}-{suffix}.{}", images.format.extension());
        std::fs::write(out.join(&name), bytes)?;
        Ok(Some(name))
    };
    let mut screens = Vec::new();
    for (k, screen) in images.screens.iter().enumerate() {
        let mut regions = Vec::new();
        for (j, region) in screen.regions.iter().enumerate() {
            let before = put(&format!("s{k}-a{j}-before"), Some(&region.before))?;
            let after = put(&format!("s{k}-a{j}-after"), Some(&region.after))?;
            regions.extend(before.zip(after));
        }
        screens.push(ScreenLinks {
            before: put(&format!("s{k}-before"), screen.before.as_deref())?,
            after: put(&format!("s{k}-after"), screen.after.as_deref())?,
            regions,
        });
    }
    Ok(RenderLinks {
        page: [
            put("before", images.before.as_deref())?,
            put("after", images.after.as_deref())?,
            put("diff", images.diff.as_deref())?,
        ],
        screens,
        shared,
        variants,
    })
}

/// Renders the changed pages of `old` against `new` into `out/render/` (which the caller has
/// created), writing each page's images as soon as it is drawn so memory holds one page of
/// pixels, then links the written files from `report`. Nothing is linked after a write error.
pub fn render_report_to_dir(
    (old, new): (&Document, &Document),
    report: &mut DiffReport,
    opts: &RenderOpts,
    (old_read, new_read): (ImageReader<'_>, ImageReader<'_>),
    out: &Path,
) -> io::Result<()> {
    let mut written: Vec<(Guid, RenderLinks)> = Vec::new();
    let mut failure = None;
    attach_render_diffs_with_sink(
        old,
        new,
        report,
        opts,
        old_read,
        new_read,
        &mut |id, images| {
            if failure.is_some() {
                return;
            }
            match write_render_images(out, id, images) {
                Ok(links) => written.push((id, links)),
                Err(e) => failure = Some(e),
            }
        },
    );
    if let Some(e) = failure {
        return Err(e);
    }
    for (id, links) in written {
        links.attach(report, id);
    }
    Ok(())
}

/// The file name only: enough to tell two versions apart, without directory paths.
#[must_use]
pub fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || "(unnamed)".into(),
        |name| name.to_string_lossy().into_owned(),
    )
}
