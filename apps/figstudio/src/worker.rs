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
use figstudio_render::color::{AlphaColor, Srgb};
use figstudio_render::images::{ImageRequest, Images};
use figstudio_render::{Renderer, Stats, Viewport};
use web_time::{Duration, Instant};

use crate::source::Source;
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
}

/// One rendered tile.
pub struct RenderDone {
    pub tab: u64,
    pub key: tiles::Key,
    pub images: u64,
    pub size: [usize; 2],
    pub rgba_premultiplied: Vec<u8>,
    pub stats: Stats,
    pub elapsed: Duration,
}

pub enum Loaded {
    Ok {
        path: PathBuf,
        doc: Arc<Document>,
        source: Option<Source>,
        elapsed: Duration,
        /// Names or text need a CJK font (the browser fetches one).
        needs_cjk: bool,
    },
    Err {
        path: PathBuf,
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

#[cfg(target_arch = "wasm32")]
fn spawn(f: impl FnOnce() + Send + 'static) {
    rayon::spawn(f);
}

/// Waits for the next job. Desktop blocks on the channel; the browser polls (module docs).
fn next_job(jobs: &Receiver<RenderJob>) -> Option<RenderJob> {
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

fn draw(renderer: &mut Renderer, job: &RenderJob, key: &tiles::Key) -> figstudio_render::Frame {
    let viewport = Viewport {
        width: tiles::TILE,
        height: tiles::TILE,
        transform: key.transform(),
        background: job.background,
    };
    #[cfg(target_arch = "wasm32")]
    {
        renderer.render_parallel(&job.doc, job.root, &viewport)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        renderer.render(&job.doc, job.root, &viewport)
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
                match source.read(&want.entry) {
                    Ok(bytes) => {
                        this.images.insert_encoded(&want.key, &bytes, want.level);
                    }
                    Err(_) => this.images.fail(&want.key, want.level),
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
    pub result: Result<(), String>,
}

pub struct Workers {
    render_tx: Option<Sender<RenderJob>>,
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
    pub fn new(ctx: &egui::Context) -> Self {
        let (render_tx, jobs) = channel::<RenderJob>();
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
            let mut renderer = Renderer::with_images(images.images.clone());
            let mut queue: Vec<RenderJob> = Vec::new();
            // Tiles finished since their tab's last batch arrived. That batch was planned
            // before their results reached the UI, so it may ask for them again.
            let mut rendered: Vec<(u64, tiles::Key, u64)> = Vec::new();
            loop {
                let first = if queue.is_empty() {
                    match next_job(&jobs) {
                        Some(job) => Some(job),
                        None => break,
                    }
                } else {
                    None
                };
                // Only the newest batch per tab matters; drop older ones still queued.
                let arrived = first
                    .into_iter()
                    .chain(std::iter::from_fn(|| jobs.try_recv().ok()));
                for mut newer in arrived {
                    let done = |k: &tiles::Key| rendered.contains(&(newer.tab, *k, newer.images));
                    newer.tiles.retain(|k| !done(k));
                    rendered.retain(|r| r.0 != newer.tab);
                    let before = queue.len();
                    queue.retain(|j| j.tab != newer.tab);
                    count.fetch_sub(before - queue.len(), Ordering::Relaxed);
                    // The newest batch is the tab on screen; other tabs' leftovers wait.
                    queue.insert(0, newer);
                }
                let Some(key) = queue[0].tiles.pop_front() else {
                    queue.remove(0);
                    count.fetch_sub(1, Ordering::Relaxed);
                    continue;
                };
                let job = &queue[0];
                let start = Instant::now();
                let mut frame = draw(&mut renderer, job, &key);
                let elapsed = start.elapsed();
                let wanted = std::mem::take(&mut frame.wanted);
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
                    size: [
                        usize::from(frame.pixmap.width()),
                        usize::from(frame.pixmap.height()),
                    ],
                    rgba_premultiplied: frame.pixmap.data_as_u8_slice().to_vec(),
                    stats: frame.stats,
                    elapsed,
                };
                rendered.push((job.tab, key, job.images));
                let last = job.tiles.is_empty();
                // Sent before `in_flight` drops, so the browser UI polls until it arrives.
                let sent = done_tx.send(done);
                if last {
                    queue.remove(0);
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
        scale: f64,
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
                export_png_with(images, &doc, source.as_ref(), root, scale, &path)
            }))
            .unwrap_or_else(|_| Err("export worker panicked".into()));
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
            .is_none_or(|tx| tx.send(job).is_err())
        {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            self.render_tx = None;
            return Err("render worker is unavailable");
        }
        Ok(())
    }

    /// Counts images read so far; when it changes, views showing placeholders render again.
    pub fn images_arrived(&self) -> u64 {
        self.queue.arrived.load(Ordering::Relaxed)
    }

    /// The image cache shared by every render (for exports).
    pub fn images(&self) -> Images {
        self.queue.images.clone()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(&self, path: PathBuf) {
        self.decode(path.clone(), move || crate::source::open_path(&path));
    }

    /// Decodes dropped bytes (used when the platform gives contents rather than a path).
    pub fn load_bytes(&self, name: PathBuf, bytes: Vec<u8>) {
        self.decode(name, move || Ok((Document::from_bytes(&bytes)?, None)));
    }

    /// Opens a document on a worker; `read` returns it and where its unread images are.
    pub fn decode(
        &self,
        path: PathBuf,
        read: impl FnOnce() -> Result<(Document, Option<Source>), figstudio_fig::FigError>
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
                Ok((doc, source)) => {
                    let name = path.file_name().map(|n| n.to_string_lossy());
                    Loaded::Ok {
                        needs_cjk: crate::theme::needs_cjk(&doc, name.as_deref().unwrap_or("")),
                        path,
                        doc: Arc::new(doc),
                        source,
                        elapsed: start.elapsed(),
                    }
                }
                Err(e) => Loaded::Err {
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
    fn begin_export(&self) -> Result<ExportRelease, &'static str> {
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

struct ExportRelease(Arc<std::sync::atomic::AtomicBool>);

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
) -> Result<(), String> {
    export_png_with(Images::default(), doc, None, root, scale, path)
}

/// [`export_png`] drawing from `images` and reading unread images from `source`.
#[cfg(not(target_arch = "wasm32"))]
pub fn export_png_with(
    images: Images,
    doc: &Document,
    source: Option<&Source>,
    root: Guid,
    scale: f64,
    path: &std::path::Path,
) -> Result<(), String> {
    let frame = crate::export_frame(images, doc, source, root, scale)
        .ok_or_else(|| "nothing to export".to_string())?;
    let png = frame.pixmap.into_png().map_err(|e| e.to_string())?;
    std::fs::write(path, png).map_err(|e| e.to_string())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
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
            .export(doc.clone(), doc.pages[0], 0.1, path.clone(), None)
            .unwrap();
        assert_eq!(
            workers.export(doc.clone(), doc.pages[0], 0.1, path.clone(), None),
            Err("an export is already running")
        );
        let done = workers
            .export_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(done.result.is_ok());
        assert!(
            workers
                .export(doc.clone(), doc.pages[0], 0.1, path.clone(), None)
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
