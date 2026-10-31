//! Background threads: file loading, canvas rendering and image reading, so the UI never
//! blocks.
//!
//! Desktop: OS threads; the render thread waits on its channel and wakes the UI when done.
//!
//! Browser: the threads are Web Workers in the Rayon pool (`wasm-bindgen-rayon`). The UI
//! thread must never wait on a lock (browsers forbid blocking the main thread), so here no
//! worker touches the egui context or waits on a channel the UI sends to: the render loop polls
//! for jobs, and the UI repaints while [`Workers::busy`] says something is in flight.
//!
//! Big files open lazily (`source.rs`): a frame lists the images it still needs, the render
//! loop hands each to an image reader (a few at a time), and when one arrives the UI renders
//! the view again ([`Workers::images_arrived`]).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use figstudio_fig::{Document, Guid};
use figstudio_render::Stats;
use figstudio_render::color::{AlphaColor, Srgb};
use figstudio_render::images::{ImageRequest, Images};
use web_time::{Duration, Instant};

#[cfg(target_arch = "wasm32")]
use crate::gpu_scenes::{GpuTexture, TileGpu};
#[cfg(not(target_arch = "wasm32"))]
use crate::gpu_tiles::{GpuTexture, TileGpu};
use crate::recordings::{Pixels, Recordings};
use crate::source::Source;
use crate::tile_queue::{Msg, Queue};
use crate::tiles;

/// Image reads running at once (each holds the compressed bytes and the decoded copy).
const READERS: usize = 4;

/// A batch of tiles to render for one tab, nearest the view centre first. A newer batch for
/// the same tab replaces the tiles of this one not rendered yet (they left the view).
pub struct RenderJob {
    pub tab: u64,
    pub doc: Arc<Document>,
    /// Where unread images come from (lazily opened documents).
    pub source: Option<Source>,
    pub root: Guid,
    pub background: Option<AlphaColor<Srgb>>,
    /// `Workers::images_arrived` when requested, echoed back (see `tiles::Entry::images`).
    pub images: u64,
    pub tiles: VecDeque<tiles::Key>,
    /// Tiles (and their `images`) the UI received since its last batch, so the worker
    /// stops skipping them: a received tile asked for again was evicted and is rendered.
    pub received: Vec<(tiles::Key, u64)>,
}

/// One rendered tile.
pub struct RenderDone {
    pub tab: u64,
    pub key: tiles::Key,
    pub images: u64,
    pub pixels: Pixels<GpuTexture>,
    pub stats: Stats,
    pub elapsed: Duration,
}

pub enum Loaded {
    Ok {
        path: PathBuf,
        doc: Arc<Document>,
        doc_key: Option<String>,
        /// The file's resolved path (symlinks and `..` followed), when it is a file on disk;
        /// resolved here so the UI thread never has to.
        real_path: Option<PathBuf>,
        source: Option<Source>,
        elapsed: Duration,
        /// What CJK the names or text need (the browser fetches a font for it).
        cjk: crate::theme::CjkNeed,
        /// Shader code in the folder next to the file (`FILE.fig.shaders/`).
        shaders: Vec<figstudio_mcp_core::shaders::ShaderSource>,
    },
    Err {
        path: PathBuf,
        /// As in `Ok`: the resolved path of a file on disk, so a link search can skip it.
        real_path: Option<PathBuf>,
        message: String,
    },
}

/// Wakes the UI from a worker. Only on desktop; the browser UI polls instead (see module docs).
#[derive(Clone)]
struct Waker {
    #[cfg(not(target_arch = "wasm32"))]
    ctx: egui::Context,
}

impl Waker {
    fn new(_ctx: &egui::Context) -> Self {
        Waker {
            #[cfg(not(target_arch = "wasm32"))]
            ctx: _ctx.clone(),
        }
    }

    fn wake(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.ctx.request_repaint();
    }
}

static PANICKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Chains a panic hook that records any panic, on any thread. The browser build aborts on
/// panic, so a worker that panics just stops: its results, counters and locks never come
/// back, and only this flag tells the UI.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn install_panic_flag() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANICKED.store(true, Ordering::Release);
        previous(info);
    }));
}

/// A panic happened since [`install_panic_flag`].
pub fn panicked() -> bool {
    PANICKED.load(Ordering::Acquire)
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

/// Runs `f` on a background thread (desktop) or a pool worker (browser), for long-lived
/// helpers such as the motion preview's renderer.
pub fn spawn_task(f: impl FnOnce() + Send + 'static) {
    spawn(f);
}

#[cfg(target_arch = "wasm32")]
fn spawn(f: impl FnOnce() + Send + 'static) {
    rayon::spawn(f);
}

/// Waits for the next job. Desktop blocks on the channel; the browser polls (module docs).
fn next_job(jobs: &Receiver<Msg>) -> Option<Msg> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        jobs.recv().ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        // Doubles from 4 ms to 32 ms over empty polls: ~30 idle wakeups a second, not 250.
        let mut nap = 4;
        loop {
            match jobs.try_recv() {
                Ok(job) => return Some(job),
                // The loop holds a pool thread, so run queued pool work (file decoding,
                // exports) while idle; with a one-thread pool nothing else would.
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if rayon::yield_now() == Some(rayon::Yield::Executed) {
                        nap = 4;
                    } else {
                        std::thread::sleep(Duration::from_millis(nap));
                        nap = (nap * 2).min(32);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            }
        }
    }
}

/// Images waiting to be read, newest request first, with at most [`READERS`] reading.
struct ImageQueue {
    pending: Mutex<(VecDeque<(ImageRequest, Source)>, usize)>,
    images: Images,
    arrived: AtomicU64,
    waker: Waker,
    in_flight: Arc<AtomicUsize>,
}

impl ImageQueue {
    /// Stops reading and storing images of `instance` and drops those already stored. The
    /// image cache refuses anything for it from now on (`Images::close_document`), so a read
    /// or render still running cannot store it back.
    fn close(&self, instance: u64) {
        self.images.close_document(instance);
        let prefix = format!("{instance:x}/");
        let mut q = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let before = q.0.len();
        q.0.retain(|(want, _)| {
            let keep = !want.key.starts_with(&prefix);
            if !keep {
                self.images.abandon(&want.key, want.level);
            }
            keep
        });
        self.in_flight
            .fetch_sub(before - q.0.len(), Ordering::Relaxed);
    }

    fn push(self: &Arc<Self>, wanted: Vec<ImageRequest>, source: &Source) {
        {
            let mut q = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            // What the latest frame wants goes first: it is what is on screen now.
            for want in wanted.into_iter().rev() {
                self.in_flight.fetch_add(1, Ordering::Relaxed);
                q.0.push_front((want, source.clone()));
            }
        }
        self.pump();
    }

    /// Starts readers up to the limit.
    fn pump(self: &Arc<Self>) {
        loop {
            let next = {
                let mut q = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
                if q.1 >= READERS {
                    return;
                }
                let Some(next) = q.0.pop_front() else {
                    return;
                };
                q.1 += 1;
                next
            };
            let this = self.clone();
            spawn(move || {
                let (want, source) = next;
                // A document closed meanwhile gets nothing stored (checked under the cache lock).
                match source.read(&want.entry) {
                    Ok(bytes) => {
                        this.images.insert_encoded(&want.key, &bytes, want.level);
                    }
                    Err(e) => {
                        tracing::debug!(error = %figstudio_fig::one_line(e.as_ref()), "image read failed");
                        this.images.fail(&want.key, want.level);
                    }
                }
                this.pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .1 -= 1;
                this.arrived.fetch_add(1, Ordering::Relaxed);
                this.in_flight.fetch_sub(1, Ordering::Relaxed);
                this.waker.wake();
                this.pump();
            });
        }
    }
}

/// The outcome of one background export (desktop), reported once by `poll`.
#[cfg(not(target_arch = "wasm32"))]
pub struct ExportDone {
    pub path: PathBuf,
    /// What in the file is not drawn exactly (empty when everything is).
    pub result: anyhow::Result<String>,
}

pub struct Workers {
    render_tx: Option<Sender<Msg>>,
    pub render_rx: Receiver<RenderDone>,
    load_tx: Sender<Loaded>,
    pub load_rx: Receiver<Loaded>,
    waker: Waker,
    /// Jobs sent and not yet finished or dropped.
    in_flight: Arc<AtomicUsize>,
    queue: Arc<ImageQueue>,
    #[cfg(not(target_arch = "wasm32"))]
    export_tx: Sender<ExportDone>,
    #[cfg(not(target_arch = "wasm32"))]
    pub export_rx: Receiver<ExportDone>,
    /// Set while an export runs (desktop and browser).
    export_busy: Arc<std::sync::atomic::AtomicBool>,
}

impl Workers {
    /// Workers drawing every canvas tile on the CPU.
    #[cfg_attr(not(test), expect(dead_code, reason = "the app passes its GPU"))]
    pub fn new(ctx: &egui::Context) -> Self {
        Self::with_gpu(ctx, TileGpu::default())
    }

    /// Workers whose render thread draws canvas tiles with `gpu` where it can.
    pub fn with_gpu(ctx: &egui::Context, mut gpu: TileGpu) -> Self {
        let (render_tx, jobs) = channel::<Msg>();
        let (done_tx, render_rx) = channel();
        let waker = Waker::new(ctx);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(ImageQueue {
            pending: Mutex::new((VecDeque::new(), 0)),
            images: Images::default(),
            arrived: AtomicU64::new(0),
            waker: waker.clone(),
            in_flight: in_flight.clone(),
        });
        let repaint = waker.clone();
        let count = in_flight.clone();
        let images = queue.clone();
        let render_loop = move || {
            #[cfg(not(target_arch = "wasm32"))]
            let mut renderer = crate::renderer(images.images.clone());
            #[cfg(target_arch = "wasm32")]
            let (mut font_generation, mut renderer) =
                crate::canvas_fonts::renderer(images.images.clone());
            let mut recordings = Recordings::default();
            let mut queue = Queue::default();
            loop {
                let first = if queue.is_empty() {
                    match next_job(&jobs) {
                        Some(msg) => Some(msg),
                        None => break,
                    }
                } else {
                    None
                };
                let arrived = first
                    .into_iter()
                    .chain(std::iter::from_fn(|| jobs.try_recv().ok()));
                for msg in arrived {
                    match msg {
                        Msg::Forget { doc } => {
                            recordings.forget(doc);
                            continue;
                        }
                        Msg::Shaders { doc, host } => {
                            images.images.set_shaders(doc, host);
                            recordings.forget(doc);
                            continue;
                        }
                        msg => count.fetch_sub(queue.take(msg), Ordering::Relaxed),
                    };
                }
                if queue.is_empty() {
                    continue;
                }
                let Some((job, key)) = queue.next() else {
                    count.fetch_sub(1, Ordering::Relaxed);
                    continue;
                };
                #[cfg(target_arch = "wasm32")]
                {
                    if crate::canvas_fonts::refresh(&mut font_generation, &mut renderer) {
                        recordings = Recordings::default();
                    }
                }
                let start = Instant::now();
                let drawn = gpu.draw(&mut recordings, &mut renderer, job, &key);
                let elapsed = start.elapsed();
                let (pixels, stats, wanted) = drawn.into_parts();
                // A document closed while drawing stores and requests nothing (`Images`).
                if !wanted.is_empty() {
                    match &job.source {
                        Some(source) => images.push(wanted, source),
                        None => wanted
                            .iter()
                            .for_each(|w| images.images.fail(&w.key, w.level)),
                    }
                }
                let done = RenderDone {
                    tab: job.tab,
                    key,
                    images: job.images,
                    pixels,
                    stats,
                    elapsed,
                };
                let (tab, images) = (job.tab, job.images);
                // Sent before `in_flight` drops, so the browser UI polls until it arrives.
                let sent = done_tx.send(done);
                if queue.sent(tab, key, images) {
                    count.fetch_sub(1, Ordering::Relaxed);
                }
                if sent.is_err() {
                    break;
                }
                repaint.wake();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("figstudio-render".into())
            .spawn(render_loop)
            .expect("spawn render thread");
        #[cfg(target_arch = "wasm32")]
        spawn(render_loop);
        let (load_tx, load_rx) = channel();
        #[cfg(not(target_arch = "wasm32"))]
        let (export_tx, export_rx) = channel();
        Workers {
            render_tx: Some(render_tx),
            render_rx,
            load_tx,
            load_rx,
            waker,
            in_flight,
            queue,
            #[cfg(not(target_arch = "wasm32"))]
            export_tx,
            #[cfg(not(target_arch = "wasm32"))]
            export_rx,
            export_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Something was sent to a worker and has not come back yet. False after a panic: a
    /// trapped browser worker never reports back, so waiting for it would poll forever.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn busy(&self) -> bool {
        !panicked() && self.in_flight.load(Ordering::Relaxed) > 0
    }

    /// Exports one page or layer in the background, reading every image it needs from
    /// `source` (lazily opened files). The result arrives on `export_rx`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export(
        &self,
        doc: Arc<Document>,
        root: Guid,
        format: ExportFormat,
        path: PathBuf,
        source: Option<Source>,
    ) -> Result<(), &'static str> {
        let release = self.begin_export()?;
        let tx = self.export_tx.clone();
        let waker = self.waker.clone();
        let images = self.images();
        std::thread::spawn(move || {
            let _release = release;
            // Kept despite M-PANIC-IS-STOP: without it a panicking export sends no ExportDone,
            // so the user sees nothing (the desktop has no panic flag). This is the guideline's
            // per-task exception: the export only reads an immutable `Arc<Document>`, and the
            // image cache recovers from a poisoned lock, so no half-changed state leaks out. The
            // browser build aborts on panic instead (see `install_panic_flag`).
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                export_with(images, &doc, source.as_ref(), root, format, &path)
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("export worker panicked")));
            drop(_release);
            let _ = tx.send(ExportDone { path, result });
            waker.wake();
        });
        Ok(())
    }

    /// Sends a render job; fails once the render worker is gone.
    pub fn render(&mut self, job: RenderJob) -> Result<(), &'static str> {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        if self
            .render_tx
            .as_ref()
            .is_none_or(|tx| tx.send(Msg::Batch(job)).is_err())
        {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            self.render_tx = None;
            return Err("render worker is unavailable");
        }
        Ok(())
    }

    /// A tab closed: its queued tiles are dropped (the tile being drawn may still finish).
    pub fn close_tab(&self, tab: u64) {
        if let Some(tx) = &self.render_tx {
            let _ = tx.send(Msg::Close { tab });
        }
    }

    /// Draws document `instance`'s shaders with `host` from now on, from scratch: the render
    /// worker installs it and drops the document's page recordings.
    pub fn set_shaders(
        &self,
        instance: u64,
        host: std::sync::Arc<dyn figstudio_render::shader::ShaderFills>,
    ) {
        if let Some(tx) = &self.render_tx {
            let _ = tx.send(Msg::Shaders {
                doc: instance,
                host,
            });
        }
    }

    /// The last tab of document `instance` closed: drops its images, its queued image reads,
    /// and anything a render or read still running would store for it.
    pub fn close_document(&self, instance: u64) {
        self.queue.close(instance);
        if let Some(tx) = &self.render_tx {
            let _ = tx.send(Msg::Forget { doc: instance });
        }
    }

    /// Counts images read so far; when it changes, views showing placeholders render again.
    pub fn images_arrived(&self) -> u64 {
        self.queue.arrived.load(Ordering::Relaxed)
    }

    /// Font and shader code changes use the same freshness stamp as arriving images.
    pub fn fonts_changed(&self) {
        self.queue.arrived.fetch_add(1, Ordering::Relaxed);
    }

    /// The image cache shared by every render (for exports).
    pub fn images(&self) -> Images {
        self.queue.images.clone()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(&self, path: PathBuf) {
        self.decode(path.clone(), true, move || {
            let doc_key = crate::source::doc_key_from_path(&path);
            let (doc, source) = crate::source::open_path(&path)?;
            Ok((doc, source, doc_key))
        });
    }

    /// Decodes dropped bytes (used when the platform gives contents rather than a path).
    pub fn load_bytes(&self, name: PathBuf, bytes: Vec<u8>) {
        self.decode(name, false, move || {
            let doc_key = Some(crate::source::doc_key_from_prefix(
                bytes.len() as u64,
                &bytes,
            ));
            Ok((Document::from_bytes(&bytes)?, None, doc_key))
        });
    }

    /// Opens a document on a worker; `read` returns it, where its unread images are, and its key.
    /// `on_disk`: `path` is a file (not just a name), so its resolved path is looked up too.
    pub fn decode(
        &self,
        path: PathBuf,
        on_disk: bool,
        read: impl FnOnce()
            -> Result<(Document, Option<Source>, Option<String>), figstudio_fig::FigError>
        + Send
        + 'static,
    ) {
        let tx = self.load_tx.clone();
        let waker = self.waker.clone();
        let count = self.in_flight.clone();
        count.fetch_add(1, Ordering::Relaxed);
        spawn(move || {
            let start = Instant::now();
            let msg = match read() {
                Ok((doc, source, doc_key)) => {
                    let name = path.file_name().map(|n| n.to_string_lossy());
                    let real_path = on_disk.then(|| std::fs::canonicalize(&path).ok()).flatten();
                    #[cfg(not(target_arch = "wasm32"))]
                    let shaders = if on_disk {
                        figstudio_mcp_core::shader_sources::load(&path)
                    } else {
                        Vec::new()
                    };
                    #[cfg(target_arch = "wasm32")]
                    let shaders = Vec::new();
                    Loaded::Ok {
                        shaders,
                        real_path,
                        cjk: crate::theme::cjk_need(&doc, name.as_deref().unwrap_or("")),
                        path,
                        doc: Arc::new(doc),
                        doc_key,
                        source,
                        elapsed: start.elapsed(),
                    }
                }
                Err(e) => Loaded::Err {
                    real_path: on_disk.then(|| std::fs::canonicalize(&path).ok()).flatten(),
                    path,
                    message: e.to_string(),
                },
            };
            let _ = tx.send(msg);
            count.fetch_sub(1, Ordering::Relaxed);
            waker.wake();
        });
    }

    /// Takes the one export slot (desktop and browser alike: an export can hold a 1 GiB
    /// frame); it is free again when the returned value is dropped.
    pub(crate) fn begin_export(&self) -> Result<ExportRelease, &'static str> {
        self.export_busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| "an export is already running")?;
        Ok(ExportRelease(self.export_busy.clone()))
    }

    /// Runs export `f` on a worker and sends its result to `tx` (browser); refused while
    /// another export runs.
    #[cfg(target_arch = "wasm32")]
    pub fn run_export<T: Send + 'static>(
        &self,
        tx: Sender<T>,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<(), &'static str> {
        let release = self.begin_export()?;
        let count = self.in_flight.clone();
        count.fetch_add(1, Ordering::Relaxed);
        spawn(move || {
            let result = f();
            drop(release);
            let _ = tx.send(result);
            count.fetch_sub(1, Ordering::Relaxed);
        });
        Ok(())
    }
}

pub(crate) struct ExportRelease(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ExportRelease {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Renders `root` of an immutable snapshot at `scale` and writes it as a PNG to `path`. Runs on a
/// worker thread, so the UI thread never waits for a large render or encode.
#[cfg(all(test, not(target_arch = "wasm32")))]
fn export_png(
    doc: &Document,
    root: Guid,
    scale: f64,
    path: &std::path::Path,
) -> anyhow::Result<String> {
    export_with(
        Images::default(),
        doc,
        None,
        root,
        ExportFormat::Png(scale),
        path,
    )
}

/// What File → Export writes: a PNG at a scale, or an SVG in design units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportFormat {
    Png(f64),
    Svg,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png(_) => "png",
            Self::Svg => "svg",
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            Self::Png(_) => "PNG image",
            Self::Svg => "SVG image",
        }
    }
}

/// [`export_png`] in `format`, drawing from `images` and reading unread images from `source`.
/// Returns what in the file is not drawn exactly (empty when everything is).
#[cfg(not(target_arch = "wasm32"))]
pub fn export_with(
    images: Images,
    doc: &Document,
    source: Option<&Source>,
    root: Guid,
    format: ExportFormat,
    path: &std::path::Path,
) -> anyhow::Result<String> {
    use anyhow::Context;
    let (bytes, gaps) = crate::export_bytes(images, doc, source, root, format)?;
    std::fs::write(path, bytes)
        .with_context(|| format!("write the {}", format.extension().to_uppercase()))?;
    Ok(gaps)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod export_tests {
    use super::*;
    use figstudio_render::Renderer;

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
        // A page is cropped to what is drawn on it.
        let expected = Renderer::new()
            .render_export_complete(&doc, doc.pages[0], 1.0, None, |_| None)
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
    fn an_export_says_what_it_does_not_draw_exactly() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/broken_images.fig"
        );
        let doc = Document::open(path).unwrap();
        let dir = std::env::temp_dir().join(format!("figstudio-gaps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (format, file) in [
            (ExportFormat::Svg, "page.svg"),
            (ExportFormat::Png(1.0), "page.png"),
        ] {
            let gaps = export_with(
                Images::default(),
                &doc,
                None,
                doc.pages[0],
                format,
                &dir.join(file),
            )
            .unwrap();
            assert!(gaps.contains("images missing"), "{file}: {gaps}");
        }
        // A page drawn exactly says nothing.
        let corners = Document::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/corners.fig"
        ))
        .unwrap();
        let clean = export_with(
            Images::default(),
            &corners,
            None,
            corners.pages[0],
            ExportFormat::Svg,
            &dir.join("clean.svg"),
        )
        .unwrap();
        assert_eq!(clean, "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_writes_an_svg_of_the_selection_in_design_units() {
        let doc = structure();
        let dir = std::env::temp_dir().join(format!("figstudio-svg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("layer.svg");
        let layer = Guid(1, 34);
        export_with(
            Images::default(),
            &doc,
            None,
            layer,
            ExportFormat::Svg,
            &out,
        )
        .unwrap();
        let svg = std::fs::read_to_string(&out).unwrap();
        // The layer's own box (it draws nothing outside it), as Figma exports it, and the
        // same box as its PNG export.
        let b = figstudio_render::node_bounds(&doc, layer).unwrap();
        let size = format!(
            r#"width="{}" height="{}""#,
            b.width().round(),
            b.height().round()
        );
        let png = Renderer::new()
            .render_export_complete(&doc, layer, 1.0, None, |_| None)
            .unwrap();
        assert_eq!(
            (
                f64::from(png.pixmap.width()),
                f64::from(png.pixmap.height())
            ),
            (b.width().round(), b.height().round())
        );
        assert!(
            svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""),
            "{svg}"
        );
        assert!(svg.contains(&size), "{size} in {svg}");
        assert!(svg.contains("<path"), "{svg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_worker_panic_is_flagged_and_stops_busy() {
        let ctx = egui::Context::default();
        let workers = Workers::new(&ctx);
        workers.in_flight.fetch_add(1, Ordering::Relaxed);
        assert!(workers.busy());
        install_panic_flag();
        assert!(std::thread::spawn(|| panic!("probe")).join().is_err());
        assert!(panicked(), "the panic was not recorded");
        assert!(!workers.busy(), "still busy after a worker panic");
    }

    #[test]
    fn export_slot_is_shared_and_released_on_drop() {
        let workers = Workers::new(&egui::Context::default());
        let slot = workers.begin_export().unwrap();
        assert!(workers.begin_export().is_err(), "second export admitted");
        drop(slot);
        assert!(workers.begin_export().is_ok());
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
            .export(
                doc.clone(),
                doc.pages[0],
                ExportFormat::Png(0.1),
                path.clone(),
                None,
            )
            .unwrap();
        assert_eq!(
            workers.export(
                doc.clone(),
                doc.pages[0],
                ExportFormat::Png(0.1),
                path.clone(),
                None
            ),
            Err("an export is already running")
        );
        let done = workers
            .export_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(done.result.is_ok());
        assert!(
            workers
                .export(
                    doc.clone(),
                    doc.pages[0],
                    ExportFormat::Png(0.1),
                    path.clone(),
                    None
                )
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
        assert!(format!("{err:#}").contains("write the PNG"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "worker_close_tests.rs"]
mod close_tests;
