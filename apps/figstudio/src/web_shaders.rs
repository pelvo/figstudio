//! Shader fills and effects in the browser. A shader's module (types stripped here, see
//! `figstudio_shader::browser_module`) runs on the page's WebGPU inside a sandboxed frame
//! (`web_shaders.js`: an opaque origin with no network and no storage; a run that takes too
//! long is stopped by replacing the frame). Runs are asynchronous: a tile drawn before its
//! shader finishes counts it as loading (`Shaded::Pending`) and is drawn again when the
//! result arrives, as with images. The UI thread never takes a lock here (a wait there traps
//! in the browser): jobs and results travel on channels, workers lock their cache.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use figstudio_fig::{Document, Val};
use figstudio_mcp_core::shaders::ShaderSource;
use figstudio_render::shader::{Shaded, ShaderFills};
use serde_json::Value;
use vello_cpu::Pixmap;
use wasm_bindgen::JsCast;

/// The sandbox bridge, run once in the page.
const BRIDGE: &str = include_str!("web_shaders.js");
/// Longest a shader may run before its sandbox is replaced.
const TIMEOUT_MS: u32 = 10_000;
/// Most outputs one file's host keeps.
const CACHE: usize = 64;

enum Entry {
    Pending,
    Done(Option<Arc<Pixmap>>),
}

/// A finished run for a host's cache.
type Finished = (u64, Option<Arc<Pixmap>>);

/// A run waiting for the main thread.
struct Job {
    /// Where its result goes (its host's channel).
    done: Sender<Finished>,
    key: u64,
    js: String,
    params: Value,
    width: u16,
    height: u16,
    time_ms: f64,
    /// An effect's input (premultiplied RGBA8, `width × height`).
    input: Option<Vec<u8>>,
}

/// Starts the browser's shader runs (on the main thread) and makes each file's host.
pub struct Runner {
    jobs_tx: Sender<Job>,
    jobs: Receiver<Job>,
    arrived: Arc<AtomicBool>,
    installed: bool,
}

impl Default for Runner {
    fn default() -> Self {
        let (jobs_tx, jobs) = channel();
        Runner {
            jobs_tx,
            jobs,
            arrived: Arc::default(),
            installed: false,
        }
    }
}

impl Runner {
    /// A host drawing a file's shaders from `sources`.
    pub fn host(&self, sources: Vec<ShaderSource>) -> Arc<dyn ShaderFills> {
        let (done, results) = channel();
        Arc::new(WebFills {
            sources,
            cache: Mutex::default(),
            results: Mutex::new(results),
            done,
            jobs: self.jobs_tx.clone(),
        })
    }

    /// Starts the runs workers asked for. True when results arrived since the last call
    /// (the caller redraws the tiles waiting for them). Call every frame.
    pub fn pump(&mut self, ctx: &egui::Context) -> bool {
        let jobs: Vec<Job> = self.jobs.try_iter().collect();
        if !jobs.is_empty() && !self.installed {
            if let Err(e) = js_sys::eval(BRIDGE) {
                tracing::warn!("shader sandbox not installed: {e:?}");
            }
            self.installed = true;
        }
        for job in jobs {
            let (arrived, ctx) = (self.arrived.clone(), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let (done, key) = (job.done.clone(), job.key);
                let out = run(job)
                    .await
                    .map_err(|e| tracing::warn!("shader not drawn: {e}"))
                    .ok();
                // A host replaced meanwhile no longer listens.
                let _ = done.send((key, out));
                arrived.store(true, Ordering::Release);
                ctx.request_repaint();
            });
        }
        self.arrived.swap(false, Ordering::AcqRel)
    }
}

/// Runs `job` in the sandbox and returns its output.
async fn run(job: Job) -> Result<Arc<Pixmap>, String> {
    let bridge = js_sys::Reflect::get(&js_sys::global(), &"__figstudioShaders".into())
        .map_err(|_| "no shader sandbox".to_owned())?;
    let call: js_sys::Function = js_sys::Reflect::get(&bridge, &"run".into())
        .ok()
        .and_then(|f| f.dyn_into().ok())
        .ok_or("no shader sandbox")?;
    let params = js_sys::JSON::parse(&job.params.to_string())
        .map_err(|_| "shader parameters are not JSON".to_owned())?;
    let input = job
        .input
        .as_deref()
        .map_or(wasm_bindgen::JsValue::NULL, |bytes| {
            js_sys::Uint8Array::from(bytes).into()
        });
    let args = js_sys::Array::new();
    for arg in [
        job.js.as_str().into(),
        params,
        f64::from(job.width).into(),
        f64::from(job.height).into(),
        job.time_ms.into(),
        input,
        f64::from(TIMEOUT_MS).into(),
    ] {
        args.push(&arg);
    }
    let promise: js_sys::Promise = call
        .apply(&bridge, &args)
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "the sandbox did not answer with a promise".to_owned())?;
    let pixels = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| {
            js_sys::Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
                .unwrap_or_else(|| format!("{e:?}"))
        })?;
    let bytes = js_sys::Uint8Array::new(&pixels).to_vec();
    let expected = usize::from(job.width) * usize::from(job.height) * 4;
    if bytes.len() != expected {
        return Err(format!(
            "the shader gave {} bytes, not {expected}",
            bytes.len()
        ));
    }
    Ok(Arc::new(figstudio_shader::pixmap(
        bytes,
        job.width,
        job.height,
        job.input.is_some(),
    )))
}

/// One file's shader host in the browser.
struct WebFills {
    sources: Vec<ShaderSource>,
    cache: Mutex<HashMap<u64, Entry>>,
    /// Finished runs, taken into the cache by the next lookup.
    results: Mutex<Receiver<Finished>>,
    done: Sender<Finished>,
    jobs: Sender<Job>,
}

impl WebFills {
    /// The output of `key` if it is done, else queues `job` (once) and says it is pending.
    fn lookup(&self, key: u64, job: impl FnOnce() -> Result<Job, String>) -> Shaded {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        for (key, out) in self
            .results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_iter()
        {
            cache.insert(key, Entry::Done(out));
        }
        match cache.get(&key) {
            Some(Entry::Pending) => return Shaded::Pending,
            Some(Entry::Done(out)) => return Shaded::from_option(out.clone()),
            None => {}
        }
        if cache.len() >= CACHE {
            cache.retain(|_, e| matches!(e, Entry::Pending));
        }
        match job() {
            Ok(job) => {
                cache.insert(key, Entry::Pending);
                let _ = self.jobs.send(job);
                crate::web_wake::wake();
                Shaded::Pending
            }
            Err(e) => {
                tracing::warn!("shader not drawn: {e}");
                cache.insert(key, Entry::Done(None));
                Shaded::Unavailable
            }
        }
    }
}

impl ShaderFills for WebFills {
    fn fill(&self, doc: &Document, paint: &Val, width: u16, height: u16, time_ms: f64) -> Shaded {
        let Some(p) = figstudio_shader::program(&self.sources, doc, paint, time_ms) else {
            return Shaded::Unavailable;
        };
        self.lookup(p.key(width, height, None), || {
            Ok(Job {
                done: self.done.clone(),
                key: p.key(width, height, None),
                js: figstudio_shader::browser_module(p.main).map_err(|e| e.to_string())?,
                params: p.params.clone(),
                width,
                height,
                time_ms: p.time_ms,
                input: None,
            })
        })
    }

    fn has_effect(&self, doc: &Document, effect: &Val) -> bool {
        figstudio_shader::program(&self.sources, doc, effect, 0.0).is_some()
    }

    fn effect(&self, doc: &Document, effect: &Val, input: &Pixmap, time_ms: f64) -> Shaded {
        let Some(p) = figstudio_shader::program(&self.sources, doc, effect, time_ms) else {
            return Shaded::Unavailable;
        };
        let key = p.key(input.width(), input.height(), Some(input));
        self.lookup(key, || {
            Ok(Job {
                done: self.done.clone(),
                key,
                js: figstudio_shader::browser_module(p.main).map_err(|e| e.to_string())?,
                params: p.params.clone(),
                width: input.width(),
                height: input.height(),
                time_ms: p.time_ms,
                input: Some(input.data_as_u8_slice().to_vec()),
            })
        })
    }
}
