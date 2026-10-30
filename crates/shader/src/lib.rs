//! Draws Figma code shaders: the shader's `main.ts` (from the file's shader folder, see
//! `figstudio_mcp_core::shader_sources`) is stripped of its types (oxc), run in Boa, and its
//! WebGPU calls go to a headless wgpu device. [`Fills`] is the renderer's
//! [`ShaderFills`](figstudio_render::shader::ShaderFills) host.
//!
//! Without the `native` feature (the browser build) only the shared part is built: the
//! TypeScript step ([`browser_module`]), the program a paint runs ([`program`]) and its cache
//! key; the browser runs the module on its own WebGPU.

#[cfg(feature = "native")]
mod gpu;
#[cfg(feature = "native")]
mod js;
mod ts;

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use figstudio_fig::{Document, Val};
use figstudio_mcp_core::shaders::{self, ShaderSource};
use serde_json::Value;
use vello_cpu::PixelMetadata;
use vello_cpu::Pixmap;
use vello_cpu::peniko::ImageAlphaType;

#[cfg(feature = "native")]
pub use native::{Fills, render, render_effect};

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
    #[cfg_attr(
        not(feature = "native"),
        expect(dead_code, reason = "native runtime only")
    )]
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

    #[cfg_attr(
        not(feature = "native"),
        expect(dead_code, reason = "native runtime only")
    )]
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

/// The JavaScript module a shader's `main.ts` runs as in a browser: types stripped and its
/// `figma:shaders` import replaced by a local `defineProperties`, so it imports nothing.
pub fn browser_module(main_ts: &str) -> Result<String, ShaderError> {
    ts::to_browser_js(main_ts)
}

/// What a paint or effect runs: its source, `main.ts`, `frame.params` and clock.
#[derive(Debug)]
pub struct Program<'s> {
    pub source: &'s ShaderSource,
    pub main: &'s str,
    pub params: Value,
    /// `time_ms` for an animated shader, 0 for a still one (its output is kept across frames).
    pub time_ms: f64,
}

/// The program the `CUSTOM` paint or effect `paint` runs at `time_ms`, when one of
/// `sources` has its code.
pub fn program<'s>(
    sources: &'s [ShaderSource],
    doc: &Document,
    paint: &Val,
    time_ms: f64,
) -> Option<Program<'s>> {
    let (source, params) = shaders::program(doc, paint, sources)?;
    let (_, main) = source
        .files
        .iter()
        .find(|(f, _)| f.rsplit('/').next() == Some("main.ts"))?;
    Some(Program {
        source,
        main: main.as_str(),
        params,
        time_ms: clock(source, time_ms),
    })
}

impl Program<'_> {
    /// Cache key of its output over `width × height` (a fill), or on `input` (an effect).
    pub fn key(&self, width: u16, height: u16, input: Option<&Pixmap>) -> u64 {
        let mut hasher = DefaultHasher::new();
        (
            &self.source.label,
            self.params.to_string(),
            width,
            height,
            self.time_ms.to_bits(),
        )
            .hash(&mut hasher);
        if let Some(input) = input {
            input.data_as_u8_slice().hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// RGBA8 output of a shader as a pixmap: a fill's is straight alpha, an effect's (it has an
/// input) premultiplied.
pub fn pixmap(rgba: Vec<u8>, width: u16, height: u16, effect: bool) -> Pixmap {
    let meta = PixelMetadata {
        may_have_transparency: true,
        alpha_type: if effect {
            ImageAlphaType::AlphaPremultiplied
        } else {
            ImageAlphaType::Alpha
        },
    };
    Pixmap::from_parts(rgba, width, height, meta)
}

/// The shader clock a run sees: `time_ms` for an animated shader, 0 for a still one.
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

#[cfg(feature = "native")]
mod native {
    use std::collections::HashMap;
    use std::fmt;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use figstudio_fig::{Document, Val};
    use figstudio_mcp_core::shaders::ShaderSource;
    use figstudio_render::shader::{Shaded, ShaderFills};
    use serde_json::Value;
    use vello_cpu::Pixmap;

    use crate::{ShaderError, js, pixmap, program, ts};

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

    /// Longest one run may take, its script and its GPU work (the browser's limit too).
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

    fn run(
        main_ts: &str,
        params: &Value,
        width: u16,
        height: u16,
        time_ms: f64,
        input: Option<&[u8]>,
    ) -> Result<Pixmap, ShaderError> {
        let js = ts::to_js(main_ts)?;
        let rgba = run_within(js, params, (width, height, time_ms), input, DEADLINE)?;
        Ok(pixmap(rgba, width, height, input.is_some()))
    }

    /// Runs `js` on its own thread and waits at most `deadline`. A run past it is left to end
    /// on its own (Boa's instruction budget bounds it) and that code is not run again in this
    /// process, so a shader that hangs costs one wait, not one per frame or tile.
    pub(crate) fn run_within(
        js: String,
        params: &Value,
        (width, height, time_ms): (u16, u16, f64),
        input: Option<&[u8]>,
        deadline: std::time::Duration,
    ) -> Result<Vec<u8>, ShaderError> {
        use std::collections::HashSet;
        use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
        use std::sync::mpsc::{RecvTimeoutError, channel};

        static TIMED_OUT: Mutex<Option<HashSet<u64>>> = Mutex::new(None);
        let code = BuildHasherDefault::<DefaultHasher>::default().hash_one(&js);
        let timed_out = |insert: bool| {
            let mut set = TIMED_OUT.lock().unwrap_or_else(|e| e.into_inner());
            let set = set.get_or_insert_with(HashSet::new);
            if insert {
                !set.insert(code)
            } else {
                set.contains(&code)
            }
        };
        if timed_out(false) {
            return Err(ShaderError::script(
                "main.ts ran past the time limit before; not run again",
            ));
        }
        let (params, input) = (params.clone(), input.map(<[u8]>::to_vec));
        let (send, result) = channel();
        std::thread::Builder::new()
            .name("shader".into())
            .spawn(move || {
                let frame = js::Frame {
                    width,
                    height,
                    time_ms,
                    input: input.as_deref(),
                };
                let _ = send.send(js::run(&js, &params, &frame));
            })
            .map_err(|e| ShaderError::script(format!("cannot start the shader thread: {e}")))?;
        match result.recv_timeout(deadline) {
            Ok(rgba) => rgba,
            Err(RecvTimeoutError::Timeout) => {
                timed_out(true);
                Err(ShaderError::script(format!(
                    "main.ts ran past {} s; stopped",
                    deadline.as_secs_f64()
                )))
            }
            Err(RecvTimeoutError::Disconnected) => Err(ShaderError::script(
                "the shader thread ended without a result",
            )),
        }
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
        fn cached(&self, key: u64, run: impl FnOnce() -> Result<Pixmap, ShaderError>) -> Shaded {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(done) = cache.get(&key) {
                return Shaded::from_option(done.clone());
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
            Shaded::from_option(out)
        }
    }

    impl ShaderFills for Fills {
        fn fill(
            &self,
            doc: &Document,
            paint: &Val,
            width: u16,
            height: u16,
            time_ms: f64,
        ) -> Shaded {
            let Some(p) = program(&self.sources, doc, paint, time_ms) else {
                return Shaded::Unavailable;
            };
            self.cached(p.key(width, height, None), || {
                render(p.main, &p.params, width, height, p.time_ms)
            })
        }

        fn has_effect(&self, doc: &Document, effect: &Val) -> bool {
            program(&self.sources, doc, effect, 0.0).is_some()
        }

        fn effect(&self, doc: &Document, effect: &Val, input: &Pixmap, time_ms: f64) -> Shaded {
            let Some(p) = program(&self.sources, doc, effect, time_ms) else {
                return Shaded::Unavailable;
            };
            self.cached(p.key(input.width(), input.height(), Some(input)), || {
                render_effect(p.main, &p.params, input, p.time_ms)
            })
        }
    }
}

#[cfg(all(test, feature = "native"))]
mod deadline_tests {
    use std::time::{Duration, Instant};

    use serde_json::json;

    use crate::native::run_within;

    #[test]
    fn a_run_past_its_deadline_stops_and_is_not_run_again() {
        // About a second of work, against a 50 ms deadline.
        let js = "export function render() { let s = 0; \
                  for (let i = 0; i < 5000000; i++) { s += i; } }"
            .to_owned();
        let size = (8, 8, 0.0);
        let first = run_within(
            js.clone(),
            &json!({}),
            size,
            None,
            Duration::from_millis(50),
        );
        let Err(error) = first else {
            panic!("a second of work finished within 50 ms");
        };
        if error.is_no_gpu() {
            return;
        }
        assert!(error.to_string().contains("ran past"), "{error}");
        let started = Instant::now();
        let again = run_within(js, &json!({}), size, None, Duration::from_secs(10));
        assert!(started.elapsed() < Duration::from_millis(50));
        let error = again.unwrap_err().to_string();
        assert!(error.contains("not run again"), "{error}");
    }
}
