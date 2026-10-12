//! Byte credits for the browser's GPU scenes (docs/GPU.md, L4 notes): the render worker
//! reserves a scene's estimated bytes before building it, and the reservation travels with the
//! scene through the channel and the main thread's queue. Dropping the scene (rendered, pruned,
//! its tab or document closed, or the channel torn down) releases the bytes, so the scenes
//! alive anywhere never exceed the budget.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Most bytes of scenes alive at once (built, in the channel, or queued on the main thread).
pub const BUDGET: usize = 64 << 20;

/// The shared budget; clones share it.
#[derive(Clone, Debug)]
pub struct Credits {
    used: Arc<AtomicUsize>,
    budget: usize,
}

impl Default for Credits {
    fn default() -> Self {
        Credits::new(BUDGET)
    }
}

impl Credits {
    pub fn new(budget: usize) -> Self {
        Credits {
            used: Arc::default(),
            budget,
        }
    }

    /// Bytes reserved now.
    #[cfg(test)]
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    /// Reserves `bytes`, or `None` when they do not fit what is left of the budget.
    pub fn reserve(&self, bytes: usize) -> Option<Reservation> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|&n| n <= self.budget)
            })
            .ok()?;
        Some(Reservation {
            used: self.used.clone(),
            bytes,
        })
    }
}

/// Reserved bytes, given back when dropped.
#[derive(Debug)]
pub struct Reservation {
    used: Arc<AtomicUsize>,
    bytes: usize,
}

impl Reservation {
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
