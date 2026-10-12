//! The browser's GPU canvas, the parts without a device (docs/GPU.md, L4 notes): wgpu objects
//! stay on the main thread in the browser, so the render worker only builds each tile's
//! `vello_gpu::Scene` from its recording, and the main thread renders a bounded number of
//! them per frame (`gpu_tiles_web.rs`). Compiled natively too, for the tests.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use figstudio_fig::Guid;
use figstudio_render::{GPU_LAYER_TEXTURE, Recording, Renderer, Viewport};
use web_time::Duration;

use crate::gpu_common::{CpuWhy, Fallback, guarded, scene};
use crate::recordings::{Drawn, Recordings};
use crate::tiles::{Key, Range};
use crate::worker::{RenderDone, RenderJob};

/// A GPU tile on its way to the main thread: its scene, rendered there.
pub type GpuTexture = vello_gpu::Scene;

/// Most GPU tiles the main thread renders in one frame.
pub const TILES_PER_FRAME: usize = 4;
/// After this much GPU work in a frame, the rest waits for the next frame.
pub const TIME_PER_FRAME: Duration = Duration::from_millis(8);

/// The render worker's side of the browser GPU: builds scenes, or says why a tile goes to
/// the CPU.
#[derive(Debug)]
pub struct TileGpu {
    /// The largest intermediate texture side, or why the canvas is on the CPU.
    gpu: Result<u16, CpuWhy>,
    /// Set by the main thread when the device is lost or a render failed: the rest of the
    /// session draws on the CPU.
    lost: Arc<AtomicBool>,
}

impl Default for TileGpu {
    /// The CPU, as asked (tests and `?renderer=cpu`).
    fn default() -> Self {
        TileGpu::new(Err(CpuWhy::Asked), Arc::default())
    }
}

impl TileGpu {
    /// Scenes for a device whose largest texture is `max_texture` (or the CPU, `why`);
    /// `lost` is shared with the main thread's renderer.
    pub fn new(gpu: Result<u32, CpuWhy>, lost: Arc<AtomicBool>) -> Self {
        let side = |s: u32| u16::try_from(s).unwrap_or(u16::MAX).min(GPU_LAYER_TEXTURE);
        TileGpu {
            gpu: gpu.map(side),
            lost,
        }
    }

    /// Draws tile `key` of `job`: its scene when its recording allows, else on the CPU with
    /// `Stats::gpu_fallback_tiles` set when the GPU was wanted.
    pub fn draw(
        &mut self,
        recordings: &mut Recordings,
        renderer: &mut Renderer,
        job: &RenderJob,
        key: &Key,
    ) -> Drawn<GpuTexture> {
        crate::gpu_common::draw(recordings, renderer, job, key, |rec, viewport| {
            self.scene(rec, viewport)
        })
    }

    fn scene(
        &self,
        rec: Option<&Recording>,
        viewport: &Viewport,
    ) -> Result<vello_gpu::Scene, Fallback> {
        let max_texture = self.gpu.map_err(Fallback::Cpu)?;
        if self.lost.load(Ordering::Acquire) {
            return Err(Fallback::Lost);
        }
        guarded(&self.lost, || scene(rec, viewport, max_texture))
    }
}

/// The `renderer` query parameter of the page's `location.search` (`?renderer=cpu|gpu`),
/// for [`crate::gpu_common::choose`].
pub fn asked_renderer(search: &str) -> Option<String> {
    let query = search.strip_prefix('?').unwrap_or(search);
    form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "renderer")
        .map(|(_, v)| v.into_owned())
}

/// Most scenes kept for the main thread after [`Pending::prune`]. A plan is at most half the
/// tile cache (63 tiles in the browser, `tiles::plan`), so the current view always fits.
pub const MAX_PENDING: usize = 64;

/// Scenes waiting for the main thread, rendered a few per frame. Pruned every frame to the
/// current plan, nearest its centre first, and to [`MAX_PENDING`], so a view that moved on
/// leaves nothing queued. Every scene dropped is handed back to the caller, which must
/// acknowledge it to the worker as not delivered (`tiles_view::received`), or the worker
/// would skip that tile when it is asked for again (`tile_queue::Queue`).
#[derive(Default)]
pub struct Pending {
    queue: VecDeque<RenderDone>,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("len", &self.queue.len())
            .finish_non_exhaustive()
    }
}

impl Pending {
    pub fn push(&mut self, item: RenderDone) {
        self.queue.push_back(item);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[cfg_attr(
        all(test, not(target_arch = "wasm32")),
        expect(dead_code, reason = "the browser repaints while scenes wait")
    )]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Drops the scenes of a closed document (its tab and worker history are gone).
    #[cfg_attr(
        all(test, not(target_arch = "wasm32")),
        expect(dead_code, reason = "the browser forgets closed documents")
    )]
    pub fn forget(&mut self, doc: u64) {
        self.queue.retain(|d| d.key.doc != doc);
    }

    /// Keeps the scenes of `plan` (the last frame's document, page and planned tiles, ring
    /// included), nearest its centre first, at most [`MAX_PENDING`]; hands every other one to
    /// `dropped`.
    pub fn prune(&mut self, plan: Option<(u64, Guid, Range)>, mut dropped: impl FnMut(RenderDone)) {
        let rank = |k: &Key| {
            let (doc, page, range) = plan?;
            if k.doc != doc || k.page != page || !range.contains(k) {
                return None;
            }
            // Twice the distance from the range's centre, in f64: no overflow at any pan.
            let d = |k: i32, a: i32, b: i32| 2.0 * f64::from(k) + 1.0 - f64::from(a) - f64::from(b);
            let (dx, dy) = (d(k.x, range.x0, range.x1), d(k.y, range.y0, range.y1));
            Some(dx * dx + dy * dy)
        };
        let mut kept = Vec::with_capacity(self.queue.len());
        for item in self.queue.drain(..) {
            match rank(&item.key) {
                Some(r) => kept.push((r, item)),
                None => dropped(item),
            }
        }
        // Stable: equally near scenes keep their arrival order.
        kept.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (i, (_, item)) in kept.into_iter().enumerate() {
            if i < MAX_PENDING {
                self.queue.push_back(item);
            } else {
                dropped(item);
            }
        }
    }

    /// Hands items to `render`, which returns the time it took, until [`TILES_PER_FRAME`]
    /// were rendered or [`TIME_PER_FRAME`] was spent. At least one goes per frame, so a slow
    /// tile (the first one compiles the shaders) cannot stall the queue.
    pub fn pump(&mut self, mut render: impl FnMut(RenderDone) -> Duration) {
        let mut spent = Duration::ZERO;
        for _ in 0..TILES_PER_FRAME {
            if spent >= TIME_PER_FRAME {
                break;
            }
            let Some(item) = self.queue.pop_front() else {
                break;
            };
            spent += render(item);
        }
    }
}

#[cfg(test)]
#[path = "gpu_scenes_tests.rs"]
mod tests;
