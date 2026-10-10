//! Browser build: entry point, and the things the browser does differently from the desktop
//! (picking and dropping files gives `File`s, not paths; exports become downloads).
//!
//! Files are never read whole: an archive is opened from its last bytes, its central
//! directory and the few entries the document needs (`Document::from_lazy`), and images are
//! read as slices when drawn. So a file bigger than the Wasm memory (4 GiB) still opens.
//!
//! JavaScript objects stay on the main thread. Workers ask for byte ranges through
//! [`WebSource`] (a file id and a channel); the main thread reads them with
//! `Blob.slice().arrayBuffer()` and answers through a channel the worker polls.
//!
//! Page setup is in `web/index.html`; build with `scripts/web-build.sh`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use figstudio_fig::archive::{self, Entry};
use figstudio_fig::{Document, FigError};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::source::{ImageSource, LAZY_ABOVE, Source};

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

/// A file the user opened: small ones as bytes, big ones still in the browser.
pub enum Picked {
    Bytes(PathBuf, Vec<u8>),
    /// Opened lazily: the parts read up front, and an id for reading the rest.
    Lazy {
        name: PathBuf,
        file: u32,
        entries: Vec<Entry>,
        eager: HashMap<String, Vec<u8>>,
    },
    Failed(String),
}

type Exported = (String, Result<Vec<u8>, String>);

/// The CJK font (`fonts/cjk.otf`, made by `scripts/web-font.sh`), fetched on first need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontState {
    NotNeeded,
    Loading,
    Loaded,
    Failed,
}

/// What workers ask the main thread for.
enum FileRequest {
    /// An archive entry of an open file, and where to send its bytes.
    Read {
        file: u32,
        entry: Entry,
        reply: Sender<Result<Vec<u8>, String>>,
    },
    /// Nothing uses the file any more (its tab and renders are gone).
    Close(u32),
}

thread_local! {
    /// Open files by id (main thread only; `File` cannot cross threads).
    static FILES: RefCell<(u32, HashMap<u32, web_sys::File>)> = RefCell::new((0, HashMap::new()));
}

fn keep_file(file: web_sys::File) -> u32 {
    FILES.with(|f| {
        let mut f = f.borrow_mut();
        f.0 += 1;
        let id = f.0;
        f.1.insert(id, file);
        id
    })
}

fn file(id: u32) -> Option<web_sys::File> {
    FILES.with(|f| f.borrow().1.get(&id).cloned())
}

fn drop_file(id: u32) {
    FILES.with(|f| f.borrow_mut().1.remove(&id));
}

/// Reads `len` bytes at `at` (main thread; async, so the UI keeps running).
async fn read_range(file: &web_sys::File, at: u64, len: u64) -> Result<Vec<u8>, String> {
    let js = |e: JsValue| format!("{e:?}");
    let blob = file
        .slice_with_f64_and_f64(at as f64, (at + len) as f64)
        .map_err(js)?;
    let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
        .await
        .map_err(js)?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    if bytes.len() as u64 != len {
        return Err("file changed or was cut short".into());
    }
    Ok(bytes)
}

async fn read_entry(file: &web_sys::File, entry: &Entry) -> Result<Vec<u8>, String> {
    let header = read_range(file, entry.header_offset, archive::LOCAL_HEADER_LEN).await?;
    let start = entry.data_start(&header).map_err(|e| e.0)?;
    let data = read_range(file, start, entry.compressed_size).await?;
    entry.decode(data).map_err(|e| e.0)
}

/// GETs `url` (same origin) and returns the body.
async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let js = |e: JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let response: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let buffer = wasm_bindgen_futures::JsFuture::from(response.array_buffer().map_err(js)?)
        .await
        .map_err(js)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Opens a big archive without reading its images.
async fn open_lazy(file: web_sys::File) -> Picked {
    let name = PathBuf::from(file.name());
    let fail = |e: String| Picked::Failed(format!("Could not read {}: {e}", file.name()));
    let len = file.size() as u64;
    let tail_len = archive::TAIL_LEN.min(len);
    let tail = match read_range(&file, len - tail_len, tail_len).await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let entries = match archive::central_directory(&tail, len) {
        Ok((at, size)) => match read_range(&file, at, size).await {
            Ok(cd) => archive::entries(&cd),
            Err(e) => return fail(e),
        },
        Err(e) => Err(e),
    };
    let entries = match entries {
        Ok(e) => e,
        Err(FigError(e)) => return fail(e),
    };
    let mut eager = HashMap::new();
    for e in entries
        .iter()
        .filter(|e| archive::EAGER_ENTRIES.contains(&e.name.as_str()))
    {
        match read_entry(&file, e).await {
            Ok(bytes) => {
                eager.insert(e.name.clone(), bytes);
            }
            Err(err) => return fail(err),
        }
    }
    Picked::Lazy {
        name,
        file: keep_file(file),
        entries,
        eager,
    }
}

/// Opens a file the user chose: big archives lazily, the rest by reading them.
async fn open_file(file: web_sys::File) -> Picked {
    let name = PathBuf::from(file.name());
    if file.size() as u64 > LAZY_ABOVE {
        let mut magic = read_range(&file, 0, 2).await.unwrap_or_default();
        magic.truncate(2);
        if magic == b"PK" {
            return open_lazy(file).await;
        }
    }
    match read_range(&file, 0, file.size() as u64).await {
        Ok(bytes) => Picked::Bytes(name, bytes),
        Err(e) => Picked::Failed(format!("Could not read {}: {e}", name.display())),
    }
}

/// Reads images of a lazily opened browser file for workers. The file is released when the
/// last copy is dropped.
pub struct WebSource {
    file: u32,
    requests: Sender<FileRequest>,
}

impl Drop for WebSource {
    fn drop(&mut self) {
        let _ = self.requests.send(FileRequest::Close(self.file));
    }
}

impl ImageSource for WebSource {
    fn read(&self, entry: &Entry) -> Result<Vec<u8>, String> {
        let (reply, answer) = channel();
        self.requests
            .send(FileRequest::Read {
                file: self.file,
                entry: entry.clone(),
                reply,
            })
            .map_err(|_| "app closed".to_string())?;
        // Workers may wait; they poll rather than block so a one-thread pool keeps running.
        loop {
            match answer.try_recv() {
                Ok(result) => return result,
                Err(TryRecvError::Empty) => {
                    if rayon::yield_now() != Some(rayon::Yield::Executed) {
                        std::thread::sleep(web_time::Duration::from_millis(2));
                    }
                }
                Err(TryRecvError::Disconnected) => return Err("read cancelled".into()),
            }
        }
    }
}

/// Results that arrive asynchronously; `App::poll` drains them.
pub struct Inbox {
    files_tx: Sender<Picked>,
    pub files: Receiver<Picked>,
    pub exports_tx: Sender<Exported>,
    pub exports: Receiver<Exported>,
    ranges_tx: Sender<FileRequest>,
    ranges: Receiver<FileRequest>,
    fonts_tx: Sender<Result<Vec<u8>, String>>,
    pub fonts: Receiver<Result<Vec<u8>, String>>,
    pub font_state: FontState,
}

impl Inbox {
    pub fn new() -> Self {
        let (files_tx, files) = channel();
        let (exports_tx, exports) = channel();
        let (ranges_tx, ranges) = channel();
        let (fonts_tx, fonts) = channel();
        Inbox {
            files_tx,
            files,
            exports_tx,
            exports,
            ranges_tx,
            ranges,
            fonts_tx,
            fonts,
            font_state: FontState::NotNeeded,
        }
    }

    /// Fetches the CJK font once; it arrives in `fonts`.
    pub fn load_cjk_font(&mut self, ctx: &egui::Context) {
        if self.font_state != FontState::NotNeeded {
            return;
        }
        self.font_state = FontState::Loading;
        let tx = self.fonts_tx.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(fetch_bytes("fonts/cjk.otf").await);
            ctx.request_repaint();
        });
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
                let _ = tx.send(open_file(f.inner().clone()).await);
                ctx.request_repaint();
            }
        });
    }

    /// Opens a dropped file; it arrives in `files`.
    pub fn read_dropped(&self, ctx: &egui::Context, dropped: egui::DroppedFileHandle) {
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        let Some(file) = dropped.web_file().cloned() else {
            let _ = tx.send(Picked::Failed("dropped item is not a file".into()));
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(open_file(file).await);
            ctx.request_repaint();
        });
    }

    /// The source workers read a lazily opened file's images from.
    pub fn source(&self, file: u32) -> Source {
        std::sync::Arc::new(WebSource {
            file,
            requests: self.ranges_tx.clone(),
        })
    }

    /// Starts reading the byte ranges workers asked for (call every frame).
    pub fn serve_ranges(&self, ctx: &egui::Context) {
        while let Ok(req) = self.ranges.try_recv() {
            let (id, entry, reply) = match req {
                FileRequest::Close(id) => {
                    drop_file(id);
                    continue;
                }
                FileRequest::Read { file, entry, reply } => (file, entry, reply),
            };
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = match file(id) {
                    Some(f) => read_entry(&f, &entry).await,
                    None => Err("file closed".into()),
                };
                let _ = reply.send(result);
                ctx.request_repaint();
            });
        }
    }
}

/// Opens a lazily picked file on a worker (decoding the canvas takes a while).
pub fn decode_lazy(
    entries: Vec<Entry>,
    eager: HashMap<String, Vec<u8>>,
) -> Result<Document, FigError> {
    Document::from_lazy(entries, eager)
}

static PAGE_REQUEST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// `figstudioPage(n)` from JavaScript: show page `n` (0-based) of the active tab (for
/// automated checks; people use the Pages panel).
#[wasm_bindgen(js_name = figstudioPage)]
pub fn request_page(n: i32) {
    PAGE_REQUEST.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// A page index asked for with `figstudioPage`, once.
pub fn take_page_request() -> Option<usize> {
    usize::try_from(PAGE_REQUEST.swap(-1, std::sync::atomic::Ordering::Relaxed)).ok()
}

/// Sets `window.figstudioStatus` to the latest status (numbers only), plus the Wasm memory
/// size, for automated checks.
pub fn publish_status(json: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let memory = wasm_bindgen::memory()
        .dyn_into::<js_sys::WebAssembly::Memory>()
        .ok()
        .and_then(|m| m.buffer().dyn_into::<js_sys::SharedArrayBuffer>().ok())
        .map_or(0.0, |b| f64::from(b.byte_length()));
    let json = format!(
        "{},\"wasm_memory_bytes\":{memory}}}",
        json.trim_end_matches('}')
    );
    let _ = js_sys::Reflect::set(&window, &"figstudioStatus".into(), &json.into());
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
