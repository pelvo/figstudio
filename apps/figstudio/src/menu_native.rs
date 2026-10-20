//! Main-thread muda menu; the callback only queues ids and wakes the egui frame.
use crate::{
    App,
    menu::{Action, Item},
};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use std::sync::mpsc::{self, Receiver};

pub(crate) struct NativeMenu {
    _menu: Menu,
    items: Vec<Entry>,
    events: Receiver<MenuId>,
}

struct Entry {
    label: &'static str,
    action: Option<Action>,
    kind: Kind,
    enabled: bool,
    checked: Option<bool>,
}

enum Kind {
    Item(MenuItem),
    Check(CheckMenuItem),
    Group(Submenu),
}

impl Entry {
    fn sync(&mut self, state: &Item) {
        if self.enabled != state.enabled {
            match &self.kind {
                Kind::Item(i) => i.set_enabled(state.enabled),
                Kind::Check(i) => i.set_enabled(state.enabled),
                Kind::Group(i) => i.set_enabled(state.enabled),
            }
            self.enabled = state.enabled;
        }
        if self.checked != state.checked {
            if let (Kind::Check(i), Some(checked)) = (&self.kind, state.checked) {
                i.set_checked(checked);
            }
            self.checked = state.checked;
        }
    }
    fn id(&self) -> &MenuId {
        match &self.kind {
            Kind::Item(i) => i.id(),
            Kind::Check(i) => i.id(),
            Kind::Group(i) => i.id(),
        }
    }
}

impl NativeMenu {
    pub(crate) fn attach(cc: &eframe::CreationContext<'_>, app: &App) -> anyhow::Result<Self> {
        let menu = Menu::new();
        let mut entries = vec![];
        #[cfg(target_os = "macos")]
        macos_app_menu(&menu, &mut entries, &app.prefs.keymap)?;
        #[cfg(target_os = "macos")]
        let mut help = None;
        for group in crate::menu::tree(app) {
            #[cfg(target_os = "macos")]
            let group = {
                let mut group = group;
                // The app menu has these on macOS.
                group
                    .children
                    .retain(|i| !matches!(i.action, Some(Action::Quit | Action::Settings)));
                group
            };
            let submenu = Submenu::new(group.label, true);
            for item in &group.children {
                add(&submenu, item, &mut entries, Some(&app.prefs.keymap))?;
            }
            menu.append(&submenu)?;
            #[cfg(target_os = "macos")]
            if group.label == "Help" {
                help = Some(submenu);
            }
        }
        let window = Submenu::new("Window", true);
        window.append(&PredefinedMenuItem::minimize(None))?;
        window.append(&PredefinedMenuItem::maximize(None))?;
        menu.insert(&window, if cfg!(target_os = "macos") { 3 } else { 2 })?;
        #[cfg(target_os = "macos")]
        {
            menu.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();
            if let Some(help) = help {
                help.set_as_help_menu_for_nsapp();
            }
        }
        #[cfg(windows)]
        attach_windows(&menu, cc)?;
        let (sender, events) = mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = sender.send(event.id);
            ctx.request_repaint();
        }));
        Ok(Self {
            _menu: menu,
            items: entries,
            events,
        })
    }

    pub(crate) fn drain(&self) -> Vec<Action> {
        self.events
            .try_iter()
            .filter_map(|id| {
                self.items
                    .iter()
                    .find(|i| *i.id() == id)
                    .and_then(|i| i.action)
            })
            .collect()
    }

    /// Shows `keys` (the user's shortcuts) as the menu's accelerators; `None` takes them all
    /// off while a new shortcut is being recorded.
    pub(crate) fn set_keys(&mut self, keys: Option<&crate::keymap::Keymap>) {
        for entry in &self.items {
            let Some(action) = entry.action else { continue };
            let accelerator = accelerator(action, keys);
            let label = crate::menu_dispatch::native_label(
                cfg!(target_os = "macos"),
                action,
                entry.label,
                keys.and_then(|keys| keys.for_action(action)),
                accelerator.is_some(),
            );
            let set = match &entry.kind {
                Kind::Item(i) => {
                    i.set_text(&label);
                    i.set_accelerator(accelerator)
                }
                Kind::Check(i) => {
                    i.set_text(&label);
                    i.set_accelerator(accelerator)
                }
                Kind::Group(_) => Ok(()),
            };
            if let Err(error) = set {
                tracing::warn!("menu shortcut not set: {error}");
            }
        }
    }

    pub(crate) fn sync(&mut self, tree: &[Item]) {
        for state in tree {
            if state.label.is_empty() {
                continue;
            }
            let entry = self.items.iter_mut().find(|entry| {
                if state.action.is_some() {
                    entry.action == state.action
                } else {
                    matches!(&entry.kind, Kind::Group(g) if g.text() == state.label)
                }
            });
            if let Some(entry) = entry {
                entry.sync(state);
            }
            self.sync(&state.children);
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_app_menu(
    menu: &Menu,
    entries: &mut Vec<Entry>,
    keys: &crate::keymap::Keymap,
) -> anyhow::Result<()> {
    let app_menu = Submenu::new("figstudio", true);
    add(
        &app_menu,
        &Item {
            label: "About figstudio",
            action: Some(Action::About),
            enabled: true,
            checked: None,
            children: vec![],
        },
        entries,
        Some(keys),
    )?;
    app_menu.append(&PredefinedMenuItem::separator())?;
    add(
        &app_menu,
        &Item {
            label: "Settings…",
            action: Some(Action::Settings),
            enabled: true,
            checked: None,
            children: vec![],
        },
        entries,
        Some(keys),
    )?;
    app_menu.append(&PredefinedMenuItem::separator())?;
    app_menu.append(&PredefinedMenuItem::hide(None))?;
    app_menu.append(&PredefinedMenuItem::hide_others(None))?;
    app_menu.append(&PredefinedMenuItem::show_all(None))?;
    app_menu.append(&PredefinedMenuItem::separator())?;
    let quit = MenuItem::new("Quit figstudio", true, Some("Cmd+Q".parse()?));
    app_menu.append(&quit)?;
    entries.push(Entry {
        label: "Quit figstudio",
        action: Some(Action::Quit),
        kind: Kind::Item(quit),
        enabled: true,
        checked: None,
    });
    menu.append(&app_menu)?;
    Ok(())
}

#[cfg(windows)]
#[expect(
    unsafe_code,
    reason = "muda requires the live eframe HWND to attach its menu"
)]
fn attach_windows(menu: &Menu, cc: &eframe::CreationContext<'_>) -> anyhow::Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::Win32(handle) = cc.window_handle()?.as_raw() else {
        anyhow::bail!("native menu requires a Win32 window");
    };
    // SAFETY: eframe supplies its live HWND; attachment occurs on its UI thread and
    // the retained Menu is dropped with App. muda handles WM_NCDESTROY cleanup.
    unsafe {
        menu.init_for_hwnd(handle.hwnd.get())?;
    }
    Ok(())
}

fn add(
    parent: &Submenu,
    item: &Item,
    entries: &mut Vec<Entry>,
    keys: Option<&crate::keymap::Keymap>,
) -> muda::Result<()> {
    if item.label.is_empty() {
        return parent.append(&PredefinedMenuItem::separator());
    }
    let kind = if let Some(action) = item.action {
        let accelerator = accelerator(action, keys);
        let label = crate::menu_dispatch::native_label(
            cfg!(target_os = "macos"),
            action,
            item.label,
            keys.and_then(|keys| keys.for_action(action)),
            accelerator.is_some(),
        );
        if let Some(checked) = item.checked {
            let i = CheckMenuItem::new(label, item.enabled, checked, accelerator);
            parent.append(&i)?;
            Kind::Check(i)
        } else {
            let i = MenuItem::new(label, item.enabled, accelerator);
            parent.append(&i)?;
            Kind::Item(i)
        }
    } else {
        let group = Submenu::new(item.label, item.enabled);
        for child in &item.children {
            add(&group, child, entries, keys)?;
        }
        parent.append(&group)?;
        Kind::Group(group)
    };
    entries.push(Entry {
        label: item.label,
        action: item.action,
        kind,
        enabled: item.enabled,
        checked: item.checked,
    });
    Ok(())
}

/// The native accelerator for `action` with the user's `keys` (`None` while a shortcut is
/// being recorded, so the menu does not swallow it).
pub(crate) fn accelerator(
    action: Action,
    keys: Option<&crate::keymap::Keymap>,
) -> Option<muda::accelerator::Accelerator> {
    use muda::accelerator::{Accelerator, Modifiers};
    // Field shortcuts must reach egui; native events cannot distinguish clicks from keys.
    if crate::menu_dispatch::hint_only(action)
        || (cfg!(target_os = "macos") && !crate::menu_dispatch::native_owned(true, true, action))
    {
        return None;
    }
    let shortcut = keys?.for_action(action)?;
    let code = code(shortcut.logical_key)?;
    let m = shortcut.modifiers;
    let mut mods = Modifiers::empty();
    if m.command || m.mac_cmd {
        mods |= if cfg!(target_os = "macos") {
            Modifiers::META
        } else {
            Modifiers::CONTROL
        };
    }
    // On a Mac, Control is its own key next to ⌘; elsewhere `ctrl` is the command key.
    if m.ctrl && (cfg!(target_os = "macos") || !m.command) {
        mods |= Modifiers::CONTROL;
    }
    if m.alt {
        mods |= Modifiers::ALT;
    }
    if m.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(mods, code))
}

/// Whether a menu accelerator can use `key` (else egui runs that shortcut).
pub(crate) fn has_code(key: egui::Key) -> bool {
    code(key).is_some()
}

/// The key code of a key that a menu accelerator can use.
fn code(key: egui::Key) -> Option<muda::accelerator::Code> {
    use egui::Key as K;
    use muda::accelerator::Code as C;
    Some(match key {
        K::A => C::KeyA,
        K::B => C::KeyB,
        K::C => C::KeyC,
        K::D => C::KeyD,
        K::E => C::KeyE,
        K::F => C::KeyF,
        K::G => C::KeyG,
        K::H => C::KeyH,
        K::I => C::KeyI,
        K::J => C::KeyJ,
        K::K => C::KeyK,
        K::L => C::KeyL,
        K::M => C::KeyM,
        K::N => C::KeyN,
        K::O => C::KeyO,
        K::P => C::KeyP,
        K::Q => C::KeyQ,
        K::R => C::KeyR,
        K::S => C::KeyS,
        K::T => C::KeyT,
        K::U => C::KeyU,
        K::V => C::KeyV,
        K::W => C::KeyW,
        K::X => C::KeyX,
        K::Y => C::KeyY,
        K::Z => C::KeyZ,
        K::Num0 => C::Digit0,
        K::Num1 => C::Digit1,
        K::Num2 => C::Digit2,
        K::Num3 => C::Digit3,
        K::Num4 => C::Digit4,
        K::Num5 => C::Digit5,
        K::Num6 => C::Digit6,
        K::Num7 => C::Digit7,
        K::Num8 => C::Digit8,
        K::Num9 => C::Digit9,
        K::F1 => C::F1,
        K::F2 => C::F2,
        K::F3 => C::F3,
        K::F4 => C::F4,
        K::F5 => C::F5,
        K::F6 => C::F6,
        K::F7 => C::F7,
        K::F8 => C::F8,
        K::F9 => C::F9,
        K::F10 => C::F10,
        K::F11 => C::F11,
        K::F12 => C::F12,
        K::Comma => C::Comma,
        K::Period => C::Period,
        K::Slash => C::Slash,
        K::Backslash => C::Backslash,
        K::Semicolon => C::Semicolon,
        K::Quote => C::Quote,
        K::Minus => C::Minus,
        K::Equals => C::Equal,
        K::OpenBracket => C::BracketLeft,
        K::CloseBracket => C::BracketRight,
        K::Backtick => C::Backquote,
        K::Space => C::Space,
        K::Enter => C::Enter,
        K::Tab => C::Tab,
        K::Escape => C::Escape,
        K::Backspace => C::Backspace,
        K::Delete => C::Delete,
        K::Home => C::Home,
        K::End => C::End,
        K::PageUp => C::PageUp,
        K::PageDown => C::PageDown,
        K::ArrowUp => C::ArrowUp,
        K::ArrowDown => C::ArrowDown,
        K::ArrowLeft => C::ArrowLeft,
        K::ArrowRight => C::ArrowRight,
        _ => return None,
    })
}
