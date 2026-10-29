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

/// Runs a shader's `main.ts` with `params` (its `frame.params`) into a `width × height`
/// output at `time_ms`; the pixels as a fill's straight alpha, premultiplied in the pixmap.
pub fn render(
    main_ts: &str,
    params: &Value,
    width: u16,
    height: u16,
    time_ms: f64,
) -> Result<Pixmap, ShaderError> {
    let js = ts::to_js(main_ts)?;
    let (width, height) = (width.max(1), height.max(1));
    let rgba = js::run(&js, params, width, height, time_ms)?;
    let meta = PixelMetadata {
        may_have_transparency: true,
        alpha_type: ImageAlphaType::Alpha,
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

impl ShaderFills for Fills {
    fn fill(&self, doc: &Document, paint: &Val, width: u16, height: u16) -> Option<Arc<Pixmap>> {
        let (source, params) = shaders::program(doc, paint, &self.sources)?;
        let (_, main) = source
            .files
            .iter()
            .find(|(f, _)| f.rsplit('/').next() == Some("main.ts"))?;
        let mut hasher = DefaultHasher::new();
        (&source.label, params.to_string(), width, height).hash(&mut hasher);
        let key = hasher.finish();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = cache.get(&key) {
            return done.clone();
        }
        let out = match render(main, &params, width, height, 0.0) {
            Ok(pixmap) => Some(Arc::new(pixmap)),
            Err(e) => {
                tracing::warn!("shader fill not drawn: {e}");
                None
            }
        };
        if cache.len() >= CACHE {
            cache.clear();
        }
        cache.insert(key, out.clone());
        out
    }
}
