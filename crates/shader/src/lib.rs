//! Draws Figma code shaders: the shader's `main.ts` (from the file's shader folder, see
//! `figstudio_mcp_core::shader_sources`) is stripped of its types (oxc), run in Boa, and its
//! WebGPU calls go to a headless wgpu device. [`Fills`] is the renderer's
//! [`ShaderFills`](figstudio_render::shader::ShaderFills) host.

mod gpu;
mod js;
mod ts;

use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};

use figstudio_fig::{Document, Val};
use figstudio_mcp_core::shaders::{self, ShaderSource};
use figstudio_render::shader::ShaderFills;
use serde_json::Value;
use vello_cpu::PixelMetadata;
use vello_cpu::Pixmap;
use vello_cpu::peniko::ImageAlphaType;

/// Why a shader did not draw.
#[derive(Debug)]
pub struct ShaderError {
    kind: Kind,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    NoGpu,
    Script,
    Gpu,
}

impl ShaderError {
    fn no_gpu() -> Self {
        ShaderError {
            kind: Kind::NoGpu,
            message: "no GPU adapter on this machine".into(),
        }
    }

    fn script(message: impl Into<String>) -> Self {
        ShaderError {
            kind: Kind::Script,
            message: message.into(),
        }
    }

    fn gpu(message: impl Into<String>) -> Self {
        ShaderError {
            kind: Kind::Gpu,
            message: message.into(),
        }
    }

    /// This machine has no GPU adapter (headless CI, for example).
    pub fn is_no_gpu(&self) -> bool {
        self.kind == Kind::NoGpu
    }

    /// The shader's code failed: it does not compile, throws, or uses a WebGPU call this
    /// runtime does not have.
    pub fn is_script(&self) -> bool {
        self.kind == Kind::Script
    }
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ShaderError {}

/// Runs a shader fill's `main.ts` with `params` (its `frame.params`) into a `width × height`
/// output at `time_ms` (ms on the playback clock). Fills draw straight alpha.
pub fn render(
    main_ts: &str,
    params: &Value,
    width: u16,
    height: u16,
    time_ms: f64,
) -> Result<Pixmap, ShaderError> {
    run(main_ts, params, width.max(1), height.max(1), time_ms, None)
}

/// Runs a shader effect's `main.ts` on `input` (the layer drawn without the effect,
/// premultiplied) and returns its output, the same size. Effects draw premultiplied alpha.
pub fn render_effect(
    main_ts: &str,
    params: &Value,
    input: &Pixmap,
    time_ms: f64,
) -> Result<Pixmap, ShaderError> {
    run(
        main_ts,
        params,
        input.width().max(1),
        input.height().max(1),
        time_ms,
        Some(input.data_as_u8_slice()),
    )
}

fn run(
    main_ts: &str,
    params: &Value,
    width: u16,
    height: u16,
    time_ms: f64,
    input: Option<&[u8]>,
) -> Result<Pixmap, ShaderError> {
    let js = ts::to_js(main_ts)?;
    let frame = js::Frame {
        width,
        height,
        time_ms,
        input,
    };
    let rgba = js::run(&js, params, &frame)?;
    let meta = PixelMetadata {
        may_have_transparency: true,
        alpha_type: if input.is_some() {
            ImageAlphaType::AlphaPremultiplied
        } else {
            ImageAlphaType::Alpha
        },
    };
    Ok(Pixmap::from_parts(rgba, width, height, meta))
}

/// Most shader outputs [`Fills`] keeps.
const CACHE: usize = 64;

/// Draws a document's shader fills from the shader sources next to it.
pub struct Fills {
    sources: Vec<ShaderSource>,
    /// Outputs (or failures) by shader, parameters and size. Runs hold the lock: one shader
    /// at a time, and parallel strips wanting the same fill wait for the first run.
    cache: Mutex<HashMap<u64, Option<Arc<Pixmap>>>>,
}

impl fmt::Debug for Fills {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fills")
            .field("sources", &self.sources.len())
            .finish_non_exhaustive()
    }
}

impl Fills {
    pub fn new(sources: Vec<ShaderSource>) -> Self {
        Fills {
            sources,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The fills for the `.fig` at `path`, `None` when it has no shader folder.
    pub fn for_file(path: &Path) -> Option<Arc<Self>> {
        let sources = figstudio_mcp_core::shader_sources::load(path);
        (!sources.is_empty()).then(|| Arc::new(Fills::new(sources)))
    }
}

impl Fills {
    /// The cached result of `key`, or `run`'s, kept.
    fn cached(
        &self,
        key: u64,
        run: impl FnOnce() -> Result<Pixmap, ShaderError>,
    ) -> Option<Arc<Pixmap>> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = cache.get(&key) {
            return done.clone();
        }
        let out = match run() {
            Ok(pixmap) => Some(Arc::new(pixmap)),
            Err(e) => {
                tracing::warn!("shader not drawn: {e}");
                None
            }
        };
        if cache.len() >= CACHE {
            cache.clear();
        }
        cache.insert(key, out.clone());
        out
    }

    /// `main.ts` of the shader `paint` (a fill or effect) uses, with its `frame.params`.
    fn program(&self, doc: &Document, paint: &Val) -> Option<(&ShaderSource, &str, Value)> {
        let (source, params) = shaders::program(doc, paint, &self.sources)?;
        let (_, main) = source
            .files
            .iter()
            .find(|(f, _)| f.rsplit('/').next() == Some("main.ts"))?;
        Some((source, main.as_str(), params))
    }
}

/// The shader clock a run sees: `time_ms` for an animated shader, 0 for a still one (so its
/// output is cached across frames).
fn clock(source: &ShaderSource, time_ms: f64) -> f64 {
    let animated = source
        .files
        .iter()
        .find(|(f, _)| f.rsplit('/').next() == Some("features.json"))
        .and_then(|(_, t)| serde_json::from_str::<Value>(t).ok())
        .and_then(|f| f["isAnimated"].as_bool())
        .unwrap_or(false);
    if animated { time_ms } else { 0.0 }
}

impl ShaderFills for Fills {
    fn fill(
        &self,
        doc: &Document,
        paint: &Val,
        width: u16,
        height: u16,
        time_ms: f64,
    ) -> Option<Arc<Pixmap>> {
        let (source, main, params) = self.program(doc, paint)?;
        let time = clock(source, time_ms);
        let mut hasher = DefaultHasher::new();
        (
            &source.label,
            params.to_string(),
            width,
            height,
            time.to_bits(),
        )
            .hash(&mut hasher);
        self.cached(hasher.finish(), || {
            render(main, &params, width, height, time)
        })
    }

    fn has_effect(&self, doc: &Document, effect: &Val) -> bool {
        self.program(doc, effect).is_some()
    }

    fn effect(
        &self,
        doc: &Document,
        effect: &Val,
        input: &Pixmap,
        time_ms: f64,
    ) -> Option<Arc<Pixmap>> {
        let (source, main, params) = self.program(doc, effect)?;
        let time = clock(source, time_ms);
        let mut hasher = DefaultHasher::new();
        (&source.label, params.to_string(), time.to_bits()).hash(&mut hasher);
        (input.width(), input.height(), input.data_as_u8_slice()).hash(&mut hasher);
        self.cached(hasher.finish(), || {
            render_effect(main, &params, input, time)
        })
    }
}
