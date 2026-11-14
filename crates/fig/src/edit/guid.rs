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
        let mut max_in_edit_session = 0u32;
        let mut session = EDIT_SESSION_BASE;
        for n in &doc.nodes {
            if n.id.0 >= EDIT_SESSION_BASE {
                if n.id.0 > session {
                    session = n.id.0;
                    max_in_edit_session = n.id.1;
                } else if n.id.0 == session {
                    max_in_edit_session = max_in_edit_session.max(n.id.1);
                }
            }
        }
        Self {
            session,
            next_local: max_in_edit_session.saturating_add(1).max(1),
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
