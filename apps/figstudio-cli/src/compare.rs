//! `compare`: diff two `.fig` files and write a report artifact folder.

use std::fs::File;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use figstudio_diff::{
    DiffReport, RenderImages, RenderOpts, attach_render_diffs_with_sink, diff_documents,
    render_html,
};
use figstudio_fig::{Document, Guid, archive};
use figstudio_render::images::ImageRequest;

/// Diffs `old` against `new` into `out`, printing one counts line.
///
/// `render` writes per-page PNGs under `out/render/` and links them from the
/// report; `html` writes `report.html`. Renders exist only for the HTML, so
/// `html == false` skips them. Stdout carries counts and the artifact dir only.
pub fn run(
    old_path: &Path,
    new_path: &Path,
    out: &Path,
    scale: f64,
    system_fonts: bool,
    render: bool,
    html: bool,
) -> Result<()> {
    // Both inputs open before any file is written, so a bad input leaves no artifact.
    let old = open(old_path, "older")?;
    let new = open(new_path, "newer")?;
    let mut report = diff_documents(&old, &new);
    report.old.path_label = file_label(old_path);
    report.new.path_label = file_label(new_path);
    if render && html {
        let opts = RenderOpts {
            scale,
            system_fonts,
            ..RenderOpts::default()
        };
        render_pages(old_path, new_path, (&old, &new), out, &mut report, &opts)?;
    }
    std::fs::create_dir_all(out).map_err(|e| anyhow!("cannot create {}: {e}", out.display()))?;
    let json = report
        .to_json()
        .map_err(|e| anyhow!("cannot serialize the report: {e}"))?;
    write(out.join("report.json"), json.as_bytes())?;
    if html {
        write(out.join("report.html"), render_html(&report).as_bytes())?;
    }
    let s = &report.summary;
    println!(
        "pages changed {}, nodes added {}, removed {}, changed {}, moved {}",
        s.pages_changed, s.nodes_added, s.nodes_removed, s.nodes_changed, s.nodes_moved
    );
    println!("{}/", out.display());
    Ok(())
}

/// Opens through the same bounded reader and lazy threshold as the other commands.
/// Failures print fixed text: decoder messages can name archive members or paths.
fn open(path: &Path, which: &str) -> Result<Document> {
    Document::open_auto(path).map_err(|_| anyhow!("cannot read the {which} file"))
}

/// The file name only: enough to tell the two versions apart, without directory paths.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "(unnamed)".into(), |n| n.to_string_lossy().into_owned())
}

fn write(path: std::path::PathBuf, bytes: &[u8]) -> Result<()> {
    std::fs::write(&path, bytes).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// Where one page's PNGs went, relative to the report folder.
type Written = (Guid, [Option<String>; 3]);

/// Renders changed pages one at a time and writes each page's PNGs as soon as it is drawn, so
/// memory holds one page of pixels, then links the written files from the report.
fn render_pages(
    old_path: &Path,
    new_path: &Path,
    (old, new): (&Document, &Document),
    out: &Path,
    report: &mut DiffReport,
    opts: &RenderOpts,
) -> Result<()> {
    let dir = out.join("render");
    std::fs::create_dir_all(&dir).map_err(|e| anyhow!("cannot create {}: {e}", dir.display()))?;
    let (old_file, new_file) = (lazy_reader(old, old_path), lazy_reader(new, new_path));
    let old_read = |want: &ImageRequest| read_lazy(&old_file, want);
    let new_read = |want: &ImageRequest| read_lazy(&new_file, want);
    let mut written: Vec<Written> = Vec::new();
    let mut failure = None;
    attach_render_diffs_with_sink(
        old,
        new,
        report,
        opts,
        &old_read,
        &new_read,
        &mut |id, images| {
            if failure.is_some() {
                return;
            }
            match write_page(out, id, images) {
                Ok(names) => written.push((id, names)),
                Err(e) => failure = Some(e),
            }
        },
    );
    if let Some(e) = failure {
        return Err(e);
    }
    for (id, [before, after, diff]) in written {
        if let Some(render) = report
            .pages
            .iter_mut()
            .find(|p| p.id == id)
            .and_then(|p| p.render.as_mut())
        {
            render.old_png = before;
            render.new_png = after;
            render.diff_png = diff;
        }
    }
    Ok(())
}

/// The archive, held open only when the document keeps its images lazily.
fn lazy_reader(doc: &Document, path: &Path) -> Option<Mutex<File>> {
    if doc.lazy_images.is_empty() {
        return None;
    }
    File::open(path).ok().map(Mutex::new)
}

fn read_lazy(file: &Option<Mutex<File>>, want: &ImageRequest) -> Option<Vec<u8>> {
    let mut file = file.as_ref()?.lock().ok()?;
    archive::read_entry(&mut *file, &want.entry).ok()
}

/// Writes one page's PNGs; guids print as "a:b", and colons are unsafe in names and links.
fn write_page(out: &Path, id: Guid, images: &RenderImages) -> Result<[Option<String>; 3]> {
    let base = format!("render/{}-{}-", id.0, id.1);
    let put = |suffix: &str, bytes: &Option<Vec<u8>>| -> Result<Option<String>> {
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let name = format!("{base}{suffix}.png");
        write(out.join(&name), bytes)?;
        Ok(Some(name))
    };
    Ok([
        put("before", &images.before)?,
        put("after", &images.after)?,
        put("diff", &images.diff)?,
    ])
}
