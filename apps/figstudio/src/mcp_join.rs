//! Joining another browser's agent-access room by link: what the URL fragment proposes,
//! and the one confirmation that gates it. Pure (no browser types) so native tests cover
//! the security-relevant decisions; `mcp_web.rs` only wires them to the browser.
use crate::mcp_wire::{room_from_fragment, token_from_fragment};

/// What the URL fragment said, at startup or on a `hashchange`.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    /// A pairing token for the local bridge (`#mcp=`).
    pub token: Option<String>,
    /// A valid room named by a link (`#room=`); not yet trusted.
    pub room: Option<String>,
    /// The fragment without its `room` key, when there was one to consume.
    pub stripped: Option<String>,
}

pub(crate) fn parse_link(fragment: Option<&str>) -> Link {
    let Some(fragment) = fragment else {
        return Link::default();
    };
    Link {
        token: token_from_fragment(fragment),
        room: room_from_fragment(fragment),
        stripped: strip_room_key(fragment),
    }
}

/// The fragment without its `room` key — consumed the moment it is read, whatever else
/// the fragment says — keeping unrelated keys such as a `#mcp=` pairing. `None` when
/// there is no `room` key. The result is a whole fragment (with `#`) or empty.
pub(crate) fn strip_room_key(fragment: &str) -> Option<String> {
    let fragment = fragment
        .rsplit_once('#')
        .map_or(fragment, |(_, after)| after);
    let pairs = form_urlencoded::parse(fragment.as_bytes());
    if !pairs.clone().any(|(key, _)| key == "room") {
        return None;
    }
    let mut kept = form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        if key != "room" {
            kept.append_pair(&key, &value);
        }
    }
    let kept = kept.finish();
    Some(if kept.is_empty() {
        String::new()
    } else {
        format!("#{kept}")
    })
}

/// What a parsed link means for joining. Deliberate precedence: a pairing being applied
/// wins (the user chose the local bridge), an unchanged `#mcp=` key does not, and a link
/// naming the room already in use asks nothing.
pub(crate) fn proposed_room(
    link: Option<String>,
    pairing_applies: bool,
    current: &str,
) -> Option<String> {
    link.filter(|_| !pairing_applies)
        .filter(|room| room != current)
}

/// A room named by a link, not yet trusted: nothing saves, connects or echoes it until
/// the user accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomProposal(String);

impl RoomProposal {
    pub(crate) fn new(room: String) -> Self {
        debug_assert!(crate::mcp_wire::valid_room(&room));
        Self(room)
    }

    pub(crate) fn room(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_room(self) -> String {
        self.0
    }

    /// Enough of the token to recognize the room, never the whole capability.
    pub(crate) fn masked(&self) -> String {
        format!(
            "{}…{}",
            &self.0[..9],
            &self.0[self.0.len().saturating_sub(4)..]
        )
    }
}

/// The reply to a pending linked-room proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JoinReply {
    Join,
    Stay,
}

/// The one question a link has to answer before anything joins. `Stay` is the default:
/// Enter alone never joins, Escape always stays (and is consumed, so the app's
/// Clear-selection shortcut stays quiet), and joining takes a click or Tab plus Enter.
pub(crate) fn join_dialog(ui: &mut egui::Ui, proposal: &RoomProposal) -> Option<JoinReply> {
    ui.label("A link asks this browser to join another agent-access room.");
    ui.label(format!(
        "Room {} would replace this browser's own room. Anyone holding that link could \
             then reach the documents open here.",
        proposal.masked()
    ));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let stay = ui.button("Stay in this room");
        let join = ui.button("Join that room");
        if !stay.has_focus() && !join.has_focus() {
            stay.request_focus();
        }
        let enter =
            ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if join.clicked() || (join.has_focus() && enter) {
            Some(JoinReply::Join)
        } else if stay.clicked() || (stay.has_focus() && enter) || escape {
            if escape {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            }
            Some(JoinReply::Stay)
        } else {
            None
        }
    })
    .inner
}

#[cfg(test)]
#[path = "mcp_join_tests.rs"]
mod tests;
