//! Browser build: entry point, and the things the browser does differently from the desktop
//! (picking and dropping files gives `File`s, not paths; exports become downloads).
//!
//! Files are never read whole: an archive is opened from its last bytes, its central
//! directory and the few entries the document needs (`Document::from_lazy`), and images are
//! read as slices when drawn. So a file bigger than the Wasm memory (4 GiB) still opens.
//!
//! JavaScript objects stay on the main thread. Workers ask for byte ranges through
//! [`WebSource`] (a file id and a channel); the main thread reads them with
//! `Blob.slice().arrayBuffer()` and answers through a channel the worker polls. The main
//! thread only copies bytes; workers inflate them.
//!
//! Page setup is in `web/index.html`; build with `scripts/web-build.sh`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use figstudio_fig::archive::{self, Entry};
use figstudio_fig::{Document, FigError, LAZY_ABOVE};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::source::{ImageSource, Source};

/// Exposed to JavaScript as `initThreadPool(n)`; await it before `start`. (The export comes
/// from the crate's `#[wasm_bindgen]`; this binary never calls it, hence the allow.)
#[allow(unused_imports)]
pub use wasm_bindgen_rayon::init_thread_pool;

/// Starts the app on `canvas`. The Rayon pool must already be running: the render loop and file
/// decoding run on it.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    init_tracing();
    let runner = eframe::WebRunner::new();
    // After eframe's hook (installed by `new`). eframe stops drawing after any panic, so the
    // page shows the error itself (`figstudioFatal`, polled by index.html).
    crate::worker::install_panic_flag();
    // Before eframe's own drop listener: a dropped file's handle is only there during the
    // event (Recents keeps it to open the file again).
    crate::recents_web::watch_drops();
    CANVAS.with(|c| *c.borrow_mut() = Some((canvas.clone(), 0.0)));
    runner
        .start(
            canvas,
            // WebGPU first, WebGL2 when the browser lacks it (egui-wgpu's default backends).
            {
                let mut options = eframe::WebOptions {
                    renderer: eframe::Renderer::Wgpu,
                    ..Default::default()
                };
                options.wgpu_options.wgpu_setup = crate::gpu::setup();
                options
            },
            Box::new(|cc| {
                // The system's "reduce motion" setting turns the UI's animations off.
                follow_reduced_motion(&cc.egui_ctx);
                Ok(Box::new(crate::App::new(cc, Vec::new())))
            }),
        )
        .await
}

/// Follows the system's "reduce motion" (`prefers-reduced-motion: reduce`): applied now, and
/// again whenever it changes while the page is open.
fn follow_reduced_motion(ctx: &egui::Context) {
    let Some(query) = web_sys::window().and_then(|w| {
        w.match_media("(prefers-reduced-motion: reduce)")
            .ok()
            .flatten()
    }) else {
        return;
    };
    if query.matches() {
        crate::anim::set_reduced(ctx, true);
    }
    let (ctx, watched) = (ctx.clone(), query.clone());
    let on_change = Closure::<dyn FnMut()>::new(move || {
        crate::anim::set_reduced(&ctx, watched.matches());
        ctx.request_repaint();
    });
    let _ = query.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
    // Lives as long as the page.
    on_change.forget();
}

/// Logs go to the browser console. No ANSI colours and no timestamps (the browser has no
/// clock for them). Default level `warn`. Runs once: later calls do nothing.
fn init_tracing() {
    use tracing_subscriber::prelude::*;
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .without_time()
                .with_writer(tracing_web::MakeWebConsoleWriter::new())
                .with_filter(tracing_subscriber::filter::LevelFilter::WARN),
        )
        .try_init();
}

/// A file the user opened: small ones as bytes, big ones still in the browser.
pub enum Picked {
    /// Name, contents and the pick id of the `File` (see `Uploads`).
    Bytes(PathBuf, Vec<u8>, u32),
    /// Opened lazily: the parts read up front (still compressed), and an id for reading the
    /// rest.
    Lazy {
        name: PathBuf,
        file: u32,
        entries: Vec<Entry>,
        eager: HashMap<String, Vec<u8>>,
        pick: u32,
        doc_key: String,
    },
    Failed(String),
    /// A Recents card's handle did not give its file (`name` as the card shows it).
    Unavailable {
        name: PathBuf,
        handle: u32,
        why: crate::recents_web::Unavailable,
    },
}

type Exported = (String, anyhow::Result<Vec<u8>>);

use crate::cjk::Variant;

const CJK_STORAGE_KEY: &str = "figstudio.cjk_font";

fn cjk_default() -> Variant {
    let Some(window) = web_sys::window() else {
        return Variant::Jp;
    };
    if let Some(variant) = window
        .local_storage()
        .ok()
        .flatten()
        .and_then(|storage| storage.get_item(CJK_STORAGE_KEY).ok().flatten())
        .and_then(|code| Variant::from_code(&code))
    {
        return variant;
    }
    let navigator = window.navigator();
    let language = navigator
        .language()
        .filter(|language| !language.is_empty())
        .or_else(|| navigator.languages().get(0).as_string())
        .unwrap_or_default();
    crate::cjk::from_language(&language)
}

type FontResult = (Variant, anyhow::Result<Vec<u8>>);

/// The selected CJK font, made by `scripts/web-font.sh`, fetched on first need.
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
        reply: Sender<anyhow::Result<Vec<u8>>>,
    },
    /// Nothing uses the file any more (its tab and renders are gone).
    Close(u32),
}

thread_local! {
    /// The app's canvas and the CSS size cap last applied to it (`cap_canvas`).
    static CANVAS: RefCell<Option<(web_sys::HtmlCanvasElement, f32)>> = const { RefCell::new(None) };
}

/// Keeps the canvas backing store within the GPU's `max_side`: eframe sizes it to the CSS box
/// times the device pixel ratio (eframe 0.36 `web/events.rs` `ResizeObserverContext`, and the
/// DPR-change handler) and configures the surface at that size; wgpu panics above the limit.
/// A CSS max-width/height stops the box from growing past the limit; the direct clamp covers
/// a DPR change, where eframe rescales the backing store before the next resize. Called every
/// frame, before eframe paints.
pub fn cap_canvas(max_side: usize) {
    let Some(window) = web_sys::window() else {
        return;
    };
    #[expect(clippy::cast_possible_truncation, reason = "a pixel ratio")]
    let dpr = window.device_pixel_ratio() as f32;
    let max_css = crate::gpu::max_points(max_side, dpr);
    CANVAS.with(|c| {
        let mut c = c.borrow_mut();
        let Some((canvas, applied)) = c.as_mut() else {
            return;
        };
        if *applied != max_css {
            let style = canvas.style();
            let _ = style.set_property("max-width", &format!("{max_css}px"));
            let _ = style.set_property("max-height", &format!("{max_css}px"));
            *applied = max_css;
        }
        let max_px = u32::try_from(max_side).unwrap_or(u32::MAX);
        if canvas.width() > max_px {
            canvas.set_width(max_px);
        }
        if canvas.height() > max_px {
            canvas.set_height(max_px);
        }
    });
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

/// A file picked or dropped here, with the number its handle is kept under where the
/// browser gave one (Recents opens it again from that).
#[derive(Clone)]
struct Upload {
    file: web_sys::File,
    origin: Origin,
}

/// Where a file picked or dropped here came from.
#[derive(Clone, Copy, Debug)]
pub struct Origin {
    /// The number its handle is kept under, where the browser gave one.
    pub handle: Option<u32>,
    /// Picked in the picker a moved Recents card opened: that card's handle.
    pub replaces: Option<u32>,
}

thread_local! {
    /// The `File` behind each open tab, so the MCP bridge can stream it (mcp_upload.rs) and
    /// Recents lists it. It follows its own pick, so of two files of one name the tab gets
    /// the one that loaded.
    static UPLOADS: RefCell<crate::mcp_wire::Uploads<Upload>> =
        RefCell::new(crate::mcp_wire::Uploads::new());
}

/// The file behind tab `tab`, if it was opened here.
pub fn upload_file(tab: u64) -> Option<web_sys::File> {
    UPLOADS.with(|u| u.borrow().get(tab)).map(|u| u.file)
}

/// Where tab `tab`'s file came from, if it was picked or dropped here (a link's or an
/// agent's was not).
pub fn upload_origin(tab: u64) -> Option<Origin> {
    UPLOADS.with(|u| u.borrow().get(tab)).map(|u| u.origin)
}

fn pick_upload(file: &web_sys::File, origin: Origin) -> u32 {
    UPLOADS.with(|u| {
        u.borrow_mut().pick(Upload {
            file: file.clone(),
            origin,
        })
    })
}

/// The load of `name` began with pick `pick`.
pub fn start_upload(name: &std::path::Path, pick: u32) {
    UPLOADS.with(|u| u.borrow_mut().start(name.to_path_buf(), pick));
}

/// Pick `pick` is not loaded (a load of the same name is already running).
pub fn discard_upload(pick: u32) {
    UPLOADS.with(|u| u.borrow_mut().discard(pick));
}

/// The load of `name` finished as tab `tab`, which owns its file from now on.
pub fn bind_upload(name: &std::path::Path, tab: u64) {
    UPLOADS.with(|u| u.borrow_mut().bind(name, tab));
}

/// The load of `name` failed; release its file.
pub fn fail_upload(name: &std::path::Path) {
    UPLOADS.with(|u| u.borrow_mut().fail(name));
}

/// Tab `tab` is closed; release its `File` handle.
pub fn forget_upload(tab: u64) {
    UPLOADS.with(|u| u.borrow_mut().forget(tab));
}

/// One upload slice: `len` bytes at `at` (at most `max`).
pub async fn read_slice(
    file: &web_sys::File,
    at: u64,
    len: u64,
    max: usize,
) -> Result<Vec<u8>, String> {
    read_range_max(file, at, len, max)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Reads `len` bytes at `at`, at most an archive chunk (main thread; async, so the UI keeps
/// running).
async fn read_range(file: &web_sys::File, at: u64, len: u64) -> anyhow::Result<Vec<u8>> {
    read_range_max(file, at, len, figstudio_fig::MAX_CHUNK_BYTES).await
}

/// [`read_range`] with its own size cap; bounds are checked like the desktop reader's.
async fn read_range_max(
    file: &web_sys::File,
    at: u64,
    len: u64,
    max: usize,
) -> anyhow::Result<Vec<u8>> {
    let js = |e: JsValue| anyhow::anyhow!("{}", crate::mcp_wire::js_error_text(&e));
    let (at, end) = archive::checked_range(at, len, max)?;
    let blob = file
        .slice_with_f64_and_f64(at as f64, end as f64)
        .map_err(js)?;
    let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
        .await
        .map_err(js)?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    if bytes.len() as u64 != len {
        anyhow::bail!("file changed or was cut short");
    }
    Ok(bytes)
}

/// An entry's bytes as stored (still compressed): inflating can take long, so workers do it
/// with [`Entry::decode`], never the main thread.
async fn read_raw(file: &web_sys::File, entry: &Entry) -> anyhow::Result<Vec<u8>> {
    let header = read_range(file, entry.header_offset, archive::LOCAL_HEADER_LEN).await?;
    let start = entry.data_start(&header)?;
    read_range(file, start, entry.compressed_size).await
}

/// GETs `url` (same origin) and returns the body.
pub(crate) async fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let js = |e: JsValue| anyhow::anyhow!("{e:?}");
    let window = web_sys::window().ok_or_else(|| anyhow::anyhow!("no window"))?;
    let response: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
    if !response.ok() {
        anyhow::bail!("HTTP {}", response.status());
    }
    let buffer = wasm_bindgen_futures::JsFuture::from(response.array_buffer().map_err(js)?)
        .await
        .map_err(js)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Opens a big archive without reading its images.
async fn open_lazy(file: web_sys::File, origin: Origin) -> Picked {
    let name = PathBuf::from(file.name());
    let fail = |e: anyhow::Error| {
        Picked::Failed(format!(
            "Could not read {}: {}",
            file.name(),
            figstudio_fig::one_line(e.as_ref())
        ))
    };
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
        Err(e) => return fail(e.into()),
    };
    let mut eager = HashMap::new();
    for e in entries
        .iter()
        .filter(|e| archive::EAGER_ENTRIES.contains(&e.name.as_str()))
    {
        match read_raw(&file, e).await {
            Ok(bytes) => {
                eager.insert(e.name.clone(), bytes);
            }
            Err(err) => return fail(err),
        }
    }
    let head_len = len.min(crate::source::DOC_KEY_PREFIX_BYTES as u64);
    let head = match read_range_max(&file, 0, head_len, crate::source::DOC_KEY_PREFIX_BYTES).await {
        Ok(bytes) => bytes,
        Err(err) => return fail(err),
    };
    let doc_key = crate::source::doc_key_from_prefix(len, &head);
    Picked::Lazy {
        name,
        pick: pick_upload(&file, origin),
        file: keep_file(file),
        entries,
        eager,
        doc_key,
    }
}

/// Opens a file the user chose: big archives lazily, the rest by reading them.
async fn open_file(file: web_sys::File, origin: Origin) -> Picked {
    let name = PathBuf::from(file.name());
    if file.size() as u64 > LAZY_ABOVE {
        let mut magic = read_range(&file, 0, 2).await.unwrap_or_default();
        magic.truncate(2);
        if magic == b"PK" {
            return open_lazy(file, origin).await;
        }
    }
    // Same limit as `Document::open` on the desktop.
    match read_range_max(&file, 0, file.size() as u64, figstudio_fig::MAX_FILE_BYTES).await {
        Ok(bytes) => Picked::Bytes(name, bytes, pick_upload(&file, origin)),
        Err(e) => Picked::Failed(format!(
            "Could not read {}: {}",
            name.display(),
            figstudio_fig::one_line(e.as_ref())
        )),
    }
}

/// Shows the picker and opens what is chosen. The File System Access picker where the
/// browser has it (each file with its handle), else the plain one (no handles). `moved`: the
/// handle of a moved Recents card; the picker starts in its folder, and a file picked there
/// with the browser's picker takes the card's place (one from the plain input, which has no
/// handle, is listed beside it).
async fn pick(tx: Sender<Picked>, ctx: egui::Context, moved: Option<u32>) {
    if crate::recents_web::keeps_handles() {
        match crate::recents_web::pick_files(moved).await {
            Ok(Some(files)) => {
                for (file, handle) in files {
                    let origin = Origin {
                        handle: Some(handle),
                        replaces: moved,
                    };
                    let _ = tx.send(open_file(file, origin).await);
                    ctx.request_repaint();
                }
                return;
            }
            Ok(None) => return,
            // Refused (no click to go with it, or an embedding page): the plain picker.
            Err(e) => tracing::warn!("file picker: {e}"),
        }
    }
    let Some(files) = rfd::AsyncFileDialog::new()
        .add_filter("Figma file", &["fig"])
        .pick_files()
        .await
    else {
        return;
    };
    for f in files {
        // No handle to take the moved card's place (`Recents::replace_moved`).
        let origin = Origin {
            handle: None,
            replaces: None,
        };
        let _ = tx.send(open_file(f.inner().clone(), origin).await);
        ctx.request_repaint();
    }
}

/// Reads images of a lazily opened browser file for workers. The file is released when the
/// last copy is dropped.
pub struct WebSource {
    file: u32,
    requests: Sender<FileRequest>,
}

impl std::fmt::Debug for WebSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSource")
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

impl Drop for WebSource {
    fn drop(&mut self) {
        let _ = self.requests.send(FileRequest::Close(self.file));
    }
}

impl ImageSource for WebSource {
    fn read(&self, entry: &Entry) -> anyhow::Result<Vec<u8>> {
        let (reply, answer) = channel();
        self.requests
            .send(FileRequest::Read {
                file: self.file,
                entry: entry.clone(),
                reply,
            })
            .map_err(|_| anyhow::anyhow!("app closed"))?;
        // Workers may wait; they poll rather than block so a one-thread pool keeps running.
        // The nap doubles from 2 ms to 16 ms so a slow read does not spin.
        let mut nap = 2;
        loop {
            match answer.try_recv() {
                Ok(result) => {
                    return Ok(entry.decode(result?)?);
                }
                Err(TryRecvError::Empty) => {
                    if rayon::yield_now() == Some(rayon::Yield::Executed) {
                        nap = 2;
                    } else {
                        std::thread::sleep(web_time::Duration::from_millis(nap));
                        nap = (nap * 2).min(16);
                    }
                }
                Err(TryRecvError::Disconnected) => return Err(anyhow::anyhow!("read cancelled")),
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
    fonts_tx: Sender<FontResult>,
    pub fonts: Receiver<FontResult>,
    pub font_state: FontState,
    /// The View menu choice (saved, or the browser language default).
    pub font_choice: Variant,
    /// Why the face is KR (for Hangul) or `font_choice`, and what the Recents names asked.
    pub face: crate::cjk::Face,
    pub symbols: crate::symbols::Loader,
}

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inbox").finish_non_exhaustive()
    }
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
            font_choice: cjk_default(),
            face: crate::cjk::Face::default(),
            symbols: crate::symbols::Loader::default(),
        }
    }

    /// Fetches the CJK font that an opened file's `need` asks for (KR for Hangul only).
    /// Otherwise it is the saved face, also when only Recents names had switched to KR.
    pub fn load_cjk_for(
        &mut self,
        ctx: &egui::Context,
        need: crate::theme::CjkNeed,
        path: &std::path::Path,
    ) {
        use crate::theme::CjkNeed;
        if need == CjkNeed::None {
            return;
        }
        let hangul_only = need == CjkNeed::HangulOnly;
        self.switch(ctx, |inbox| inbox.face.document(hangul_only, path));
        self.load_cjk_font(ctx);
    }

    /// Fetches the CJK font for the Recents page's names (see [`crate::cjk::Face::recents`]).
    pub fn load_cjk_for_labels<'a>(
        &mut self,
        ctx: &egui::Context,
        labels: &[String],
        open: impl IntoIterator<Item = &'a std::path::Path>,
    ) {
        let need = crate::theme::cjk_need_of(labels.iter().map(String::as_str));
        let mut fetch = false;
        self.switch(ctx, |inbox| fetch = inbox.face.recents(need, open));
        if fetch {
            self.load_cjk_font(ctx);
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
        let variant = self.font_variant();
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("fonts/cjk-{}.otf", variant.code());
            let _ = tx.send((variant, fetch_bytes(&url).await));
            ctx.request_repaint();
        });
    }

    /// The face in use: KR while a Hangul-only file or Recents name asked for it, else the
    /// menu choice.
    pub fn font_variant(&self) -> Variant {
        if self.face.korean() {
            Variant::Kr
        } else {
            self.font_choice
        }
    }

    /// A View menu pick: saved, and it ends the session's KR override.
    pub fn select_cjk_font(&mut self, variant: Variant, ctx: &egui::Context) {
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item(CJK_STORAGE_KEY, variant.code());
        }
        self.switch(ctx, |inbox| {
            inbox.font_choice = variant;
            inbox.face.pick();
        });
    }

    /// Applies `change`; if that changes the face in use, a loaded or loading font is
    /// fetched again (`poll_web` drops the result of the old one).
    fn switch(&mut self, ctx: &egui::Context, change: impl FnOnce(&mut Self)) {
        let before = self.font_variant();
        change(self);
        if self.font_variant() != before && self.font_state != FontState::NotNeeded {
            self.font_state = FontState::NotNeeded;
            self.load_cjk_font(ctx);
        }
    }

    /// Shows the browser's file picker; chosen files arrive in `files`. Where the browser
    /// gives file handles it is the File System Access picker, so Recents can open the files
    /// again.
    pub fn pick_files(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.files_tx.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(pick(tx, ctx, None));
    }

    /// Opens the file behind a Recents card's handle (asking to read it, if needed). When
    /// it was moved or deleted, the picker starts in its folder.
    pub fn open_handle(&self, ctx: &egui::Context, handle: u32, name: PathBuf) {
        let (tx, ctx) = (self.files_tx.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            match crate::recents_web::open_handle(handle).await {
                Ok(file) => {
                    let origin = Origin {
                        handle: Some(handle),
                        replaces: None,
                    };
                    let _ = tx.send(open_file(file, origin).await);
                    ctx.request_repaint();
                }
                Err(why) => {
                    let _ = tx.send(Picked::Unavailable { name, handle, why });
                    ctx.request_repaint();
                    if why == crate::recents_web::Unavailable::Missing {
                        pick(tx, ctx, Some(handle)).await;
                    }
                }
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
            let handle = crate::recents_web::dropped_handle(&file).await;
            let origin = Origin {
                handle,
                replaces: None,
            };
            let _ = tx.send(open_file(file, origin).await);
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
                    Some(f) => read_raw(&f, &entry).await,
                    None => Err(anyhow::anyhow!("file closed")),
                };
                let _ = reply.send(result);
                ctx.request_repaint();
            });
        }
    }
}

/// Opens a lazily picked file on a worker: inflates the eager entries (read raw by
/// `open_lazy`), then decodes the canvas; both take a while.
pub fn decode_lazy(
    entries: Vec<Entry>,
    raw: HashMap<String, Vec<u8>>,
) -> Result<Document, FigError> {
    let eager = archive::decode_raw(&entries, raw)?;
    Document::from_lazy(entries, eager)
}

/// `figstudioFatal()` from JavaScript: a panic happened on some thread. The app is stopped
/// and only a reload helps.
#[wasm_bindgen(js_name = figstudioFatal)]
pub fn fatal() -> bool {
    crate::worker::panicked()
}

static PAGE_REQUEST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// `figstudioPage(n)` from JavaScript: show page `n` (0-based) of the active tab.
/// Normal pages precede internal pages, matching `figstudioStatus.page` and `.pages`.
/// This is for automated checks; people use the Pages panel.
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
pub fn download(name: &str, mime: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let js = |e: JsValue| anyhow::anyhow!("{e:?}");
    // Copy out of the shared Wasm memory: a Blob cannot be made from a SharedArrayBuffer view.
    let len = u32::try_from(bytes.len()).map_err(|_| anyhow::anyhow!("file too large"))?;
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
        .ok_or_else(|| anyhow::anyhow!("no document"))?;
    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(js)?
        .dyn_into()
        .map_err(|_| anyhow::anyhow!("not an anchor"))?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    web_sys::Url::revoke_object_url(&url).map_err(js)
}
