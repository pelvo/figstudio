//! Joining another browser's agent-access room by link: what the URL fragment proposes,
//! and the one confirmation that gates it. Pure (no browser types) so native tests cover
//! the security-relevant decisions; `mcp_web.rs` only wires them to the browser.
use crate::mcp_pair::{ExecutorPair, pair_from_fragment};
use crate::mcp_wire::token_from_fragment;

/// What the URL fragment said, at startup or on a `hashchange`.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    /// A pairing token for the local bridge (`#mcp=`).
    pub token: Option<String>,
    /// A valid room/executor pair named by a link (`#room=…&exec=…`); not yet trusted.
    pub pair: Option<ExecutorPair>,
    /// The fragment without its `room` and `exec` keys, when there was one to consume.
    pub stripped: Option<String>,
}

pub(crate) fn parse_link(fragment: Option<&str>) -> Link {
    let Some(fragment) = fragment else {
        return Link::default();
    };
    Link {
        token: token_from_fragment(fragment),
        pair: pair_from_fragment(fragment),
        stripped: strip_secret_keys(fragment),
    }
}

/// The fragment without its `room` and `exec` keys — consumed the moment they are read,
/// whatever else the fragment says and whether the pair was even valid — keeping unrelated
/// keys such as a `#mcp=` pairing. `None` when there was no `room` or `exec` key at all.
/// The result is a whole fragment (with `#`) or empty.
pub(crate) fn strip_secret_keys(fragment: &str) -> Option<String> {
    let fragment = fragment
        .rsplit_once('#')
        .map_or(fragment, |(_, after)| after);
    let pairs = form_urlencoded::parse(fragment.as_bytes());
    if !pairs.clone().any(|(key, _)| key == "room" || key == "exec") {
        return None;
    }
    let mut kept = form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        if key != "room" && key != "exec" {
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
/// naming the pair already in use asks nothing (the same pair cannot re-prompt).
pub(crate) fn proposed_pair(
    link: Option<ExecutorPair>,
    pairing_applies: bool,
    current: Option<&ExecutorPair>,
) -> Option<ExecutorPair> {
    link.filter(|_| !pairing_applies)
        .filter(|pair| Some(pair) != current)
}

/// A room named by a link, not yet trusted: nothing saves, connects or echoes it until
/// the user accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomProposal(ExecutorPair);

impl RoomProposal {
    pub(crate) fn new(pair: ExecutorPair) -> Self {
        Self(pair)
    }

    pub(crate) fn pair(&self) -> &ExecutorPair {
        &self.0
    }

    pub(crate) fn into_pair(self) -> ExecutorPair {
        self.0
    }

    /// Enough of the room to recognize it, never the whole capability.
    pub(crate) fn masked(&self) -> String {
        let room = self.0.room();
        format!("{}…{}", &room[..9], &room[room.len().saturating_sub(4)..])
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
        "That link grants executor authority: anyone holding it can run the MCP tools \
             on the documents open here. Room {} would replace this browser's own room, so \
             copy the new MCP URL once after joining.",
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
