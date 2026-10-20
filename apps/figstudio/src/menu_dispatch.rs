//! Native frame integration, inert on platforms with the drawn menu.
use crate::{App, menu::Action};

/// Whether the menu bar runs a shortcut: it owns the action (`owned`) and has an accelerator
/// for the keys. Keys a menu cannot show (`!`, F13…, Insert) stay with egui, or they would do
/// nothing.
pub(crate) fn menu_runs(owned: bool, keys: Option<egui::KeyboardShortcut>) -> bool {
    #[cfg(any(target_os = "macos", windows))]
    let shown = keys.is_some_and(|k| crate::menu_native::has_code(k.logical_key));
    #[cfg(not(any(target_os = "macos", windows)))]
    let shown = {
        let _ = keys;
        false
    };
    owned && shown
}

pub(crate) fn native_owned(macos: bool, attached: bool, action: Action) -> bool {
    macos
        && attached
        && matches!(
            action,
            Action::Open
                | Action::OpenLink
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

    /// Whether the menu bar runs `action` from `keys` itself (macOS, attached, and a key the
    /// menu can show). Otherwise the egui shortcut handling runs it.
    pub(crate) fn native_takes(
        &self,
        action: Action,
        keys: Option<egui::KeyboardShortcut>,
    ) -> bool {
        menu_runs(self.native_owns(action), keys)
    }

    /// Gives the menu bar the user's shortcuts, or none while one is being recorded (it
    /// would take the keys before the window sees them).
    pub(crate) fn sync_native_keys(&mut self) {
        #[cfg(any(target_os = "macos", windows))]
        {
            let keys = (self.settings.recording().is_none()).then(|| self.prefs.keymap.clone());
            if let Some(menu) = &mut self.native_menu
                && self.native_keys.as_ref() != Some(&keys)
            {
                menu.set_keys(keys.as_ref());
                self.native_keys = Some(keys);
            }
        }
    }

    /// Runs what the menu bar picked, unless something modal takes the keys: while a
    /// shortcut is recorded nothing runs (the chord is the new shortcut, not its old
    /// action); while the command bar is open, only its toggle and Quit.
    #[cfg(any(target_os = "macos", windows, test))]
    pub(crate) fn apply_native(&mut self, actions: Vec<Action>, ctx: &egui::Context) {
        for action in actions {
            if self.settings.recording().is_some() {
                continue;
            }
            if self.command.is_up(ctx) && !matches!(action, Action::CommandBar | Action::Quit) {
                continue;
            }
            self.apply(action, ctx);
        }
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
            self.apply_native(actions, _ctx);
            let tree = crate::menu::tree(self);
            if let Some(menu) = &mut self.native_menu {
                menu.sync(&tree);
            }
            self.sync_native_keys();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_a_menu_cannot_show_stay_with_egui() {
        let k = |m, key| Some(egui::KeyboardShortcut::new(m, key));
        let cmd = egui::Modifiers::COMMAND;
        assert!(!menu_runs(false, k(cmd, egui::Key::O)), "not owned");
        assert!(!menu_runs(true, None), "no keys");
        // Shift+1 is `!` on a US layout; F13 and Insert have no menu key code either.
        for key in [
            egui::Key::Exclamationmark,
            egui::Key::F13,
            egui::Key::Insert,
        ] {
            assert!(!menu_runs(true, k(cmd, key)), "{key:?}");
        }
        if cfg!(any(target_os = "macos", windows)) {
            assert!(menu_runs(true, k(cmd, egui::Key::O)));
        }
    }

    #[test]
    fn only_command_actions_are_native_owned_on_attached_macos() {
        for action in [
            Action::Open,
            Action::OpenLink,
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
