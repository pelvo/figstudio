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
            &mut entries,
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
        for mut group in crate::menu::tree(app) {
            group
                .children
                .retain(|i| !matches!(i.action, Some(Action::Quit | Action::About)));
            let submenu = Submenu::new(group.label, true);
            for item in &group.children {
                add(&submenu, item, &mut entries)?;
            }
            menu.append(&submenu)?;
            if group.label == "Help" {
                submenu.set_as_help_menu_for_nsapp();
            }
        }
        let window = Submenu::new("Window", true);
        window.append(&PredefinedMenuItem::minimize(None))?;
        window.append(&PredefinedMenuItem::maximize(None))?;
        menu.insert(&window, 3)?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
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
            if !state.label.is_empty() {
                if let Some(entry) = self.items.iter_mut().find(|entry| {
                    if state.action.is_some() {
                        entry.action == state.action
                    } else {
                        matches!(&entry.kind, Kind::Group(g) if g.text() == state.label)
                    }
                }) {
                    entry.sync(state);
                }
                self.sync(&state.children);
            }
        }
    }
}

fn add(parent: &Submenu, item: &Item, entries: &mut Vec<Entry>) -> muda::Result<()> {
    if item.label.is_empty() {
        return parent.append(&PredefinedMenuItem::separator());
    }
    let kind = if let Some(action) = item.action {
        let accelerator = accelerator(action);
        // The egui labels include textual hints; native menus render their own accelerators.
        let label = item.label.split("   ").next().unwrap_or(item.label);
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
        mods |= Modifiers::META;
    }
    if shortcut.modifiers.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(mods, code))
}
