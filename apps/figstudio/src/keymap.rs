//! Keyboard shortcuts the user can change in Settings → Shortcuts, as in Arc. Each
//! [`Binding`] has a default; the [`Keymap`] keeps only the changes (saved with the window
//! state), so new defaults reach people who never changed that shortcut.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};

use crate::canvas::Tool;
use crate::menu::Action;

/// Something a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Binding {
    CommandBar,
    FindOnPage,
    NextMatch,
    PreviousMatch,
    Open,
    CloseFile,
    ExportSelection,
    Settings,
    ToggleSidebar,
    ZoomFit,
    ActualSize,
    ZoomIn,
    ZoomOut,
    SelectTool,
    HandTool,
    ZoomTool,
    NextSpace,
    PreviousSpace,
    /// Go to the space at this place in the footer (1 to 9), as Arc's ⌃1…⌃9.
    Space(u8),
}

/// The keys ⌃1…⌃9 go to the spaces with.
const DIGITS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

const SPACE_LABELS: [&str; 9] = [
    "Go to space 1",
    "Go to space 2",
    "Go to space 3",
    "Go to space 4",
    "Go to space 5",
    "Go to space 6",
    "Go to space 7",
    "Go to space 8",
    "Go to space 9",
];

impl Binding {
    pub const ALL: [Binding; 27] = [
        Binding::CommandBar,
        Binding::FindOnPage,
        Binding::NextMatch,
        Binding::PreviousMatch,
        Binding::Open,
        Binding::CloseFile,
        Binding::ExportSelection,
        Binding::Settings,
        Binding::ToggleSidebar,
        Binding::ZoomFit,
        Binding::ActualSize,
        Binding::ZoomIn,
        Binding::ZoomOut,
        Binding::SelectTool,
        Binding::HandTool,
        Binding::ZoomTool,
        Binding::NextSpace,
        Binding::PreviousSpace,
        Binding::Space(1),
        Binding::Space(2),
        Binding::Space(3),
        Binding::Space(4),
        Binding::Space(5),
        Binding::Space(6),
        Binding::Space(7),
        Binding::Space(8),
        Binding::Space(9),
    ];

    /// The binding that goes to the space at `index` (0-based) in the footer, if any.
    pub fn space(index: usize) -> Option<Binding> {
        u8::try_from(index + 1)
            .ok()
            .filter(|n| (1..=9).contains(n))
            .map(Binding::Space)
    }

    pub fn label(self) -> &'static str {
        match self {
            Binding::CommandBar => "Command bar",
            Binding::FindOnPage => "Find on page",
            Binding::NextMatch => "Next match",
            Binding::PreviousMatch => "Previous match",
            Binding::Open => "Open a file",
            Binding::CloseFile => "Close the file",
            Binding::ExportSelection => "Export the selection at 2×",
            Binding::Settings => "Settings",
            Binding::ToggleSidebar => "Show or hide the sidebar",
            Binding::ZoomFit => "Zoom to fit",
            Binding::ActualSize => "Actual size",
            Binding::ZoomIn => "Zoom in",
            Binding::ZoomOut => "Zoom out",
            Binding::SelectTool => "Select tool",
            Binding::HandTool => "Hand tool",
            Binding::ZoomTool => "Zoom tool (Alt to zoom out)",
            Binding::NextSpace => "Next space",
            Binding::PreviousSpace => "Previous space",
            Binding::Space(n) => SPACE_LABELS
                .get(usize::from(n).wrapping_sub(1))
                .copied()
                .unwrap_or("Go to a space"),
        }
    }

    /// The Shortcuts list's section.
    pub fn group(self) -> &'static str {
        match self {
            Binding::CommandBar
            | Binding::FindOnPage
            | Binding::NextMatch
            | Binding::PreviousMatch
            | Binding::Open
            | Binding::CloseFile
            | Binding::ExportSelection
            | Binding::Settings => "General",
            Binding::ToggleSidebar
            | Binding::ZoomFit
            | Binding::ActualSize
            | Binding::ZoomIn
            | Binding::ZoomOut => "View",
            Binding::SelectTool | Binding::HandTool | Binding::ZoomTool => "Tools",
            Binding::NextSpace | Binding::PreviousSpace | Binding::Space(_) => "Spaces",
        }
    }

    pub fn default_keys(self) -> KeyboardShortcut {
        let k = KeyboardShortcut::new;
        let cmd = Modifiers::COMMAND;
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        match self {
            Binding::CommandBar => k(cmd, Key::L),
            Binding::FindOnPage => k(cmd, Key::F),
            Binding::NextMatch => k(cmd, Key::G),
            Binding::PreviousMatch => k(shift_cmd, Key::G),
            Binding::Open => k(cmd, Key::O),
            Binding::CloseFile => k(cmd, Key::W),
            Binding::ExportSelection => k(shift_cmd, Key::E),
            Binding::Settings => k(cmd, Key::Comma),
            Binding::ToggleSidebar => k(cmd, Key::Backslash),
            Binding::ZoomFit => k(Modifiers::SHIFT, Key::Num1),
            Binding::ActualSize => k(Modifiers::SHIFT, Key::Num0),
            Binding::ZoomIn => k(Modifiers::NONE, Key::Equals),
            Binding::ZoomOut => k(Modifiers::NONE, Key::Minus),
            Binding::SelectTool => k(Modifiers::NONE, Key::V),
            Binding::HandTool => k(Modifiers::NONE, Key::H),
            Binding::ZoomTool => k(Modifiers::NONE, Key::Z),
            Binding::NextSpace => k(Modifiers::CTRL, Key::Tab),
            Binding::PreviousSpace => k(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab),
            Binding::Space(n) => k(
                Modifiers::CTRL,
                DIGITS
                    .get(usize::from(n).wrapping_sub(1))
                    .copied()
                    .unwrap_or(Key::Num0),
            ),
        }
    }

    /// What the shortcut runs.
    pub fn action(self) -> Action {
        match self {
            Binding::CommandBar => Action::CommandBar,
            Binding::FindOnPage => Action::FindOnPage,
            Binding::NextMatch => Action::FindNext,
            Binding::PreviousMatch => Action::FindPrevious,
            Binding::Open => Action::Open,
            Binding::CloseFile => Action::CloseTab,
            Binding::ExportSelection => Action::ExportSelection(2.0),
            Binding::Settings => Action::Settings,
            Binding::ToggleSidebar => Action::ToggleLayers,
            Binding::ZoomFit => Action::ZoomFit,
            Binding::ActualSize => Action::ActualSize,
            Binding::ZoomIn => Action::ZoomBy(1.25),
            Binding::ZoomOut => Action::ZoomBy(0.8),
            Binding::SelectTool => Action::SelectTool(Tool::Select),
            Binding::HandTool => Action::SelectTool(Tool::Hand),
            Binding::ZoomTool => Action::SelectTool(Tool::Zoom),
            Binding::NextSpace => Action::NextSpace,
            Binding::PreviousSpace => Action::PreviousSpace,
            Binding::Space(n) => Action::GoToSpace(usize::from(n).saturating_sub(1)),
        }
    }

    /// The binding that runs `action`, if any.
    pub fn of(action: Action) -> Option<Binding> {
        Binding::ALL.into_iter().find(|b| b.action() == action)
    }

    /// Keys without ⌘, Ctrl or Alt act only while nothing takes typing.
    pub fn is_plain(keys: &KeyboardShortcut) -> bool {
        let m = keys.modifiers;
        !(m.command || m.ctrl || m.mac_cmd || m.alt)
    }
}

/// A shortcut as it is saved: the key's name and the logical modifiers (`command` is ⌘ on a
/// Mac and Ctrl elsewhere; `ctrl` is the Control key on a Mac).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    key: String,
    #[serde(default)]
    command: bool,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    shift: bool,
}

impl Keys {
    pub fn from_shortcut(s: &KeyboardShortcut) -> Self {
        let m = s.modifiers;
        Self {
            key: s.logical_key.name().to_owned(),
            command: m.command || m.mac_cmd,
            // `ctrl` duplicates `command` except on a Mac, where ⌘ (`mac_cmd`) and Control
            // are two keys and a chord can hold both.
            ctrl: m.ctrl && (m.mac_cmd || !m.command),
            alt: m.alt,
            shift: m.shift,
        }
    }

    pub fn to_shortcut(&self) -> Option<KeyboardShortcut> {
        let key = Key::from_name(&self.key)?;
        let mut m = Modifiers::NONE;
        if self.command {
            m |= Modifiers::COMMAND;
        }
        if self.ctrl {
            m |= Modifiers::CTRL;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        Some(KeyboardShortcut::new(m, key))
    }
}

/// The shortcuts that differ from the defaults. `None` keys: no shortcut.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    /// Loaded leniently: a damaged entry is dropped, not the whole saved window state.
    #[serde(default, deserialize_with = "lenient_changes")]
    changes: Vec<(Binding, Option<Keys>)>,
}

/// One saved change, or something this version cannot read (an unknown shortcut name, a
/// key with no name).
#[derive(Deserialize)]
#[serde(untagged)]
enum Saved {
    Change((Binding, Option<Keys>)),
    Unreadable(serde::de::IgnoredAny),
}

/// The changes that can be read, each binding once, and no two with the same keys (the
/// earlier one keeps them, as `Keymap::set` leaves it).
fn lenient_changes<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<(Binding, Option<Keys>)>, D::Error> {
    let saved = Vec::<Saved>::deserialize(d)?;
    let mut map = Keymap::default();
    for item in saved {
        let Saved::Change((binding, keys)) = item else {
            continue;
        };
        let real = Binding::ALL.contains(&binding);
        let readable = keys.as_ref().is_none_or(|k| k.to_shortcut().is_some());
        if !real || !readable || map.is_changed(binding) {
            continue;
        }
        let shortcut = keys.as_ref().and_then(Keys::to_shortcut);
        if shortcut.is_some_and(|s| {
            Binding::ALL
                .into_iter()
                .any(|b| b != binding && map.get(b).is_some_and(|k| same(&k, &s)))
        }) {
            continue;
        }
        map.put(binding, shortcut);
    }
    Ok(map.changes)
}

impl Keymap {
    /// The keys that run `binding`, if any.
    pub fn get(&self, binding: Binding) -> Option<KeyboardShortcut> {
        match self.changes.iter().find(|(b, _)| *b == binding) {
            Some((_, keys)) => keys.as_ref().and_then(Keys::to_shortcut),
            None => Some(binding.default_keys()),
        }
    }

    /// The keys that run `action` (for menus and the command bar).
    pub fn for_action(&self, action: Action) -> Option<KeyboardShortcut> {
        Binding::of(action).and_then(|b| self.get(b))
    }

    pub fn is_changed(&self, binding: Binding) -> bool {
        self.changes.iter().any(|(b, _)| *b == binding)
    }

    pub fn any_changed(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Gives `binding` the `keys` (`None`: no shortcut). Another binding with the same keys
    /// loses them; that one is returned so the person can be told.
    pub fn set(&mut self, binding: Binding, keys: Option<KeyboardShortcut>) -> Option<Binding> {
        let taken = keys.and_then(|keys| {
            Binding::ALL
                .into_iter()
                .find(|b| *b != binding && self.get(*b).is_some_and(|k| same(&k, &keys)))
        });
        if let Some(other) = taken {
            self.put(other, None);
        }
        self.put(binding, keys);
        taken
    }

    pub fn reset(&mut self, binding: Binding) {
        self.changes.retain(|(b, _)| *b != binding);
        // Its default may now be on another binding too: that one gives it up.
        let keys = binding.default_keys();
        let clash = Binding::ALL
            .into_iter()
            .find(|b| *b != binding && self.get(*b).is_some_and(|k| same(&k, &keys)));
        if let Some(other) = clash {
            self.put(other, None);
        }
    }

    pub fn reset_all(&mut self) {
        self.changes.clear();
    }

    fn put(&mut self, binding: Binding, keys: Option<KeyboardShortcut>) {
        self.changes.retain(|(b, _)| *b != binding);
        if keys.is_some_and(|k| same(&k, &binding.default_keys())) {
            return;
        }
        self.changes
            .push((binding, keys.as_ref().map(Keys::from_shortcut)));
    }
}

/// The same keys and the same logical modifiers.
pub fn same(a: &KeyboardShortcut, b: &KeyboardShortcut) -> bool {
    Keys::from_shortcut(a) == Keys::from_shortcut(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_moves_the_keys_and_a_reset_brings_the_default_back() {
        let mut map = Keymap::default();
        let j = KeyboardShortcut::new(Modifiers::COMMAND, Key::J);
        assert_eq!(
            map.get(Binding::CommandBar),
            Some(Binding::CommandBar.default_keys())
        );
        assert_eq!(map.set(Binding::CommandBar, Some(j)), None);
        assert_eq!(map.get(Binding::CommandBar), Some(j));
        assert!(map.is_changed(Binding::CommandBar));
        // Taking keys another shortcut has moves them.
        let l = Binding::CommandBar.default_keys();
        assert_eq!(
            map.set(Binding::FindOnPage, Some(j)),
            Some(Binding::CommandBar)
        );
        assert_eq!(
            map.get(Binding::CommandBar),
            None,
            "the command bar lost ⌘J"
        );
        assert_eq!(map.get(Binding::FindOnPage), Some(j));
        map.reset(Binding::CommandBar);
        assert_eq!(map.get(Binding::CommandBar), Some(l));
        map.set(Binding::HandTool, None);
        assert_eq!(map.get(Binding::HandTool), None, "no shortcut");
        map.reset_all();
        assert!(!map.any_changed());
        assert_eq!(
            map.get(Binding::FindOnPage),
            Some(Binding::FindOnPage.default_keys())
        );
    }

    #[test]
    fn a_reset_takes_its_default_back_from_another_binding() {
        let mut map = Keymap::default();
        map.set(Binding::HandTool, Some(Binding::SelectTool.default_keys()));
        assert_eq!(map.get(Binding::SelectTool), None);
        map.reset(Binding::SelectTool);
        assert_eq!(
            map.get(Binding::SelectTool),
            Some(Binding::SelectTool.default_keys())
        );
        assert_eq!(map.get(Binding::HandTool), None, "it gave the V back");
    }

    #[test]
    fn control_is_kept_beside_command_on_a_mac_and_not_doubled_elsewhere() {
        let mac_ctrl_cmd = Modifiers {
            ctrl: true,
            command: true,
            mac_cmd: true,
            ..Modifiers::NONE
        };
        let mac_cmd = Modifiers {
            command: true,
            mac_cmd: true,
            ..Modifiers::NONE
        };
        let both = KeyboardShortcut::new(mac_ctrl_cmd, Key::K);
        let cmd = KeyboardShortcut::new(mac_cmd, Key::K);
        assert!(!same(&both, &cmd), "⌃⌘K is not ⌘K");
        let back = Keys::from_shortcut(&both).to_shortcut().expect("readable");
        assert!(back.modifiers.ctrl && back.modifiers.command, "{back:?}");
        // Windows and Linux: Ctrl is the command key, reported as both.
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let saved = Keys::from_shortcut(&KeyboardShortcut::new(ctrl, Key::K));
        assert!(!saved.ctrl && saved.command, "{saved:?}");
    }

    #[test]
    fn keys_survive_saving() {
        for b in Binding::ALL {
            let keys = b.default_keys();
            assert_eq!(
                Keys::from_shortcut(&keys).to_shortcut(),
                Some(keys),
                "{b:?}"
            );
        }
        let mut map = Keymap::default();
        map.set(
            Binding::Open,
            Some(KeyboardShortcut::new(
                Modifiers::ALT | Modifiers::SHIFT,
                Key::P,
            )),
        );
        let text = serde_json::to_string(&map).expect("saves");
        let back: Keymap = serde_json::from_str(&text).expect("loads");
        assert_eq!(back, map);
        assert_eq!(Binding::of(Action::CommandBar), Some(Binding::CommandBar));
        for b in Binding::ALL {
            assert_eq!(Binding::of(b.action()), Some(b), "one action each: {b:?}");
        }
    }
}
