//! `compare`: diff two `.fig` files and write a report artifact folder.

use std::fs::File;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use figstudio_diff::{
    DiffReport, RenderOpts, diff_documents, file_label, render_html, render_report_to_dir,
};
use figstudio_fig::{Document, archive};
use figstudio_render::images::ImageRequest;

/// Diffs `old` against `new` into `out`, printing one counts line.
///
/// `render` writes per-page images under `out/render/` and links them from the
/// report; `html` writes `report.html`. Renders exist only for the HTML, so
/// `html == false` skips them. Stdout carries counts and the artifact dir only.
pub fn run(
    old_path: &Path,
    new_path: &Path,
    out: &Path,
    scale: f64,
    system_fonts: bool,
    render: bool,
    (html, format): (bool, figstudio_render::encoding::ImageFormat),
) -> Result<()> {
    // Both inputs open before any file is written, so a bad input leaves no artifact.
    let old = open(old_path, "older")?;
    let new = open(new_path, "newer")?;
    let mut report = diff_documents(&old, &new);
    report.old.path_label = file_label(old_path);
    report.new.path_label = file_label(new_path);
    if render && html {
        let opts = RenderOpts {
            format,
            scale,
            system_fonts,
            ..RenderOpts::default()
        };
        render_pages(old_path, new_path, (&old, &new), out, &mut report, &opts)?;
    }
    // The report and its HTML are built from `report` alone; free both documents first.
    drop((old, new));
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
        "pages added {}, removed {}, changed {}, nodes added {}, removed {}, changed {}, moved {}",
        s.pages_added,
        s.pages_removed,
        s.pages_changed,
        s.nodes_added,
        s.nodes_removed,
        s.nodes_changed,
        s.nodes_moved
    );
    println!("{}/", out.display());
    Ok(())
}

/// Opens through the same bounded reader and lazy threshold as the other commands.
/// Failures print fixed text: decoder messages can name archive members or paths.
fn open(path: &Path, which: &str) -> Result<Document> {
    Document::open_auto(path).map_err(|_| anyhow!("cannot read the {which} file"))
}

fn write(path: std::path::PathBuf, bytes: &[u8]) -> Result<()> {
    std::fs::write(&path, bytes).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// Renders changed pages one at a time and writes each page's images as soon as it is drawn, so
/// memory holds one page of pixels, then links the written files from the report.
fn render_pages(
    old_path: &Path,
    new_path: &Path,
    docs: (&Document, &Document),
    out: &Path,
    report: &mut DiffReport,
    opts: &RenderOpts,
) -> Result<()> {
    let dir = out.join("render");
    std::fs::create_dir_all(&dir).map_err(|e| anyhow!("cannot create {}: {e}", dir.display()))?;
    let (old_file, new_file) = (lazy_reader(docs.0, old_path), lazy_reader(docs.1, new_path));
    let old_read = |want: &ImageRequest| read_lazy(&old_file, want);
    let new_read = |want: &ImageRequest| read_lazy(&new_file, want);
    render_report_to_dir(docs, report, opts, (&old_read, &new_read), out)?;
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
