//! Copy as PNG: render a page region (or one node) off the UI thread, then put the image on the
//! clipboard from `drive`, which `main.rs` calls every frame.
//!
//! Desktop renders on a `std::thread` holding `Workers::begin_export`'s slot; the browser uses
//! `Workers::run_export`. Either way the pixels come back on a channel. Size is capped by
//! [`plan`]. egui's clipboard holds one item, so this copies the image only; the text is
//! "Copy region".

use std::sync::mpsc::{Receiver, Sender, channel};

use figstudio_fig::{Document, Guid};
use figstudio_render::images::{ImageRequest, Images};
use figstudio_render::{Renderer, Stats, node_bounds};

use crate::source::Source;
use crate::tab::Tab;
use crate::worker::Workers;

use figstudio_mcp_core::region_shot::plan;

/// A node is copied at this scale (the 2x export), within the caps.
const NODE_SCALE: f64 = 2.0;

/// What to render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Job {
    /// `rect` (`[x, y, w, h]`, page units) of `page`, at `zoom` pixels per page unit.
    Region {
        page: Guid,
        rect: [f64; 4],
        zoom: f32,
    },
    /// One node, at 2x.
    Node(Guid),
}

/// A finished render.
#[derive(Debug)]
pub struct Done {
    /// File name for the browser download fallback.
    #[cfg_attr(
        all(not(target_arch = "wasm32"), not(test)),
        expect(dead_code, reason = "browser download only")
    )]
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Premultiplied RGBA8, as Vello writes it.
    pub rgba: Vec<u8>,
    /// PNG bytes (browser only, for the download fallback).
    #[cfg_attr(
        all(not(target_arch = "wasm32"), not(test)),
        expect(dead_code, reason = "browser download only")
    )]
    pub png: Option<Vec<u8>>,
    /// What the render drew approximately or skipped, e.g. `2 effects skipped`.
    pub gaps: String,
}

type Msg = Result<Done, String>;

/// Where finished renders arrive.
#[derive(Debug)]
pub struct Inbox {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for Inbox {
    fn default() -> Self {
        let (tx, rx) = channel();
        Inbox { tx, rx }
    }
}

/// The approximated/skipped/missing parts of `stats`, the way the status bar lists them.
pub fn gaps(stats: &Stats) -> String {
    let notes = stats.to_string();
    let kept: Vec<&str> = notes
        .split(", ")
        .filter(|s| s.contains("skipped") || s.contains("missing") || s.contains("approximated"))
        .collect();
    kept.join(", ")
}

/// Renders `job`. Runs on a worker thread; never panics on a bad job.
pub fn render(
    images: Images,
    doc: &Document,
    source: Option<&Source>,
    job: Job,
    want_png: bool,
) -> Result<Done, String> {
    let (frame, name) = match job {
        Job::Region { page, rect, zoom } => {
            let (viewport, _) =
                figstudio_mcp_core::region_shot::viewport(doc, page, rect, f64::from(zoom))
                    .ok_or("Region is too small to copy as an image")?;
            let read = |want: &ImageRequest| source.and_then(|s| s.read(&want.entry).ok());
            let frame = Renderer::with_images(images).render_complete(doc, page, &viewport, read);
            (frame, "region".to_string())
        }
        Job::Node(id) => {
            let b = node_bounds(doc, id).ok_or("Nothing to copy as an image")?;
            let p = plan(b.width(), b.height(), NODE_SCALE).ok_or("Nothing to copy as an image")?;
            let frame = crate::export_frame(images, doc, source, id, p.scale)
                .ok_or("Nothing to copy as an image")?;
            (
                frame,
                doc.node(id).map_or("node".into(), |n| file_stem(&n.name)),
            )
        }
    };
    let gaps = gaps(&frame.stats);
    let (width, height) = (
        usize::from(frame.pixmap.width()),
        usize::from(frame.pixmap.height()),
    );
    let rgba = frame.pixmap.data_as_u8_slice().to_vec();
    let png = if want_png {
        Some(frame.pixmap.into_png().map_err(|e| e.to_string())?)
    } else {
        None
    };
    Ok(Done {
        name,
        width,
        height,
        rgba,
        png,
        gaps,
    })
}

/// A node name usable as a file name.
fn file_stem(name: &str) -> String {
    let s = name.replace(['/', '\\', '\n', '\r'], "_");
    if s.trim().is_empty() {
        "node".into()
    } else {
        s
    }
}

/// The toast for a copied image: size and the render's gaps.
pub fn message(width: usize, height: usize, gaps: &str) -> String {
    let warn = if gaps.is_empty() {
        String::new()
    } else {
        format!(" · ⚠ {gaps}")
    };
    format!("Copied {width}×{height} image{warn}")
}

/// Handles the picked menu item and the renders that finished. Returns toasts `(text, error)`.
pub fn drive(
    ctx: &egui::Context,
    workers: &Workers,
    inbox: &Inbox,
    tab: Option<&mut Tab>,
) -> Vec<(String, bool)> {
    let mut toasts = Vec::new();
    if let Some(tab) = tab
        && let Some(item) = tab.pending_copy.take()
    {
        match item.png {
            None => toasts.push((crate::copy_menu::copy(ctx, item), false)),
            Some(job) => match start(ctx, workers, inbox, tab, job) {
                Ok(()) => toasts.push(("Rendering image…".into(), false)),
                Err(e) => toasts.push((e.into(), true)),
            },
        }
    }
    while let Ok(msg) = inbox.rx.try_recv() {
        toasts.push(match msg {
            Ok(done) => finish(ctx, done),
            Err(e) => (e, true),
        });
    }
    toasts
}

#[cfg(not(target_arch = "wasm32"))]
fn start(
    ctx: &egui::Context,
    workers: &Workers,
    inbox: &Inbox,
    tab: &Tab,
    job: Job,
) -> Result<(), &'static str> {
    let release = workers.begin_export()?;
    let (tx, ctx) = (inbox.tx.clone(), ctx.clone());
    let (images, doc, source) = (workers.images(), tab.doc.clone(), tab.source.clone());
    std::thread::spawn(move || {
        // Kept despite M-PANIC-IS-STOP, as in `Workers::export`: without it a panicking render
        // sends nothing and the user sees nothing. It reads an immutable `Arc<Document>`.
        let msg = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render(images, &doc, source.as_ref(), job, false)
        }))
        .unwrap_or_else(|_| Err("image render panicked".into()));
        drop(release);
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn start(
    _ctx: &egui::Context,
    workers: &Workers,
    inbox: &Inbox,
    tab: &Tab,
    job: Job,
) -> Result<(), &'static str> {
    let (images, doc, source) = (workers.images(), tab.doc.clone(), tab.source.clone());
    workers.run_export(inbox.tx.clone(), move || {
        render(images, &doc, source.as_ref(), job, true)
    })
}

/// Puts a finished image on the clipboard; in the browser, downloads it when the clipboard
/// cannot take images. Returns the toast.
fn finish(ctx: &egui::Context, done: Done) -> (String, bool) {
    #[cfg(target_arch = "wasm32")]
    if !can_copy_image() {
        let file = format!("{}.png", done.name);
        return match done
            .png
            .as_deref()
            .map(|p| crate::web::download(&file, "image/png", p))
        {
            Some(Ok(())) => (
                format!("This browser cannot copy images here; downloaded {file}"),
                false,
            ),
            Some(Err(e)) => (e, true),
            None => ("Could not make the image".into(), true),
        };
    }
    let image = egui::ColorImage::from_rgba_premultiplied([done.width, done.height], &done.rgba);
    ctx.copy_image(image);
    let text = message(done.width, done.height, &done.gaps);
    // egui-winit only logs a failed image copy (some Wayland setups cannot copy images).
    #[cfg(not(target_arch = "wasm32"))]
    let text = text + "\nIf pasting gives nothing, this system cannot copy images.";
    (text, false)
}

/// The browser can write `image/png` to the clipboard: a secure context with `ClipboardItem`.
/// Safari may still refuse the write later, once the user gesture has expired; that cannot be
/// seen from here.
#[cfg(target_arch = "wasm32")]
fn can_copy_image() -> bool {
    use wasm_bindgen::JsValue;
    let global = js_sys::global();
    let secure = js_sys::Reflect::get(&global, &JsValue::from_str("isSecureContext"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    secure && js_sys::Reflect::has(&global, &JsValue::from_str("ClipboardItem")).unwrap_or(false)
}

#[cfg(test)]
#[path = "region_png_tests.rs"]
mod tests;
