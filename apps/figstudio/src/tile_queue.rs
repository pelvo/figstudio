//! The render worker's tile queue: one batch per tab, newest first, and what was sent but
//! not yet seen by the UI. Pure (no threads), so it is tested without a worker.

use crate::tiles::Key;
use crate::worker::RenderJob;

/// What the UI sends to the render worker.
pub enum Msg {
    Batch(RenderJob),
    /// A tab closed: drop its queued tiles and history.
    Close {
        tab: u64,
    },
}

#[derive(Default)]
pub struct Queue {
    jobs: Vec<RenderJob>,
    /// Tiles sent to the UI and not acknowledged by it yet: `(tab, key, images)`. A batch is
    /// planned before those results reach the UI, so it may list them again; they are skipped.
    unacked: Vec<(u64, Key, u64)>,
}

impl Queue {
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Takes a message; returns how many queued batches it dropped (each counts as one job
    /// in flight).
    pub fn take(&mut self, msg: Msg) -> usize {
        match msg {
            Msg::Batch(mut job) => {
                let tab = job.tab;
                for (key, images) in std::mem::take(&mut job.received) {
                    self.unacked.retain(|u| *u != (tab, key, images));
                }
                let images = job.images;
                let unacked = &self.unacked;
                job.tiles.retain(|k| !unacked.contains(&(tab, *k, images)));
                let dropped = self.drop_tab(tab);
                // The newest batch is the tab on screen; other tabs' leftovers wait.
                self.jobs.insert(0, job);
                dropped
            }
            Msg::Close { tab } => {
                self.unacked.retain(|u| u.0 != tab);
                self.drop_tab(tab)
            }
        }
    }

    fn drop_tab(&mut self, tab: u64) -> usize {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.tab != tab);
        before - self.jobs.len()
    }

    /// The next tile to render and its batch, or `None` after dropping an empty batch at the
    /// front (the caller counts it as finished and asks again).
    pub fn next(&mut self) -> Option<(&RenderJob, Key)> {
        let key = self.jobs.first_mut()?.tiles.pop_front();
        match key {
            Some(key) => Some((&self.jobs[0], key)),
            None => {
                self.jobs.remove(0);
                None
            }
        }
    }

    /// Records that `key` of the front batch was sent to the UI; returns true when that batch
    /// is now finished (and removed).
    pub fn sent(&mut self, tab: u64, key: Key, images: u64) -> bool {
        self.unacked.push((tab, key, images));
        let done = self
            .jobs
            .first()
            .is_some_and(|j| j.tab == tab && j.tiles.is_empty());
        if done {
            self.jobs.remove(0);
        }
        done
    }
}

#[cfg(test)]
#[path = "tile_queue_tests.rs"]
mod tests;
