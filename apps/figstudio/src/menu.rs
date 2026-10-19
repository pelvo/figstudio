//! Shared menu state and actions; renderers never implement the commands themselves.
use crate::{App, Tool};
use egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Action {
    Open,
    ExportPage(f64),
    ExportSelection(f64),
    CloseTab,
    Quit,
    ZoomFit,
    ActualSize,
    ToggleLayers,
    ToggleProperties,
    CommandBar,
    SpaceIcon,
    RenameSpace,
    SpaceColour,
    ResetSpace,
    FindOnPage,
    Appearance(Appearance),
    ReduceMotion,
    AgentAccess,
    About,
    SelectTool(Tool),
    ClearSelection,
    ZoomBy(f32),
    #[cfg(target_arch = "wasm32")]
    CjkFont(crate::cjk::Variant),
}

/// View > Appearance: follow the system, or force light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl From<egui::ThemePreference> for Appearance {
    fn from(preference: egui::ThemePreference) -> Self {
        match preference {
            egui::ThemePreference::System => Self::System,
            egui::ThemePreference::Light => Self::Light,
            egui::ThemePreference::Dark => Self::Dark,
        }
    }
}

impl Appearance {
    fn label(self) -> &'static str {
        match self {
            Self::System => "Match system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

impl Action {
    pub(crate) fn shortcut(self) -> Option<KeyboardShortcut> {
        let (modifiers, key) = match self {
            Self::Open => (Modifiers::COMMAND, Key::O),
            Self::CloseTab => (Modifiers::COMMAND, Key::W),
            Self::ExportSelection(2.0) => (Modifiers::COMMAND | Modifiers::SHIFT, Key::E),
            Self::ZoomFit => (Modifiers::SHIFT, Key::Num1),
            Self::CommandBar => return Some(crate::command::SHORTCUT),
            Self::FindOnPage => (Modifiers::COMMAND, Key::F),
            Self::ToggleLayers => (Modifiers::COMMAND, Key::Backslash),
            Self::ActualSize => (Modifiers::SHIFT, Key::Num0),
            _ => return None,
        };
        Some(KeyboardShortcut::new(modifiers, key))
    }
}

#[derive(Debug)]
pub(crate) struct Item {
    pub label: &'static str,
    pub action: Option<Action>,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub children: Vec<Item>,
}

impl Item {
    fn action(label: &'static str, action: Action, enabled: bool) -> Self {
        Self {
            label,
            action: Some(action),
            enabled,
            checked: None,
            children: vec![],
        }
    }
    fn group(label: &'static str, children: Vec<Self>) -> Self {
        Self {
            label,
            action: None,
            enabled: true,
            checked: None,
            children,
        }
    }
    fn separator() -> Self {
        Self::group("", vec![])
    }
    fn checked(label: &'static str, action: Action, checked: bool) -> Self {
        Self {
            checked: Some(checked),
            ..Self::action(label, action, true)
        }
    }
}

pub(crate) fn tree(app: &App) -> Vec<Item> {
    let has_tab = app.tabs.get(app.active).is_some();
    let selection = app
        .tabs
        .get(app.active)
        .is_some_and(|t| t.selection.is_some());
    let export = |label, selection_only| Item {
        enabled: if selection_only { selection } else { has_tab },
        ..Item::group(
            label,
            [1.0, 2.0, 3.0]
                .into_iter()
                .map(|scale| {
                    let action = if selection_only {
                        Action::ExportSelection(scale)
                    } else {
                        Action::ExportPage(scale)
                    };
                    Item::action(
                        match scale {
                            1.0 => "@1x…",
                            2.0 => "@2x…",
                            _ => "@3x…",
                        },
                        action,
                        true,
                    )
                })
                .collect(),
        )
    };
    let mut view = vec![
        Item::action("Find on page…", Action::FindOnPage, has_tab),
        Item::separator(),
        Item::action("Zoom to fit   ⇧1", Action::ZoomFit, has_tab),
        Item::action("Actual size   ⇧0", Action::ActualSize, has_tab),
        Item::separator(),
        Item::checked("Sidebar", Action::ToggleLayers, app.show_left),
        Item::checked("Properties panel", Action::ToggleProperties, app.show_right),
        Item::action("Command bar…", Action::CommandBar, true),
        Item::separator(),
        Item::group(
            "Appearance",
            [Appearance::System, Appearance::Light, Appearance::Dark]
                .into_iter()
                .map(|a| Item::checked(a.label(), Action::Appearance(a), app.appearance == a))
                .collect(),
        ),
        Item::checked("Reduce motion", Action::ReduceMotion, app.reduce_motion),
    ];
    #[cfg(target_arch = "wasm32")]
    view.push(Item::group(
        "CJK font",
        crate::cjk::Variant::ALL
            .into_iter()
            .map(|v| Item::checked(v.label(), Action::CjkFont(v), app.web.font_choice == v))
            .collect(),
    ));
    if cfg!(any(feature = "mcp", target_arch = "wasm32")) {
        view.push(Item::action("Agent access…", Action::AgentAccess, true));
    }
    vec![
        Item::group(
            "File",
            vec![
                Item::action("Open…", Action::Open, true),
                export("Export page as PNG", false),
                export("Export selection as PNG", true),
                Item::separator(),
                Item::action("Close file", Action::CloseTab, has_tab),
                Item::action("Quit", Action::Quit, true),
            ],
        ),
        Item::group("View", view),
        Item::group(
            "Space",
            vec![
                Item::action("Change space icon…", Action::SpaceIcon, has_tab),
                Item::action("Rename space…", Action::RenameSpace, has_tab),
                Item::action("Edit theme colour…", Action::SpaceColour, has_tab),
                Item::separator(),
                Item::action("Reset space look", Action::ResetSpace, has_tab),
            ],
        ),
        Item::group(
            "Help",
            vec![Item::action("About figstudio", Action::About, true)],
        ),
    ]
}

impl App {
    pub(crate) fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open => self.open_dialog(ctx),
            Action::ExportPage(scale) => self.export_png(ctx, false, scale),
            Action::ExportSelection(scale) => self.export_png(ctx, true, scale),
            Action::CloseTab => self.close(self.active),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::ToggleLayers => self.show_left = !self.show_left,
            Action::ToggleProperties => self.show_right = !self.show_right,
            Action::CommandBar => self.command.toggle(ctx),
            Action::SpaceIcon | Action::RenameSpace | Action::SpaceColour => {
                let kind = match action {
                    Action::SpaceIcon => crate::space_editor::Popover::Icon,
                    Action::RenameSpace => crate::space_editor::Popover::Rename,
                    _ => crate::space_editor::Popover::Colour,
                };
                self.space_editing = self
                    .tabs
                    .get(self.active)
                    .map(|tab| crate::space_editor::SpaceEditor::new(tab, kind, ctx));
            }
            Action::ResetSpace => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.style = Default::default();
                    self.spaces
                        .remember(tab.doc_key.as_deref(), &tab.title(), &tab.style);
                }
            }
            Action::FindOnPage => self.find.show(),
            Action::Appearance(appearance) => {
                self.appearance = appearance;
                ctx.set_theme(appearance.preference());
            }
            Action::ReduceMotion => {
                self.reduce_motion = !crate::anim::reduced(ctx);
                crate::anim::set_reduced(ctx, self.reduce_motion);
            }
            Action::SelectTool(tool) => self.tool = tool,
            Action::ZoomFit => {
                if let Some(t) = self.tab() {
                    t.fit_pending = true;
                }
            }
            Action::ActualSize => {
                if let Some(t) = self.tab() {
                    t.zoom_centre(1.0);
                }
            }
            Action::ZoomBy(factor) => {
                if let Some(t) = self.tab() {
                    t.zoom_centre(t.view.zoom * factor);
                }
            }
            Action::ClearSelection => {
                if let Some(t) = self.tab() {
                    t.select(None);
                }
            }
            Action::AgentAccess => self.agent_window = true,
            Action::About => self.about_window = true,
            #[cfg(target_arch = "wasm32")]
            Action::CjkFont(v) => self.web.select_cjk_font(v, ctx),
        }
    }
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
