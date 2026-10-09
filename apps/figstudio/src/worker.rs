//! Background threads: file loading and canvas rendering, so the UI never blocks.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use figstudio_fig::{Document, Guid};
use figstudio_render::{Renderer, Stats, Viewport};

use crate::tab::View;

pub struct RenderJob {
    pub tab: u64,
    pub generation: u64,
    pub doc: Arc<Document>,
    pub root: Guid,
    pub viewport: Viewport,
    /// Echoed back so the frame can be placed correctly after the view has moved on.
    pub view: View,
    pub size_points: egui::Vec2,
}

pub struct RenderDone {
    pub tab: u64,
    pub generation: u64,
    pub root: Guid,
    pub view: View,
    pub size_points: egui::Vec2,
    pub size: [usize; 2],
    pub rgba_premultiplied: Vec<u8>,
    pub stats: Stats,
    pub elapsed: Duration,
}

pub enum Loaded {
    Ok {
        path: PathBuf,
        doc: Arc<Document>,
        elapsed: Duration,
    },
    Err {
        path: PathBuf,
        message: String,
    },
}

/// The outcome of one background export, reported once by `poll`.
pub struct ExportDone {
    pub path: PathBuf,
    pub result: Result<(), String>,
}

pub struct Workers {
    render_tx: Option<Sender<RenderJob>>,
    pub render_rx: Receiver<RenderDone>,
    load_tx: Sender<Loaded>,
    pub load_rx: Receiver<Loaded>,
    export_tx: Sender<ExportDone>,
    pub export_rx: Receiver<ExportDone>,
    export_busy: Arc<std::sync::atomic::AtomicBool>,
    ctx: egui::Context,
}

impl Workers {
    pub fn new(ctx: &egui::Context) -> Self {
        let (render_tx, jobs) = channel::<RenderJob>();
        let (done_tx, render_rx) = channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("figstudio-render".into())
            .spawn(move || {
                let mut renderer = Renderer::new();
                let mut queue: Vec<RenderJob> = Vec::new();
                loop {
                    if queue.is_empty() {
                        match jobs.recv() {
                            Ok(job) => queue.push(job),
                            Err(_) => break,
                        }
                    }
                    // Only the newest request per tab matters; drop older ones still queued.
                    while let Ok(newer) = jobs.try_recv() {
                        queue.retain(|j| j.tab != newer.tab);
                        queue.push(newer);
                    }
                    let job = queue.remove(0);
                    let start = Instant::now();
                    let frame = renderer.render(&job.doc, job.root, &job.viewport);
                    let size = [
                        usize::from(frame.pixmap.width()),
                        usize::from(frame.pixmap.height()),
                    ];
                    let done = RenderDone {
                        tab: job.tab,
                        generation: job.generation,
                        root: job.root,
                        view: job.view,
                        size_points: job.size_points,
                        size,
                        rgba_premultiplied: frame.pixmap.data_as_u8_slice().to_vec(),
                        stats: frame.stats,
                        elapsed: start.elapsed(),
                    };
                    if done_tx.send(done).is_err() {
                        break;
                    }
                    repaint.request_repaint();
                }
            })
            .expect("spawn render thread");
        let (load_tx, load_rx) = channel();
        let (export_tx, export_rx) = channel();
        Workers {
            render_tx: Some(render_tx),
            render_rx,
            load_tx,
            load_rx,
            export_tx,
            export_rx,
            export_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ctx: ctx.clone(),
        }
    }

    /// Exports one page or layer in the background. The result arrives on `export_rx`.
    pub fn export(
        &self,
        doc: Arc<Document>,
        root: Guid,
        scale: f64,
        path: PathBuf,
    ) -> Result<(), &'static str> {
        self.export_busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| "an export is already running")?;
        let busy = self.export_busy.clone();
        let tx = self.export_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _release = ExportRelease(busy);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                export_png(&doc, root, scale, &path)
            }))
            .unwrap_or_else(|_| Err("export worker panicked".into()));
            drop(_release);
            let _ = tx.send(ExportDone { path, result });
            ctx.request_repaint();
        });
        Ok(())
    }

    pub fn render(&mut self, job: RenderJob) -> Result<(), &'static str> {
        if self
            .render_tx
            .as_ref()
            .is_none_or(|tx| tx.send(job).is_err())
        {
            self.render_tx = None;
            return Err("render worker is unavailable");
        }
        Ok(())
    }

    pub fn load(&self, path: PathBuf) {
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let msg = match Document::open(&path) {
                Ok(doc) => Loaded::Ok {
                    path,
                    doc: Arc::new(doc),
                    elapsed: start.elapsed(),
                },
                Err(e) => Loaded::Err {
                    path,
                    message: e.to_string(),
                },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    /// Decodes dropped bytes (used when the platform gives contents rather than a path).
    pub fn load_bytes(&self, name: PathBuf, bytes: Vec<u8>) {
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let msg = match Document::from_bytes(&bytes) {
                Ok(doc) => Loaded::Ok {
                    path: name,
                    doc: Arc::new(doc),
                    elapsed: start.elapsed(),
                },
                Err(e) => Loaded::Err {
                    path: name,
                    message: e.to_string(),
                },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }
}

struct ExportRelease(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ExportRelease {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Renders `root` of an immutable snapshot at `scale` and writes it as a PNG to `path`. Runs on a
/// worker thread, so the UI thread never waits for a large render or encode.
pub fn export_png(
    doc: &Document,
    root: Guid,
    scale: f64,
    path: &std::path::Path,
) -> Result<(), String> {
    let mut renderer = Renderer::new();
    let frame = renderer
        .render_fit(doc, root, scale, 0.0, None)
        .ok_or_else(|| "nothing to export".to_string())?;
    let png = frame.pixmap.into_png().map_err(|e| e.to_string())?;
    std::fs::write(path, png).map_err(|e| e.to_string())
}

#[cfg(test)]
mod export_tests {
    use super::*;

    fn structure() -> Document {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/structure.fig"
        );
        Document::open(path).unwrap()
    }

    #[test]
    fn export_writes_a_png_of_the_page() {
        let doc = structure();
        let dir = std::env::temp_dir().join(format!("figstudio-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("page.png");
        export_png(&doc, doc.pages[0], 1.0, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        let expected = Renderer::new()
            .render_fit(&doc, doc.pages[0], 1.0, 0.0, None)
            .unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (
                u32::from(expected.pixmap.width()),
                u32::from(expected.pixmap.height())
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worker_admits_only_one_export_and_releases_slot_on_completion() {
        let ctx = egui::Context::default();
        let workers = Workers::new(&ctx);
        let doc = Arc::new(structure());
        let dir =
            std::env::temp_dir().join(format!("figstudio-export-admission-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("page.png");
        workers
            .export(doc.clone(), doc.pages[0], 0.1, path.clone())
            .unwrap();
        assert_eq!(
            workers.export(doc.clone(), doc.pages[0], 0.1, path.clone()),
            Err("an export is already running")
        );
        let done = workers
            .export_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(done.result.is_ok());
        assert!(
            workers
                .export(doc.clone(), doc.pages[0], 0.1, path.clone())
                .is_ok()
        );
        let _ = workers
            .export_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn export_reports_a_failed_write_instead_of_panicking() {
        let doc = structure();
        let dir = std::env::temp_dir().join(format!("figstudio-export-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A directory where the file should be: the write must fail and say so.
        let err = export_png(&doc, doc.pages[0], 1.0, &dir).expect_err("writing onto a directory");
        assert!(!err.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
