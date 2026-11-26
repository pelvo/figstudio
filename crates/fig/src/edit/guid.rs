//! Monotonic, collision-free `Guid` allocation for newly created or pasted nodes.

use crate::{Document, Guid};

/// Base session ID for newly allocated nodes (above normal Figma user session IDs).
pub const EDIT_SESSION_BASE: u32 = 3_000_000_000;

/// Allocates fresh [`Guid`]s guaranteed not to collide with any node in a [`Document`].
#[derive(Clone, Debug)]
pub struct GuidAlloc {
    session: u32,
    next_local: u32,
}

impl GuidAlloc {
    /// Creates an allocator seeded above every existing `(sessionID, localID)` in `doc`.
    pub fn for_document(doc: &Document) -> Self {
        let mut alloc = Self {
            session: EDIT_SESSION_BASE,
            next_local: 1,
        };
        for n in &doc.nodes {
            alloc.reserve(n.id);
        }
        alloc
    }

    /// Advances the allocator past `id` so later calls to [`Self::next_guid`] never collide with it.
    pub fn reserve(&mut self, id: Guid) {
        if id.0 > self.session {
            if id.0 == u32::MAX && id.1 == u32::MAX {
                return;
            }
            self.session = id.0;
            if id.1 == u32::MAX {
                self.session = self.session.saturating_add(1);
                self.next_local = 1;
            } else {
                self.next_local = id.1 + 1;
            }
        } else if id.0 == self.session && id.1 >= self.next_local {
            if id.1 == u32::MAX {
                self.session = self.session.saturating_add(1);
                self.next_local = 1;
            } else {
                self.next_local = id.1 + 1;
            }
        }
    }

    /// Uses `explicit` when provided (advancing the allocator past it), or allocates a fresh [`Guid`].
    pub fn take_or_next(&mut self, explicit: Option<Guid>) -> Guid {
        match explicit {
            Some(id) => {
                self.reserve(id);
                id
            }
            None => self.next_guid(),
        }
    }

    /// Allocates the next unique [`Guid`]. Rolls over to the next session ID if `localID` wraps.
    pub fn next_guid(&mut self) -> Guid {
        if self.next_local == u32::MAX {
            self.session = self.session.saturating_add(1);
            self.next_local = 1;
        }
        let id = Guid(self.session, self.next_local);
        self.next_local += 1;
        id
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
        };
        let a = alloc.next_guid();
        let b = alloc.next_guid();
        assert_ne!(a, b);
        assert_eq!(a, Guid(EDIT_SESSION_BASE, 1));
        assert_eq!(b, Guid(EDIT_SESSION_BASE, 2));
    }
}
