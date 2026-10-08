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

pub struct Workers {
    render_tx: Sender<RenderJob>,
    pub render_rx: Receiver<RenderDone>,
    load_tx: Sender<Loaded>,
    pub load_rx: Receiver<Loaded>,
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
        Workers {
            render_tx,
            render_rx,
            load_tx,
            load_rx,
            ctx: ctx.clone(),
        }
    }

    pub fn render(&self, job: RenderJob) {
        let _ = self.render_tx.send(job);
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
