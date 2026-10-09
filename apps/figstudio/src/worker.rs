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
use figstudio_render::images::{ImageRequest, Images};
use figstudio_render::{Renderer, Stats, Viewport};
use web_time::{Duration, Instant};

use crate::source::Source;
use crate::tab::View;

/// Image reads running at once (each holds the compressed bytes and the decoded copy).
const READERS: usize = 4;

pub struct RenderJob {
    pub tab: u64,
    pub generation: u64,
    pub doc: Arc<Document>,
    /// Where unread images come from (lazily opened documents).
    pub source: Option<Source>,
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
    loop {
        match jobs.try_recv() {
            Ok(job) => return Some(job),
            // The loop holds a pool thread, so run queued pool work (file decoding, exports)
            // while idle; with a one-thread pool nothing else would.
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if rayon::yield_now() != Some(rayon::Yield::Executed) {
                    std::thread::sleep(Duration::from_millis(4));
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
        }
    }
}

fn draw(renderer: &mut Renderer, job: &RenderJob) -> figstudio_render::Frame {
    #[cfg(target_arch = "wasm32")]
    {
        renderer.render_parallel(&job.doc, job.root, &job.viewport)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        renderer.render(&job.doc, job.root, &job.viewport)
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
                        this.images.insert_encoded(&want.hash, &bytes, want.level);
                    }
                    Err(_) => this.images.fail(&want.hash, want.level),
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

pub struct Workers {
    render_tx: Sender<RenderJob>,
    pub render_rx: Receiver<RenderDone>,
    load_tx: Sender<Loaded>,
    pub load_rx: Receiver<Loaded>,
    waker: Waker,
    /// Jobs sent and not yet finished or dropped.
    in_flight: Arc<AtomicUsize>,
    queue: Arc<ImageQueue>,
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
            loop {
                if queue.is_empty() {
                    match next_job(&jobs) {
                        Some(job) => queue.push(job),
                        None => break,
                    }
                }
                // Only the newest request per tab matters; drop older ones still queued.
                while let Ok(newer) = jobs.try_recv() {
                    let before = queue.len();
                    queue.retain(|j| j.tab != newer.tab);
                    count.fetch_sub(before - queue.len(), Ordering::Relaxed);
                    queue.push(newer);
                }
                let job = queue.remove(0);
                let start = Instant::now();
                let mut frame = draw(&mut renderer, &job);
                let elapsed = start.elapsed();
                let wanted = std::mem::take(&mut frame.wanted);
                if !wanted.is_empty() {
                    match &job.source {
                        Some(source) => images.push(wanted, source),
                        None => wanted
                            .iter()
                            .for_each(|w| images.images.fail(&w.hash, w.level)),
                    }
                }
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
                    elapsed,
                };
                let sent = done_tx.send(done);
                count.fetch_sub(1, Ordering::Relaxed);
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
        Workers {
            render_tx,
            render_rx,
            load_tx,
            load_rx,
            waker,
            in_flight,
            queue,
        }
    }

    /// Something was sent to a worker and has not come back yet.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn busy(&self) -> bool {
        self.in_flight.load(Ordering::Relaxed) > 0
    }

    /// Counts images read so far; when it changes, views showing placeholders render again.
    pub fn images_arrived(&self) -> u64 {
        self.queue.arrived.load(Ordering::Relaxed)
    }

    /// The image cache shared by every render (for exports).
    pub fn images(&self) -> Images {
        self.queue.images.clone()
    }

    pub fn render(&self, job: RenderJob) {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        if self.render_tx.send(job).is_err() {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
        }
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

    /// Runs `f` on a worker and sends its result to `tx` (browser export).
    #[cfg(target_arch = "wasm32")]
    pub fn run<T: Send + 'static>(&self, tx: Sender<T>, f: impl FnOnce() -> T + Send + 'static) {
        let count = self.in_flight.clone();
        count.fetch_add(1, Ordering::Relaxed);
        spawn(move || {
            let _ = tx.send(f());
            count.fetch_sub(1, Ordering::Relaxed);
        });
    }
}
