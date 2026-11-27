//! Monotonic, collision-free `Guid` allocation for newly created or pasted nodes.

use std::collections::HashSet;

use crate::{Document, Guid};

/// Base session ID for newly allocated nodes (above normal Figma user session IDs).
pub const EDIT_SESSION_BASE: u32 = 3_000_000_000;

/// Allocates fresh [`Guid`]s guaranteed not to collide with any node in a [`Document`].
#[derive(Clone, Debug)]
pub struct GuidAlloc {
    session: u32,
    next_local: u32,
    occupied: HashSet<Guid>,
}

fn next_session(session: u32) -> u32 {
    session.checked_add(1).unwrap_or(1)
}

impl GuidAlloc {
    /// Creates an allocator seeded against every existing `(sessionID, localID)` in `doc`.
    pub fn for_document(doc: &Document) -> Self {
        let mut alloc = Self {
            session: EDIT_SESSION_BASE,
            next_local: 1,
            occupied: HashSet::with_capacity(doc.nodes.len()),
        };
        for n in &doc.nodes {
            alloc.reserve(n.id);
        }
        alloc
    }

    /// Records `id` as occupied and advances the cursor past it when it lies in the active
    /// session so later calls to [`Self::next_guid`] never collide with it.
    pub fn reserve(&mut self, id: Guid) {
        self.occupied.insert(id);
        if id.0 == self.session && id.1 >= self.next_local {
            if id.1 == u32::MAX {
                self.session = next_session(self.session);
                self.next_local = 1;
            } else {
                self.next_local = id.1 + 1;
            }
        }
    }

    /// Uses `explicit` when provided (recording it as occupied), or allocates a fresh [`Guid`].
    pub fn take_or_next(&mut self, explicit: Option<Guid>) -> Guid {
        match explicit {
            Some(id) => {
                self.reserve(id);
                id
            }
            None => self.next_guid(),
        }
    }

    /// Allocates the next unique [`Guid`]. Rolls over to the next session ID if `localID` wraps
    /// and skips any already-occupied IDs.
    pub fn next_guid(&mut self) -> Guid {
        loop {
            let candidate = Guid(self.session, self.next_local);
            if self.next_local == u32::MAX {
                self.session = next_session(self.session);
                self.next_local = 1;
            } else {
                self.next_local += 1;
            }
            if self.occupied.insert(candidate) {
                return candidate;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_strictly_unique_monotonic_guids() {
        let mut alloc = GuidAlloc {
            session: EDIT_SESSION_BASE,
            next_local: 1,
            occupied: HashSet::new(),
        };
        let a = alloc.next_guid();
        let b = alloc.next_guid();
        assert_ne!(a, b);
        assert_eq!(a, Guid(EDIT_SESSION_BASE, 1));
        assert_eq!(b, Guid(EDIT_SESSION_BASE, 2));
    }

    #[test]
    fn wraps_session_past_u32_max_and_skips_occupied() {
        let mut alloc = GuidAlloc {
            session: u32::MAX,
            next_local: u32::MAX,
            occupied: HashSet::from([Guid(u32::MAX, u32::MAX), Guid(1, 1)]),
        };
        assert_eq!(alloc.next_guid(), Guid(1, 2));
    }
}
