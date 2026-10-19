//! Native frame integration, inert on platforms with the drawn menu.
use crate::{App, menu::Action};

pub(crate) fn native_owned(macos: bool, attached: bool, action: Action) -> bool {
    macos
        && attached
        && matches!(
            action,
            Action::Open
                | Action::CloseTab
                | Action::ExportSelection(2.0)
                | Action::CommandBar
                | Action::ToggleLayers
                | Action::FindOnPage
                | Action::Settings
        )
}

fn show_drawn_bar(attached: bool) -> bool {
    !attached
}

impl App {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn attach_menu(&mut self, _cc: &eframe::CreationContext<'_>) {
        #[cfg(any(target_os = "macos", windows))]
        match crate::menu_native::NativeMenu::attach(_cc, self) {
            Ok(menu) => self.native_menu = Some(menu),
            Err(error) => tracing::warn!("native menu unavailable: {error}"),
        }
    }
    pub(crate) fn has_native_menu(&self) -> bool {
        #[cfg(any(target_os = "macos", windows))]
        {
            self.native_menu.is_some()
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        false
    }

    pub(crate) fn native_owns(&self, action: Action) -> bool {
        native_owned(cfg!(target_os = "macos"), self.has_native_menu(), action)
    }

    pub(crate) fn show_drawn_menu(&self) -> bool {
        show_drawn_bar(self.has_native_menu())
    }

    pub(crate) fn native_events(&mut self, _ctx: &egui::Context) {
        #[cfg(any(target_os = "macos", windows))]
        {
            let actions = self
                .native_menu
                .as_ref()
                .map(|menu| menu.drain())
                .unwrap_or_default();
            for action in actions {
                // While the command bar is open, the menus only close it or quit.
                if self.command.is_open() && !matches!(action, Action::CommandBar | Action::Quit) {
                    continue;
                }
                self.apply(action, _ctx);
            }
            let tree = crate::menu::tree(self);
            // The menus show the user's shortcuts, and none while one is being recorded.
            let keys = (self.settings.recording().is_none()).then(|| self.prefs.keymap.clone());
            if let Some(menu) = &mut self.native_menu {
                menu.sync(&tree);
                if self.native_keys.as_ref() != Some(&keys) {
                    menu.set_keys(keys.as_ref());
                    self.native_keys = Some(keys);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_command_actions_are_native_owned_on_attached_macos() {
        for action in [
            Action::Open,
            Action::CloseTab,
            Action::ExportSelection(2.0),
            Action::CommandBar,
            Action::ToggleLayers,
            Action::FindOnPage,
            Action::Settings,
        ] {
            assert!(native_owned(true, true, action));
            assert!(
                !native_owned(true, false, action),
                "drawn fallback owns shortcuts"
            );
            assert!(
                !native_owned(false, true, action),
                "Windows keeps egui shortcuts"
            );
        }
        for action in [
            Action::ZoomFit,
            Action::ActualSize,
            Action::ExportSelection(1.0),
        ] {
            assert!(
                !native_owned(true, true, action),
                "egui must retain non-command shortcuts"
            );
        }
    }

    #[test]
    fn attachment_hides_the_drawn_bar_and_failure_keeps_it() {
        assert!(show_drawn_bar(false));
        assert!(!show_drawn_bar(true));
    }
}
