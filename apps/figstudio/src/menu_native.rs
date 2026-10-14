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
        macos_app_menu(&menu, &mut entries)?;
        #[cfg(target_os = "macos")]
        let mut help = None;
        for group in crate::menu::tree(app) {
            #[cfg(target_os = "macos")]
            let group = {
                let mut group = group;
                group
                    .children
                    .retain(|i| !matches!(i.action, Some(Action::Quit)));
                group
            };
            let submenu = Submenu::new(group.label, true);
            for item in &group.children {
                add(&submenu, item, &mut entries)?;
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
fn macos_app_menu(menu: &Menu, entries: &mut Vec<Entry>) -> anyhow::Result<()> {
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
    )?;
    app_menu.append(&PredefinedMenuItem::separator())?;
    app_menu.append(&PredefinedMenuItem::hide(None))?;
    app_menu.append(&PredefinedMenuItem::hide_others(None))?;
    app_menu.append(&PredefinedMenuItem::show_all(None))?;
    app_menu.append(&PredefinedMenuItem::separator())?;
    let quit = MenuItem::new("Quit figstudio", true, Some("Cmd+Q".parse()?));
    app_menu.append(&quit)?;
    entries.push(Entry {
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

fn add(parent: &Submenu, item: &Item, entries: &mut Vec<Entry>) -> muda::Result<()> {
    if item.label.is_empty() {
        return parent.append(&PredefinedMenuItem::separator());
    }
    let kind = if let Some(action) = item.action {
        let accelerator = accelerator(action);
        // The egui labels include textual hints; native menus render their own accelerators.
        let label = if accelerator.is_some() {
            item.label.split("   ").next().unwrap_or(item.label)
        } else {
            item.label
        };
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
            add(&group, child, entries)?;
        }
        parent.append(&group)?;
        Kind::Group(group)
    };
    entries.push(Entry {
        action: item.action,
        kind,
        enabled: item.enabled,
        checked: item.checked,
    });
    Ok(())
}

fn accelerator(action: Action) -> Option<muda::accelerator::Accelerator> {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    if cfg!(target_os = "macos") && !crate::menu_dispatch::native_owned(true, true, action) {
        return None;
    }
    let shortcut = action.shortcut()?;
    let code = match shortcut.logical_key {
        egui::Key::O => Code::KeyO,
        egui::Key::W => Code::KeyW,
        egui::Key::E => Code::KeyE,
        egui::Key::Num1 => Code::Digit1,
        egui::Key::Num0 => Code::Digit0,
        _ => return None,
    };
    let mut mods = Modifiers::empty();
    if shortcut.modifiers.command {
        mods |= if cfg!(target_os = "macos") {
            Modifiers::META
        } else {
            Modifiers::CONTROL
        };
    }
    if shortcut.modifiers.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(mods, code))
}
