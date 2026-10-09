//! Browser build: entry point, and the things the browser does differently from the desktop
//! (picking and dropping files gives bytes, not paths; exports become downloads).
//!
//! Page setup is in `web/index.html`; build with `scripts/web-build.sh`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Exposed to JavaScript as `initThreadPool(n)`; await it before `start`. (The export comes
/// from the crate's `#[wasm_bindgen]`; this binary never calls it, hence the allow.)
#[allow(unused_imports)]
pub use wasm_bindgen_rayon::init_thread_pool;

/// Starts the app on `canvas`. The Rayon pool must already be running: the render loop and file
/// decoding run on it.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| Ok(Box::new(crate::App::new(cc, Vec::new())))),
        )
        .await
}

type Picked = Result<(PathBuf, Vec<u8>), String>;
type Exported = (String, Result<Vec<u8>, String>);

/// Results that arrive asynchronously; `App::poll` drains them.
pub struct Inbox {
    files_tx: Sender<Picked>,
    pub files: Receiver<Picked>,
    pub exports_tx: Sender<Exported>,
    pub exports: Receiver<Exported>,
}

impl Inbox {
    pub fn new() -> Self {
        let (files_tx, files) = channel();
        let (exports_tx, exports) = channel();
        Inbox {
            files_tx,
            files,
            exports_tx,
            exports,
        }
    }

    /// Shows the browser's file picker; chosen files arrive in `files`.
    pub fn pick_files(&self, ctx: &egui::Context) {
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Some(files) = rfd::AsyncFileDialog::new()
                .add_filter("Figma file", &["fig"])
                .pick_files()
                .await
            else {
                return;
            };
            for f in files {
                let bytes = f.read().await;
                let _ = tx.send(Ok((PathBuf::from(f.file_name()), bytes)));
                ctx.request_repaint();
            }
        });
    }

    /// Reads a dropped file; it arrives in `files`.
    pub fn read_dropped(&self, ctx: &egui::Context, file: egui::DroppedFileHandle) {
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let path = file.path().to_path_buf();
            let msg = file
                .bytes_async()
                .await
                .map(|bytes| (path.clone(), bytes))
                .map_err(|e| format!("Could not read {}: {e}", path.display()));
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }
}

/// Saves `bytes` as a download named `name`.
pub fn download(name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: JsValue| format!("{e:?}");
    // Copy out of the shared Wasm memory: a Blob cannot be made from a SharedArrayBuffer view.
    let len = u32::try_from(bytes.len()).map_err(|_| "file too large".to_string())?;
    let array = js_sys::Uint8Array::new_with_length(len);
    array.copy_from(bytes);
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(
        &js_sys::Array::of1(&array),
        &options,
    )
    .map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    web_sys::Url::revoke_object_url(&url).map_err(js)
}
