//! Native frame integration, inert on platforms with the drawn menu.
use crate::App;

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

    pub(crate) fn native_shortcuts(&self) -> bool {
        cfg!(target_os = "macos") && self.has_native_menu()
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
                self.apply(action, _ctx);
            }
            let tree = crate::menu::tree(self);
            if let Some(menu) = &mut self.native_menu {
                menu.sync(&tree);
            }
        }
    }
}
