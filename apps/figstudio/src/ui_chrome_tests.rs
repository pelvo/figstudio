//! The Arc-style chrome through the real app: the sidebar and its title strip, the file
//! switcher, the tool pill and the command bar.
use egui::accesskit::Role;
use egui::{Key, KeyboardShortcut, Modifiers};

use super::*;

#[test]
fn hiding_the_sidebar_leaves_a_title_strip_whose_button_brings_it_back() {
    let mut h = harness();
    assert!(
        h.query_by_label("Page 1").is_some(),
        "the page list is in the sidebar"
    );
    h.get_by_label("Hide sidebar").click();
    h.run_steps(3);
    assert!(!h.state().show_left);
    assert!(
        h.query_by_label("Page 1").is_none(),
        "the page list went with it"
    );
    h.get_by_label("Show sidebar").click();
    h.run_steps(3);
    assert!(h.state().show_left);
    assert!(h.query_by_label("Page 1").is_some());
}

#[test]
fn each_open_file_gets_its_own_colour_and_its_dot_switches_to_it() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.run_steps(3);
    let tints: Vec<usize> = h.state().tabs.iter().map(|t| t.tint).collect();
    assert_ne!(tints[0], tints[1], "each file its own colour");
    let first = h.state().tabs[0].title();
    let other = (h.state().active + 1) % 2;
    let title = h.state().tabs[other].title();
    h.get_by_role_and_label(Role::Button, &title).click();
    h.run_steps(3);
    assert_eq!(h.state().active, other, "{first} / {title}");
}

#[test]
fn the_tool_pill_picks_tools_and_zooms() {
    let mut h = harness();
    h.get_by_label("Zoom tool").click();
    h.run_steps(3);
    assert_eq!(h.state().tool, crate::canvas::Tool::Zoom);
    let before = h.state().tabs[0].view.zoom;
    h.get_by_label("Zoom in").click();
    h.run_steps(3);
    let zoomed = h.state().tabs[0].view.zoom;
    assert!(
        (zoomed / before - 1.25).abs() < 1e-3,
        "{before} -> {zoomed}"
    );
    h.get_by_label("Zoom to fit").click();
    h.run_steps(3);
    assert!(
        (h.state().tabs[0].view.zoom - before).abs() < 1e-3,
        "fits again"
    );
}

#[test]
fn the_command_bar_opens_with_its_shortcut_and_runs_what_is_picked() {
    let mut h = harness();
    let fitted = h.state().tabs[0].view.zoom;
    h.state_mut().tabs[0].view.zoom = fitted * 4.0;
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    assert!(h.state().command.is_open());
    h.event(egui::Event::Text("zoom to fit".into()));
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(!h.state().command.is_open(), "closes after running");
    assert!(
        (h.state().tabs[0].view.zoom - fitted).abs() < 1e-3,
        "zoomed to fit"
    );

    h.get_by_label_contains("Command bar (").click();
    h.run_steps(3);
    assert!(
        h.state().command.is_open(),
        "the sidebar field opens it too"
    );
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().command.is_open());
    assert_eq!(h.state().tabs[0].selection, None);
}

#[test]
fn reduce_motion_and_appearance_apply_to_the_whole_ui() {
    use crate::menu::{Action, Appearance};
    let mut h = harness();
    let ctx = h.ctx.clone();
    assert!(
        crate::anim::reduced(&ctx),
        "the harness starts without motion"
    );
    h.state_mut().apply(Action::ReduceMotion, &ctx);
    h.state_mut().apply(Action::ReduceMotion, &ctx);
    assert!(crate::anim::reduced(&ctx), "toggled twice: still off");
    h.state_mut().apply(Action::ReduceMotion, &ctx);
    assert!(!crate::anim::reduced(&ctx));
    h.state_mut()
        .apply(Action::Appearance(Appearance::Light), &ctx);
    h.run_steps(2);
    assert_eq!(ctx.theme(), egui::Theme::Light);
    h.state_mut()
        .apply(Action::Appearance(Appearance::Dark), &ctx);
    h.run_steps(2);
    assert_eq!(ctx.theme(), egui::Theme::Dark);
}

#[test]
fn a_space_dot_menu_changes_colour_icon_and_name_and_saves_them_for_the_file() {
    use crate::space_editor::Popover;
    use crate::spaces::{Style, Symbol};
    use crate::theme::Colour;
    let mut h = harness();
    let open = |h: &mut Harness<'_, App>, item: &str| {
        let name = h.state().tabs[0].space_name();
        h.get_by_role_and_label(Role::Button, &name)
            .click_secondary();
        h.run_steps(3);
        h.get_by_label(item).click();
        h.run_steps(3);
    };
    let kind = |h: &Harness<'_, App>| h.state().space_editing.as_ref().map(|e| e.kind);
    let saved = |h: &Harness<'_, App>| {
        let tab = &h.state().tabs[0];
        h.state()
            .spaces
            .lookup(crate::spaces::FileId::of(tab, &tab.title()))
    };

    open(&mut h, "Edit theme colour…");
    assert_eq!(kind(&h), Some(Popover::Colour));
    h.get_by_label("Rose").click();
    h.run_steps(2);
    assert_eq!(
        h.state().tabs[0].colour(),
        Colour::Preset(4),
        "the window follows at once"
    );
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(kind(&h), None, "Escape closes it");

    open(&mut h, "Change space icon…");
    assert_eq!(kind(&h), Some(Popover::Icon));
    h.get_by_label("🚀").click();
    h.run_steps(2);
    assert_eq!(kind(&h), None, "picking an icon closes it");

    open(&mut h, "Rename space…");
    assert_eq!(kind(&h), Some(Popover::Rename));
    h.event(egui::Event::Text("Launch".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(kind(&h), None, "Enter keeps the name");
    assert_eq!(
        saved(&h),
        Some(Style {
            colour: Some(Colour::Preset(4)),
            symbol: Some(Symbol::Emoji("🚀".into())),
            name: Some("Launch".into()),
        })
    );
    assert!(
        h.query_by_role_and_label(Role::Button, "Launch").is_some(),
        "the dot is renamed"
    );

    open(&mut h, "Rename space…");
    h.event(egui::Event::Text(" and more".into()));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].space_name(), "Launch", "Escape goes back");

    open(&mut h, "Reset space look");
    assert_eq!(saved(&h), Some(Style::default()), "the reset is kept");
    assert_eq!(h.state().tabs[0].style, Style::default());

    // A reset while renaming closes the popover, so its Escape cannot bring the old name
    // back.
    open(&mut h, "Rename space…");
    h.event(egui::Event::Text("Draft".into()));
    h.run_steps(2);
    let ctx = h.ctx.clone();
    h.state_mut().apply(crate::menu::Action::ResetSpace, &ctx);
    h.run_steps(2);
    assert_eq!(kind(&h), None);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].style, Style::default());
    assert_eq!(saved(&h), Some(Style::default()));
}

#[test]
fn command_f_finds_text_on_the_page_and_steps_through_it() {
    let mut h = harness();
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(3);
    assert!(h.state().find.is_open());
    h.event(egui::Event::Text("abc".into()));
    h.run_steps(3);
    assert_eq!(
        h.state().tabs[0].find_hits.len(),
        2,
        "the layer and the instance"
    );
    assert_eq!(h.state().tabs[0].find_current, Some(0));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state().tabs[0].find_current,
        Some(1),
        "Enter: the next match"
    );
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::G);
    h.run_steps(3);
    assert_eq!(
        h.state().tabs[0].find_current,
        Some(0),
        "⇧⌘G: the previous one"
    );
    assert!(h.query_by_label_contains("1 of 2").is_some());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().find.is_open(), "Escape closes the bar");
    assert!(h.state().tabs[0].find_hits.is_empty(), "and the highlights");

    // After a click on the canvas the field has no focus; Escape still closes the bar
    // (before it clears the selection), and ⌘F does not get past an open command bar.
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(3);
    let canvas = h.state().tabs[0].canvas_rect.expect("laid out");
    let empty = canvas.left_bottom() + egui::vec2(20.0, -60.0);
    h.event(Event::PointerMoved(empty));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: empty,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run_steps(3);
    assert!(!h.ctx.egui_wants_keyboard_input(), "the field lost focus");
    assert!(h.state().find.is_open());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().find.is_open(), "Escape closes it all the same");
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(3);
    assert!(!h.state().find.is_open(), "the command bar is modal");
}

#[test]
fn a_file_opens_with_the_style_saved_for_it() {
    use crate::spaces::{Style, Symbol};
    use crate::theme::Colour;
    let mut h = harness();
    let saved = Style {
        colour: Some(Colour::Custom([10, 120, 250])),
        symbol: Some(Symbol::Icon("component".into())),
        name: Some("Icons".into()),
    };
    let folder = crate::spaces::folder_of(&fixture("vector.fig"));
    let id = crate::spaces::FileId {
        key: None,
        file: "vector.fig",
        folder: folder.as_deref(),
    };
    h.state_mut().spaces.remember(id, &saved);
    h.state_mut().open(fixture("vector.fig"));
    wait(&mut h, &mut Vec::new(), "the second file", |a| {
        a.tabs.len() == 2
    });
    assert_eq!(h.state().tabs[1].style, saved);
    assert_eq!(
        h.state().tabs[0].style,
        Style::default(),
        "others keep theirs"
    );
}

#[test]
fn workspace_malformed_legacy_payload_cannot_reset_other_preferences() {
    let mut memory = Memory::default();
    memory.0.insert(crate::UI_STATE_KEY.to_owned(), "(show_left:false,show_right:true,pages_share:0.3,prefs:(renderer:Cpu,frame_labels:false),workspace:\"invalid\")".to_owned());
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&memory);
    let app = App::new(&cc, Vec::new());
    assert!(
        !app.show_left && !app.prefs.frame_labels,
        "workspace decoding must not discard unrelated settings"
    );
    assert_eq!(app.prefs.renderer, crate::settings::Renderer::Cpu);
}

#[test]
fn workspace_paths_are_not_written_into_eframe_preferences() {
    use eframe::App as _;
    let mut h = harness();
    let mut memory = Memory::default();
    h.state_mut().save(&mut memory);
    assert!(
        memory
            .0
            .values()
            .all(|value| !value.contains("structure.fig")),
        "file paths must not enter the non-private preferences store"
    );
}

#[test]
fn workspace_startup_bounds_decode_concurrency_and_retains_the_queue() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let paths: Vec<_> = (0..5)
        .map(|i| {
            let path = dir.path().join(format!("copy-{i}.fig"));
            std::fs::copy(fixture("structure.fig"), &path).expect("public fixture");
            path
        })
        .collect();
    let mut h = Harness::builder().with_size(SIZE).build_eframe(|cc| {
        let app = App::new(cc, paths);
        assert!(
            app.loading.len() <= 4,
            "startup must not spawn one decoder for every saved path"
        );
        let snapshot =
            serde_json::to_value(crate::session::Session::capture(&app)).expect("snapshot");
        assert_eq!(
            snapshot["files"].as_array().expect("files").len(),
            5,
            "queued restores survive a save before decoding finishes"
        );
        app
    });
    wait(&mut h, &mut Vec::new(), "all queued files", |a| {
        a.tabs.len() == 5 && a.loading.len() == 0
    });
}

#[test]
fn workspace_resolved_parent_aliases_open_only_one_space() {
    let dir = tempfile::tempdir().expect("workspace folder");
    std::fs::create_dir(dir.path().join("nested")).expect("nested folder");
    let path = dir.path().join("design.fig");
    std::fs::copy(fixture("structure.fig"), &path).expect("public fixture");
    let alias = dir.path().join("nested").join("..").join("design.fig");
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![path, alias]));
    wait(&mut h, &mut Vec::new(), "alias loads", |a| {
        a.loading.len() == 0
    });
    assert_eq!(
        h.state().tabs.len(),
        1,
        "worker-resolved identity prevents duplicate spaces"
    );
}

#[cfg(unix)]
#[test]
fn workspace_alias_deduplication_preserves_symlink_parent_semantics() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let real = dir.path().join("real");
    let inner = real.join("inner");
    std::fs::create_dir_all(&inner).expect("real folder");
    let path = real.join("design.fig");
    std::fs::copy(fixture("structure.fig"), &path).expect("public fixture");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&inner, &link).expect("directory symlink");
    let alias = link.join("..").join("design.fig");
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![path, alias]));
    wait(&mut h, &mut Vec::new(), "symlink alias loads", |a| {
        a.loading.len() == 0
    });
    assert_eq!(h.state().tabs.len(), 1);
    assert!(
        !h.state().toasts.iter().any(|t| t.error),
        "do not lexically resolve a symlink's parent to the wrong file"
    );
}

#[test]
fn desktop_workspace_restores_order_active_space_and_pending_files() {
    use crate::spaces::{Style, Symbol};
    use eframe::App as _;

    let dir = tempfile::tempdir().expect("private workspace folder");
    let workspace = dir.path().join("workspace.json");
    let mut first = Harness::builder().with_size(SIZE).build_eframe(|cc| {
        App::with_workspace_store(
            cc,
            vec![fixture("structure.fig"), fixture("vector.fig")],
            crate::session_store::Store::at(workspace.clone()),
        )
    });
    wait(&mut first, &mut Vec::new(), "both workspace files", |a| {
        a.tabs.len() == 2
    });
    // Use a chosen order, not whichever decode happened to finish first.
    first.state_mut().tabs.sort_by(|a, b| b.path.cmp(&a.path));
    first.state_mut().active = 0;
    let expected: Vec<_> = first.state().tabs.iter().map(|t| t.path.clone()).collect();
    let style = Style {
        name: Some("Workspace".to_owned()),
        symbol: Some(Symbol::Icon("component".to_owned())),
        ..Default::default()
    };
    let id = crate::spaces::FileId::of(&first.state().tabs[0], "vector.fig");
    let key = id.key.map(str::to_owned);
    let folder = id.folder.map(str::to_owned);
    first.state_mut().spaces.remember(
        crate::spaces::FileId {
            key: key.as_deref(),
            file: "vector.fig",
            folder: folder.as_deref(),
        },
        &style,
    );
    let mut saved = Memory::default();
    first.state_mut().save(&mut saved);

    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&saved);
    let mut interrupted = App::with_workspace_store(
        &cc,
        Vec::new(),
        crate::session_store::Store::at(workspace.clone()),
    );
    assert_eq!(
        interrupted.loading.len(),
        2,
        "restore starts without opening files manually"
    );
    let mut pending = Memory::default();
    interrupted.save(&mut pending);

    let spaces = std::mem::take(&mut first.state_mut().spaces);
    let mut reopened = Harness::builder().with_size(SIZE).build_eframe(|cc| {
        let mut saved_cc = eframe::CreationContext::_new_kittest(cc.egui_ctx.clone());
        saved_cc.storage = Some(&pending);
        let mut app = App::with_workspace_store(
            &saved_cc,
            Vec::new(),
            crate::session_store::Store::at(workspace.clone()),
        );
        app.spaces = spaces;
        app
    });
    wait(&mut reopened, &mut Vec::new(), "restored workspace", |a| {
        a.tabs.len() == 2 && a.loading.len() == 0
    });
    assert_eq!(
        reopened
            .state()
            .tabs
            .iter()
            .map(|t| t.path.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        reopened.state().tabs[reopened.state().active].path,
        expected[0]
    );
    assert_eq!(
        reopened.state().tabs[0].style,
        style,
        "existing space styling is reused"
    );
}

#[test]
fn desktop_workspace_skips_missing_files_merges_startup_files_and_forgets_closed_spaces() {
    use eframe::App as _;
    let dir = tempfile::tempdir().expect("workspace folder");
    let missing = dir.path().join("missing.fig");
    let kept = dir.path().join("kept.fig");
    let workspace = dir.path().join("workspace.json");
    for path in [&missing, &kept] {
        std::fs::copy(fixture("structure.fig"), path).expect("public fixture copy");
    }
    let mut first = Harness::builder().with_size(SIZE).build_eframe(|cc| {
        App::with_workspace_store(
            cc,
            vec![missing.clone(), kept.clone()],
            crate::session_store::Store::at(workspace.clone()),
        )
    });
    wait(&mut first, &mut Vec::new(), "saved files", |a| {
        a.tabs.len() == 2
    });
    let mut saved = Memory::default();
    first.state_mut().save(&mut saved);
    std::fs::remove_file(&missing).expect("remove one saved file");
    let extra = fixture("vector.fig");
    let mut reopened = Harness::builder().with_size(SIZE).build_eframe(|cc| {
        let mut saved_cc = eframe::CreationContext::_new_kittest(cc.egui_ctx.clone());
        saved_cc.storage = Some(&saved);
        let app = App::with_workspace_store(
            &saved_cc,
            vec![kept.clone(), extra.clone(), extra.clone()],
            crate::session_store::Store::at(workspace.clone()),
        );
        assert_eq!(
            app.loading.len(),
            3,
            "saved files plus one unique explicit file"
        );
        app
    });
    wait(&mut reopened, &mut Vec::new(), "surviving files", |a| {
        a.tabs.len() == 2 && a.loading.len() == 0
    });
    assert_eq!(
        reopened
            .state()
            .tabs
            .iter()
            .map(|t| &t.path)
            .collect::<Vec<_>>(),
        [&kept, &extra]
    );
    assert_eq!(
        reopened.state().tabs[reopened.state().active].path,
        extra,
        "explicit file takes focus"
    );
    assert!(
        reopened
            .state()
            .toasts
            .iter()
            .any(|t| t.error && t.text.contains("Could not open"))
    );
    reopened.state_mut().close(0);
    let mut closed = Memory::default();
    reopened.state_mut().save(&mut closed);
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&closed);
    let only_open = App::with_workspace_store(
        &cc,
        Vec::new(),
        crate::session_store::Store::at(workspace.clone()),
    );
    assert_eq!(only_open.loading.iter().collect::<Vec<_>>(), [&extra]);

    reopened.state_mut().close(0);
    reopened.state_mut().save(&mut closed);
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&closed);
    assert_eq!(
        App::with_workspace_store(&cc, Vec::new(), crate::session_store::Store::at(workspace))
            .loading
            .len(),
        0,
        "closing every space saves an empty workspace"
    );
}

#[cfg(feature = "mcp")]
#[test]
fn desktop_agent_access_preferences_survive_restart() {
    use eframe::App as _;
    for (on, remote) in [(true, false), (true, true), (false, true), (false, false)] {
        let mut storage = Memory::default();
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut first = App::new(&cc, Vec::new());
        let access = first.mcp.access().expect("desktop agent settings");
        *access.on = on;
        *access.remote = remote;
        *access.remote_addr = "127.0.0.1:0".to_owned();
        first.save(&mut storage);
        let saved = &storage.0[crate::UI_STATE_KEY];
        assert!(
            !saved.contains("token"),
            "bearer tokens are not preferences"
        );
        assert!(
            !saved.contains("endpoint"),
            "bound ports are not preferences"
        );

        let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        cc.storage = Some(&storage);
        let mut second = App::new(&cc, Vec::new());
        let access = second
            .mcp
            .access()
            .expect("restored desktop agent settings");
        assert_eq!((*access.on, *access.remote), (on, remote));
        assert_eq!(access.remote_addr, "127.0.0.1:0");
        assert!(access.endpoint.is_none(), "only sync starts the server");
        if on && remote {
            second.mcp.sync(&cc.egui_ctx, &[], 0);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                assert_eq!(second.mcp.frame(&mut [], &mut 0), None);
                let access = second.mcp.access().expect("agent settings");
                if let Some((endpoint, _)) = access.endpoint
                    && endpoint != "Starting…"
                {
                    assert!(endpoint.starts_with("127.0.0.1:"), "{endpoint}");
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "server did not restart"
                );
                std::thread::yield_now();
            }
            second.mcp.stop();
        }
    }
}

#[cfg(feature = "mcp")]
#[test]
fn desktop_agent_access_defaults_off_for_older_saved_preferences() {
    let mut storage = Memory::default();
    storage.0.insert(
        crate::UI_STATE_KEY.to_owned(),
        "(show_left:false,show_right:true,pages_share:0.3,prefs:(renderer:Cpu))".to_owned(),
    );
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&storage);
    let mut app = App::new(&cc, Vec::new());
    assert!(!app.show_left, "the older state still loads");
    assert_eq!(app.prefs.renderer, crate::settings::Renderer::Cpu);
    let access = app.mcp.access().expect("desktop agent settings");
    assert!(!*access.on && !*access.remote);
    assert!(access.remote_addr.is_empty() && access.endpoint.is_none());
}

#[test]
fn the_sidebar_and_panels_come_back_as_they_were_left() {
    use eframe::App as _;
    let mut storage = Memory::default();
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut first = App::new(&cc, Vec::new());
    assert!(
        first.show_left && first.show_right,
        "shown on a first launch"
    );
    first.show_left = false;
    first.pages_share = 0.3;
    first.prefs.renderer = crate::settings::Renderer::Cpu;
    first.prefs.frame_labels = false;
    let cmd_j = egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::J);
    first
        .prefs
        .keymap
        .set(crate::keymap::Binding::CommandBar, Some(cmd_j));
    // Hand and Select swap keys: each change clashes only with a default the other replaces.
    let (hand, select) = (
        crate::keymap::Binding::HandTool,
        crate::keymap::Binding::SelectTool,
    );
    let (h_key, v_key) = (hand.default_keys(), select.default_keys());
    first.prefs.keymap.set(hand, Some(v_key));
    first.prefs.keymap.set(select, Some(h_key));
    first.save(&mut storage);
    // A saved shortcut this version cannot read (an unknown name, a key with no name, keys
    // another one has) is dropped on its own; the rest of the window state still loads.
    let saved = storage.0.get(crate::UI_STATE_KEY).cloned().expect("saved");
    let list = saved.find("changes:[").expect("the keymap");
    let end = list + saved[list..].find(']').expect("the list ends");
    let damaged = format!(
        "{},(NoSuchShortcut,None),(Open,Some((command:true))),(FindOnPage,Some((key:\"J\",command:true))){}",
        &saved[..end],
        &saved[end..]
    );
    storage.0.insert(crate::UI_STATE_KEY.to_owned(), damaged);
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&storage);
    let second = App::new(&cc, Vec::new());
    assert!(!second.show_left, "the hidden sidebar stays hidden");
    assert!(second.show_right);
    assert!((second.pages_share - 0.3).abs() < 1e-6);
    assert_eq!(
        second.prefs.renderer,
        crate::settings::Renderer::Cpu,
        "Settings → Canvas"
    );
    assert!(!second.prefs.frame_labels);
    assert_eq!(
        second.prefs.keymap.get(crate::keymap::Binding::CommandBar),
        Some(cmd_j),
        "changed shortcuts"
    );
    assert_eq!(
        (
            second.prefs.keymap.get(hand),
            second.prefs.keymap.get(select)
        ),
        (Some(v_key), Some(h_key)),
        "a swap"
    );
    assert_eq!(
        second.prefs.keymap.get(crate::keymap::Binding::Open),
        Some(crate::keymap::Binding::Open.default_keys()),
        "the unreadable change is dropped"
    );
    assert_eq!(
        second.prefs.keymap.get(crate::keymap::Binding::FindOnPage),
        Some(crate::keymap::Binding::FindOnPage.default_keys()),
        "⌘J stays with the command bar, which had it first"
    );
}

#[test]
fn a_sideways_swipe_over_the_sidebar_moves_between_spaces() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.run_steps(2);
    let swipe = |h: &mut Harness<'_, App>, dx: f32| {
        let wheel = |delta, phase| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase,
            modifiers: egui::Modifiers::NONE,
        };
        h.event(Event::PointerMoved(Pos2::new(120.0, 400.0)));
        h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::Start));
        for _ in 0..4 {
            h.event(wheel(egui::vec2(dx / 4.0, 0.0), egui::TouchPhase::Move));
            h.step();
        }
        h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::End));
        h.run_steps(2);
    };
    assert_eq!(h.state().active, 1, "the last file opened is active");
    swipe(&mut h, -120.0);
    assert_eq!(h.state().active, 1, "no space after the last");
    swipe(&mut h, 120.0);
    assert_eq!(
        h.state().active,
        0,
        "swiping right shows the previous space"
    );
    swipe(&mut h, -30.0);
    assert_eq!(h.state().active, 0, "a short swipe springs back");
    swipe(&mut h, -120.0);
    assert_eq!(h.state().active, 1);
}

#[test]
fn the_sidebar_keeps_its_width_while_spaces_slide() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    // With motion on, so the pages really move.
    crate::anim::set_reduced(&h.ctx.clone(), false);
    h.run_steps(30);
    let width = |h: &Harness<'_, App>| {
        egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new(crate::chrome::SIDEBAR))
            .map(|s| s.size().x)
            .expect("the sidebar was shown")
    };
    let before = width(&h);
    let edge = Pos2::new(before, 400.0);
    h.event(Event::PointerMoved(edge));
    h.step();
    press(&h, edge, PointerButton::Primary, true);
    for i in 1..=4 {
        h.event(Event::PointerMoved(edge - egui::vec2(5.0 * i as f32, 0.0)));
        h.step();
    }
    press(
        &h,
        edge - egui::vec2(20.0, 0.0),
        PointerButton::Primary,
        false,
    );
    h.run_steps(5);
    let narrowed = width(&h);
    assert!(
        (narrowed - (before - 20.0)).abs() < 1.5,
        "{before} -> {narrowed}"
    );
    for _ in 0..3 {
        let other = (h.state().active + 1) % 2;
        let name = h.state().tabs[other].space_name();
        h.get_by_role_and_label(Role::Button, &name).click();
        for _ in 0..40 {
            h.step();
            let now = width(&h);
            assert!(
                (now - narrowed).abs() < 1.5,
                "the width moved to {now} from {narrowed}"
            );
        }
    }
}

#[test]
fn mid_swipe_both_spaces_show_and_the_colour_is_between_theirs() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.state_mut().active = 0;
    h.run_steps(3);
    let wheel = |delta, phase| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(Event::PointerMoved(Pos2::new(120.0, 400.0)));
    h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::Start));
    for _ in 0..4 {
        h.event(wheel(egui::vec2(-25.0, 0.0), egui::TouchPhase::Move));
        h.step();
    }
    h.step();
    let app = h.state();
    assert!(
        (app.slide + 100.0).abs() < 1.0,
        "the pages follow the fingers: {}",
        app.slide
    );
    assert!(
        h.query_all_by_label("Page 1").count() >= 2,
        "both spaces' page lists are on screen"
    );
    let dark = h.ctx.theme() == egui::Theme::Dark;
    let shown = app.tint_now(&h.ctx).top;
    let own = crate::theme::space_tint(app.tabs[0].colour(), dark).top;
    let next = crate::theme::space_tint(app.tabs[1].colour(), dark).top;
    assert!(
        shown != own && shown != next,
        "{shown:?} between {own:?} and {next:?}"
    );
    h.event(wheel(egui::Vec2::ZERO, egui::TouchPhase::End));
    h.run_steps(3);
    assert_eq!(h.state().active, 1, "past a third: the next space");
}

#[test]
fn a_space_change_is_saved_when_its_popover_is_replaced_its_file_closes_or_the_app_quits() {
    use crate::menu::Action;
    use eframe::App as _;
    let mut h = harness();
    let ctx = h.ctx.clone();
    let saved_name = |h: &Harness<'_, App>, key: &Option<String>, folder: &Option<String>| {
        let id = crate::spaces::FileId {
            key: key.as_deref(),
            file: FIXTURE,
            folder: folder.as_deref(),
        };
        h.state().spaces.lookup(id).and_then(|s| s.name)
    };
    let (key, folder) = {
        let tab = &h.state().tabs[0];
        (tab.doc_key.clone(), tab.folder.clone())
    };
    let rename = |h: &mut Harness<'_, App>, text: &str| {
        let ctx = h.ctx.clone();
        h.state_mut().apply(Action::RenameSpace, &ctx);
        h.run_steps(2);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(2);
    };

    rename(&mut h, "One");
    h.state_mut().apply(Action::SpaceIcon, &ctx);
    h.run_steps(2);
    assert_eq!(
        saved_name(&h, &key, &folder).as_deref(),
        Some("One"),
        "replaced by another popover"
    );

    h.key_press(Key::Escape);
    h.run_steps(2);
    h.state_mut().apply(Action::RenameSpace, &ctx);
    h.run_steps(2);
    h.event(egui::Event::Text(" more".into()));
    h.run_steps(2);
    h.state_mut().on_exit();
    assert_eq!(
        saved_name(&h, &key, &folder).as_deref(),
        Some("One more"),
        "the app quits"
    );

    let mut h = harness();
    rename(&mut h, "Two");
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(2);
    assert!(h.state().tabs.is_empty(), "⌘W closed the file");
    assert_eq!(
        saved_name(&h, &key, &folder).as_deref(),
        Some("Two"),
        "its file closed"
    );
}

#[test]
fn the_command_bar_is_modal_and_never_lists_another_files_layers() {
    use crate::command::Target;
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    // The files load side by side; find which tab is which.
    let structure = (h.state().tabs.iter())
        .position(|t| t.title() == FIXTURE)
        .expect("open");
    h.state_mut().active = structure;
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(2);
    h.event(egui::Event::Text("symbols and images".into()));
    h.run_steps(2);
    let layer = |h: &Harness<'_, App>| {
        h.state()
            .command
            .results()
            .iter()
            .any(|e| matches!(e.target, Target::Layer(_)))
    };
    assert!(layer(&h), "the layer of the active file is listed");
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    h.run_steps(2);
    assert_eq!(
        h.state().tabs.len(),
        2,
        "⌘W does not close a file under the bar"
    );
    assert_eq!(
        h.state().active,
        structure,
        "Ctrl+Tab does not switch under the bar"
    );
    assert!(h.state().command.is_open());
    // Should the active file change anyway (an agent, a menu), the list follows it.
    h.state_mut().active = 1 - structure;
    h.run_steps(2);
    assert!(!layer(&h), "the other file has no such layer");
}

#[test]
fn during_an_eased_zoom_clicks_hit_what_is_drawn_and_tiles_are_asked_for_the_target() {
    // A rounded rectangle away from other nodes (as in ui_copy_tests.rs).
    const NODE: Guid = Guid(1, 34);
    // Frames a 60th of a second apart, so the 0.28 s ease takes several.
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE)]));
    wait(&mut h, &mut Vec::new(), "the file to load", |a| {
        a.tabs
            .first()
            .is_some_and(|t| t.canvas_rect.is_some() && !t.fit_pending)
    });
    let ctx = h.ctx.clone();
    crate::anim::set_reduced(&ctx, false);
    h.run_steps(2);
    h.state_mut().apply(crate::menu::Action::ZoomBy(2.0), &ctx);
    for _ in 0..5 {
        h.step();
    }
    let tab = &h.state().tabs[0];
    let (shown, target) = (tab.shown, tab.view);
    assert!(
        shown.zoom > target.zoom / 2.0 && shown.zoom < target.zoom,
        "mid-ease: {shown:?}"
    );
    let ppp = h.ctx.pixels_per_point();
    let level = crate::tiles::level_for(target.zoom * ppp);
    assert!(
        tab.tile_requests.iter().all(|k| k.level == level),
        "tiles for the target level {level}: {:?}",
        tab.tile_requests
            .iter()
            .map(|k| k.level)
            .collect::<Vec<_>>()
    );
    // Click where the node is drawn now, not where it will end up.
    let b = figstudio_render::node_bounds(&tab.doc, NODE).expect("bounds");
    let page = Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    let at = shown.to_screen(tab.canvas_rect.expect("laid out").min, page);
    h.event(Event::PointerMoved(at));
    h.step();
    press(&h, at, PointerButton::Primary, true);
    press(&h, at, PointerButton::Primary, false);
    h.step();
    assert_eq!(h.state().tabs[0].selection, Some(NODE));
}

#[test]
fn a_click_on_the_peeking_sidebar_does_not_reach_the_canvas() {
    const NODE: Guid = Guid(1, 34);
    let mut h = harness();
    h.state_mut().show_left = false;
    h.run_steps(3);
    h.state_mut().tabs[0].select(Some(NODE));
    h.state_mut().peek = true;
    h.run_steps(3);
    // The card's left padding, over the canvas's empty margin.
    let at = Pos2::new(crate::theme::GAP + 3.0, 300.0);
    h.event(Event::PointerMoved(at));
    h.step();
    assert!(h.state().peek, "still peeking");
    press(&h, at, PointerButton::Primary, true);
    press(&h, at, PointerButton::Primary, false);
    h.run_steps(2);
    assert_eq!(
        h.state().tabs[0].selection,
        Some(NODE),
        "the click stayed on the sidebar"
    );
}

#[test]
fn a_narrow_window_keeps_the_status_and_the_zoom_percentage() {
    // The smallest window, both panels open: the canvas card is about 200 points wide.
    let mut h = Harness::builder()
        .with_size(egui::vec2(720.0, 560.0))
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE)]));
    wait(&mut h, &mut Vec::new(), "the file to load", |a| {
        a.tabs
            .first()
            .is_some_and(|t| t.canvas_rect.is_some() && !t.fit_pending)
    });
    h.run_steps(3);
    let card = h.state().tabs[0].canvas_rect.expect("laid out");
    assert!(card.width() < 260.0, "narrow: {}", card.width());
    assert!(
        h.query_by_label_contains("layers drawn").is_some(),
        "the status pill"
    );
    assert!(
        h.query_by_label("Zoom to fit").is_some(),
        "the zoom percentage"
    );
    assert!(h.query_by_label("Hand tool").is_some());
}

#[test]
fn many_files_scroll_in_the_footer_and_a_dot_closes_from_its_menu_not_a_cross() {
    let names = [
        "structure.fig",
        "vector.fig",
        "sections.fig",
        "corners.fig",
        "stacks_wrap.fig",
        "noise_and_grain.fig",
        "broken_images.fig",
    ];
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(move |cc| App::new(cc, names.iter().map(|n| fixture(n)).collect()));
    wait(&mut h, &mut Vec::new(), "all files to load", |a| {
        a.tabs.len() == 7
    });
    h.run_steps(3);
    // The narrowest sidebar.
    let width =
        egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new(crate::chrome::SIDEBAR))
            .map(|s| s.size().x)
            .expect("shown");
    let edge = Pos2::new(width, 400.0);
    h.event(Event::PointerMoved(edge));
    h.step();
    press(&h, edge, PointerButton::Primary, true);
    // To just above its minimum (past it, the sidebar would close).
    let to = 200.0 - width;
    for i in 1..=4 {
        h.event(Event::PointerMoved(
            edge + egui::vec2(to * i as f32 / 4.0, 0.0),
        ));
        h.step();
    }
    press(
        &h,
        edge + egui::vec2(to, 0.0),
        PointerButton::Primary,
        false,
    );
    h.run_steps(5);
    assert!(h.state().show_left, "the sidebar is still open");
    let plus = h.get_by_label("Open file…").rect();
    let app = h.state();
    assert!(
        app.dots_clip.right() < plus.left(),
        "the dots stop before the + button"
    );
    let active = app.tabs[app.active].id;
    let (_, dot) = *app
        .space_dots
        .iter()
        .find(|(id, _)| *id == active)
        .expect("a dot");
    assert!(
        app.dots_clip.expand(0.5).contains_rect(dot),
        "the active file's dot is scrolled into view, clear of the + button: {dot:?} in {:?}",
        app.dots_clip
    );
    // No close button on the dot (Arc has none); its right-click menu closes it.
    let name = app.tabs[app.active].space_name();
    h.event(Event::PointerMoved(dot.center()));
    h.run_steps(2);
    assert!(h.query_by_label(&format!("Close {name}")).is_none());
    h.get_by_role_and_label(Role::Button, &name)
        .click_secondary();
    h.run_steps(2);
    h.get_by_label("Close space").click();
    h.run_steps(2);
    assert_eq!(h.state().tabs.len(), 6);
}

#[test]
fn without_the_blur_the_window_clears_to_an_opaque_colour() {
    use eframe::App as _;
    let h = harness();
    let clear = h.state().clear_color(&egui::Visuals::dark());
    assert_eq!(
        clear[3], 1.0,
        "no blur here (Linux, Windows, the browser, tests): {clear:?}"
    );
}

#[test]
fn settings_open_with_their_shortcut_and_their_tabs_switch() {
    let mut h = harness();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    h.run_steps(3);
    assert!(h.state().settings.is_open());
    for tab in ["General", "Canvas", "Shortcuts", "Agents", "About"] {
        assert!(
            h.query_by_role_and_label(Role::Button, tab).is_some(),
            "{tab} tab"
        );
    }
    h.get_by_role_and_label(Role::Button, "Shortcuts").click();
    h.run_steps(3);
    let row =
        |h: &Harness<'_, App>, what: &str| h.query_by_role_and_label(Role::Label, what).is_some();
    assert!(row(&h, "Command bar"), "the list");
    assert!(row(&h, "Find on page"));
    let search = h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput
            && n.placeholder() == Some("Type a feature name or shortcut")
    });
    search.click();
    h.run_steps(2);
    h.event(egui::Event::Text("find".into()));
    h.run_steps(3);
    assert!(row(&h, "Find on page"));
    assert!(!row(&h, "Command bar"), "filtered out");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().settings.is_open(), "Escape closes them");
}

#[test]
fn general_and_canvas_settings_change_the_app() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut().apply(crate::menu::Action::Settings, &ctx);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Light").click();
    h.run_steps(3);
    assert_eq!(ctx.theme(), egui::Theme::Light);
    let reduced = crate::anim::reduced(&ctx);
    h.get_by_role_and_label(Role::CheckBox, "Reduce motion")
        .click();
    h.run_steps(2);
    assert_eq!(crate::anim::reduced(&ctx), !reduced);

    h.get_by_role_and_label(Role::Button, "Canvas").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::CheckBox, "Frame names")
        .click();
    h.run_steps(2);
    assert!(!h.state().prefs.frame_labels);
    h.get_by_role_and_label(Role::Button, "Zooms").click();
    h.run_steps(2);
    assert!(h.state().prefs.scroll_zooms);
    h.key_press(Key::Escape);
    // Motion is on again (toggled above): Settings takes the pointer until it has faded.
    h.run_steps(40);
    // A plain scroll on the canvas now zooms.
    let before = h.state().tabs[0].view;
    let at = h.state().tabs[0].canvas_rect.expect("laid out").center();
    h.event(Event::PointerMoved(at));
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 60.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    let after = h.state().tabs[0].view;
    assert!(after.zoom > before.zoom, "{before:?} -> {after:?}");
}

#[test]
fn a_shortcut_can_be_changed_removed_and_reset_and_the_app_follows_it() {
    use crate::keymap::Binding;
    let mut h = harness();
    let ctx = h.ctx.clone();
    let open_shortcuts = |h: &mut Harness<'_, App>| {
        h.state_mut().apply(crate::menu::Action::Settings, &ctx);
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, "Shortcuts").click();
        h.run_steps(2);
    };
    let record = |h: &mut Harness<'_, App>, what: &str, modifiers: Modifiers, key: Key| {
        // Searched for, so its row is on screen to click.
        h.state_mut().settings.filter = what.to_owned();
        h.run_steps(2);
        h.get_by_label(&format!("{what} shortcut")).click();
        h.run_steps(2);
        assert!(h.state().settings.recording().is_some(), "recording {what}");
        h.key_press_modifiers(modifiers, key);
        h.run_steps(2);
        assert!(h.state().settings.recording().is_none());
    };
    let cmd_j = KeyboardShortcut::new(Modifiers::COMMAND, Key::J);

    open_shortcuts(&mut h);
    record(&mut h, "Command bar", Modifiers::COMMAND, Key::J);
    assert_eq!(h.state().prefs.keymap.get(Binding::CommandBar), Some(cmd_j));
    assert!(
        h.query_by_role_and_label(Role::Label, "Customized Shortcuts")
            .is_some()
    );
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().settings.is_open());

    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(2);
    assert!(!h.state().command.is_open(), "⌘L no longer opens it");
    h.key_press_modifiers(Modifiers::COMMAND, Key::J);
    h.run_steps(2);
    assert!(h.state().command.is_open(), "⌘J does");
    h.key_press(Key::Escape);
    h.run_steps(2);

    // Keys another shortcut has move, and the person is told.
    open_shortcuts(&mut h);
    record(&mut h, "Find on page", Modifiers::COMMAND, Key::J);
    assert_eq!(h.state().prefs.keymap.get(Binding::FindOnPage), Some(cmd_j));
    assert_eq!(h.state().prefs.keymap.get(Binding::CommandBar), None);
    assert!(h.query_by_label_contains("moved here").is_some());
    // Backspace removes a shortcut; Escape keeps it.
    record(&mut h, "Hand tool", Modifiers::NONE, Key::Backspace);
    assert_eq!(h.state().prefs.keymap.get(Binding::HandTool), None);
    record(&mut h, "Select tool", Modifiers::NONE, Key::Escape);
    assert_eq!(
        h.state().prefs.keymap.get(Binding::SelectTool),
        Some(Binding::SelectTool.default_keys())
    );
    h.get_by_label("Reset All Shortcuts").click();
    h.run_steps(2);
    assert!(!h.state().prefs.keymap.any_changed());
    assert_eq!(
        h.state().prefs.keymap.get(Binding::CommandBar),
        Some(Binding::CommandBar.default_keys())
    );
}

#[test]
fn a_setting_s_note_stays_clear_of_its_control() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.state_mut()
        .settings
        .show(Some(crate::settings::Tab::Canvas), &ctx);
    h.run_steps(3);
    let note = h.get_by_label_contains("Applies at the next start").rect();
    for choice in ["Auto", "GPU", "CPU"] {
        let control = h.get_by_role_and_label(Role::Button, choice).rect();
        assert!(
            !note.intersects(control),
            "{choice} {control:?} drawn over the note {note:?}"
        );
    }
}

#[test]
fn control_and_a_digit_go_to_that_space_and_hovering_a_dot_names_it_with_those_keys() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.run_steps(3);
    h.state_mut().active = 0;
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::CTRL, Key::Num2);
    h.run_steps(2);
    assert_eq!(h.state().active, 1, "⌃2 goes to the second space");
    h.key_press_modifiers(Modifiers::CTRL, Key::Num1);
    h.run_steps(2);
    assert_eq!(h.state().active, 0);
    h.key_press_modifiers(Modifiers::CTRL, Key::Num9);
    h.run_steps(2);
    assert_eq!(h.state().active, 0, "no ninth space: nothing happens");

    // Hovering the second dot: a pill with its name and ⌃2, as Arc's.
    let name = h.state().tabs[1].space_name();
    let keys = h
        .ctx
        .format_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Num2));
    let pill = format!("{name}, {keys}");
    assert!(h.query_by_label_contains(&pill).is_none());
    let id = h.state().tabs[1].id;
    let (_, dot) = *h
        .state()
        .space_dots
        .iter()
        .find(|(t, _)| *t == id)
        .expect("a dot");
    h.event(Event::PointerMoved(dot.center()));
    h.run_steps(4);
    let shown = h.get_by_label_contains(&pill).rect();
    assert!(
        shown.bottom() <= dot.top(),
        "above the dot: {shown:?} over {dot:?}"
    );
}

#[test]
fn the_name_pill_waits_for_the_pointer_to_rest_on_a_dot_then_follows_it_along_the_row() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(0.1)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.run_steps(3);
    let dot = |h: &Harness<'_, App>, i: usize| {
        let id = h.state().tabs[i].id;
        h.state()
            .space_dots
            .iter()
            .find(|(t, _)| *t == id)
            .map(|(_, r)| *r)
            .expect("a dot")
    };
    let pill = |h: &Harness<'_, App>, i: usize| {
        let name = h.state().tabs[i].space_name();
        h.query_by_label_contains(&format!("{name}, ")).is_some()
    };
    let (first, second) = (dot(&h, 0), dot(&h, 1));
    h.event(Event::PointerMoved(second.center()));
    h.run_steps(2);
    assert!(
        !pill(&h, 1),
        "not at once: the pointer has to rest on the dot"
    );
    // Still moving over the dot: the wait counts from when it stops, as a tooltip's.
    h.run_steps(2);
    h.event(Event::PointerMoved(second.center() + egui::vec2(3.0, 1.0)));
    h.run_steps(3);
    assert!(!pill(&h, 1), "moved 0.3 s ago: still waiting");
    h.run_steps(3);
    assert!(pill(&h, 1), "after a moment it shows");
    h.event(Event::PointerMoved(second.center()));
    h.run_steps(1);
    assert!(pill(&h, 1), "once shown, moving on the dot keeps it");
    // Moving along the row: the next dot's pill shows at once.
    h.event(Event::PointerMoved(first.center()));
    h.run_steps(1);
    assert!(pill(&h, 0), "the neighbour's shows straight away");
    assert!(!pill(&h, 1));
    // Away for a while, then back: the wait starts again.
    let canvas = h.state().tabs[h.state().active]
        .canvas_rect
        .expect("laid out")
        .center();
    h.event(Event::PointerMoved(canvas));
    h.run_steps(10);
    assert!(!pill(&h, 0) && !pill(&h, 1));
    h.event(Event::PointerMoved(second.center()));
    h.run_steps(2);
    assert!(!pill(&h, 1), "a fresh hover waits again");
}

#[test]
fn overlays_fade_away_instead_of_vanishing_and_then_are_gone() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE)]));
    wait(&mut h, &mut Vec::new(), "the file to load", |a| {
        a.tabs.len() == 1
    });
    let ctx = h.ctx.clone();
    crate::anim::set_reduced(&ctx, false);
    h.run_steps(10);
    // Closed with Escape: still on screen two frames later, gone after 0.6 s.
    let closes =
        |h: &mut Harness<'_, App>, what: &str, there: &dyn Fn(&Harness<'_, App>) -> bool| {
            h.run_steps(30);
            assert!(there(h), "{what} opened");
            h.key_press(Key::Escape);
            h.run_steps(2);
            assert!(there(h), "{what} fades out instead of vanishing");
            h.run_steps(36);
            assert!(!there(h), "{what} is gone afterwards");
        };

    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    closes(&mut h, "the command bar", &|h| {
        h.query_by_label_contains("Esc to close").is_some()
    });

    h.state_mut().apply(crate::menu::Action::Settings, &ctx);
    closes(&mut h, "Settings", &|h| {
        h.query_by_label("Close settings").is_some()
    });

    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(5);
    h.event(egui::Event::Text("abc".into()));
    closes(&mut h, "the find highlights", &|h| {
        !h.state().tabs[0].find_hits.is_empty()
    });

    h.state_mut().apply(crate::menu::Action::RenameSpace, &ctx);
    closes(&mut h, "the space popover", &|h| {
        h.query_by_label("Space name").is_some()
    });
}

#[test]
fn the_name_pill_fades_out_after_the_pointer_leaves() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    let ctx = h.ctx.clone();
    crate::anim::set_reduced(&ctx, false);
    h.run_steps(30);
    let visible = |h: &Harness<'_, App>| {
        h.ctx
            .data(|d| d.get_temp::<bool>(egui::Id::new("space-pill-visible")))
            .unwrap_or(false)
    };
    let id = h.state().tabs[1].id;
    let (_, dot) = *h
        .state()
        .space_dots
        .iter()
        .find(|(t, _)| *t == id)
        .expect("a dot");
    h.event(Event::PointerMoved(dot.center()));
    h.run_steps(45);
    assert!(visible(&h), "shown after resting on the dot");
    let canvas = h.state().tabs[h.state().active]
        .canvas_rect
        .expect("laid out")
        .center();
    h.event(Event::PointerMoved(canvas));
    h.run_steps(2);
    assert!(visible(&h), "fading out, not gone at once");
    h.run_steps(40);
    assert!(!visible(&h), "gone once faded");
}

#[test]
fn recording_takes_a_real_keyboard_chord_and_nothing_else_acts_on_it() {
    use crate::keymap::Binding;
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    h.run_steps(2);
    let ctx = h.ctx.clone();
    let keys = |b: Binding| KeyboardShortcut::new(Modifiers::COMMAND, b.default_keys().logical_key);
    // The hints show the command bar's and Open's keys as set.
    let hint = |h: &Harness<'_, App>, name: &str, k: KeyboardShortcut| {
        let label = format!("{name} ({})", h.ctx.format_shortcut(&k));
        h.query_by_label(&label).is_some()
    };
    assert!(hint(&h, "Command bar", keys(Binding::CommandBar)));

    h.state_mut()
        .settings
        .show(Some(crate::settings::Tab::Shortcuts), &ctx);
    h.run_steps(2);
    // Typed into the search first, so its field has focus when the keycap is clicked.
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput
            && n.placeholder() == Some("Type a feature name or shortcut")
    })
    .click();
    h.run_steps(2);
    h.event(egui::Event::Text("Command".into()));
    h.run_steps(2);
    assert_eq!(h.state().settings.filter, "Command");
    h.get_by_label("Command bar shortcut").click();
    h.run_steps(2);
    assert!(h.state().settings.recording().is_some());

    // While recording, the menu bar's actions do not run (on a Mac it may still have had
    // the old accelerator for the chord being pressed).
    let tabs = h.state().tabs.len();
    h.state_mut()
        .apply_native(vec![crate::menu::Action::CloseTab], &ctx);
    assert_eq!(h.state().tabs.len(), tabs, "⌘W did not close a file");

    // A keyboard sends ⌘ as a key of its own, then J with ⌘ held, and J as text too.
    let cmd = Modifiers::COMMAND;
    h.event(egui::Event::Key {
        key: Key::SuperLeft,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: cmd,
    });
    h.run_steps(1);
    assert!(
        h.state().settings.recording().is_some(),
        "⌘ alone does not end it"
    );
    h.event(egui::Event::Key {
        key: Key::J,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: cmd,
    });
    h.event(egui::Event::Text("j".into()));
    h.run_steps(2);
    let cmd_j = KeyboardShortcut::new(cmd, Key::J);
    assert_eq!(h.state().prefs.keymap.get(Binding::CommandBar), Some(cmd_j));
    assert_eq!(
        h.state().settings.filter,
        "Command",
        "the letter did not reach the search"
    );
    assert!(!h.state().command.is_open(), "the chord did not also run");

    // ⇧⌘C arrives as a Copy event with no key event; it is recorded all the same.
    h.get_by_label("Command bar shortcut").click();
    h.run_steps(2);
    let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
    h.event_modifiers(egui::Event::Copy, shift_cmd);
    h.run_steps(2);
    assert_eq!(
        h.state().prefs.keymap.get(Binding::CommandBar),
        Some(KeyboardShortcut::new(shift_cmd, Key::C))
    );
    h.state_mut()
        .prefs
        .keymap
        .set(Binding::CommandBar, Some(cmd_j));

    h.key_press(Key::Escape);
    h.run_steps(30);
    assert!(
        hint(&h, "Command bar", cmd_j),
        "the sidebar hint follows the change"
    );
    // A shortcut removed: no hint at all.
    h.state_mut().prefs.keymap.set(Binding::CommandBar, None);
    h.run_steps(2);
    assert!(h.query_by_label("Command bar").is_some());
}

#[test]
fn the_name_pill_wakes_the_app_and_a_long_name_stays_inside_a_narrow_window() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(720.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    // A name far wider than the window (a long file title shows the same way).
    h.state_mut().tabs[1].style.name = Some("A very long space name ".repeat(12));
    h.run_steps(10);
    let id = h.state().tabs[1].id;
    let (_, dot) = *h
        .state()
        .space_dots
        .iter()
        .find(|(t, _)| *t == id)
        .expect("a dot");
    h.event(Event::PointerMoved(dot.center()));
    // Until the pointer has rested, nothing else asks for a frame; the app asks to be woken
    // when the pill is due rather than waiting for the next event.
    h.run();
    let visible = |h: &Harness<'_, App>| {
        h.ctx
            .data(|d| d.get_temp::<bool>(egui::Id::new("space-pill-visible")))
            .unwrap_or(false)
    };
    assert!(!visible(&h), "not yet");
    let wake = h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert!(
        wake <= std::time::Duration::from_millis(500),
        "woken for the pill: {wake:?}"
    );
    h.run_steps(40);
    assert!(visible(&h), "shown after the wait");
    // The pill reads "{name}, {keys}"; the dot is the name alone.
    let pill = h.get_by_label_contains("space name , ").rect();
    let screen = h.ctx.content_rect();
    assert!(
        screen.contains_rect(pill),
        "the cut pill fits the window: {pill:?} in {screen:?}"
    );
}

#[test]
fn a_dot_sliding_over_is_clicked_where_it_is_drawn() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    crate::anim::set_reduced(&h.ctx.clone(), false);
    h.run_steps(60);
    let dot = |h: &Harness<'_, App>, id: u64| {
        h.state()
            .space_dots
            .iter()
            .find(|(t, _)| *t == id)
            .map(|(_, r)| *r)
            .expect("a dot")
    };
    let second = h.state().tabs[1].id;
    let before = dot(&h, second);
    // Closing the first: the second slides into the middle of the row.
    h.state_mut().active = 1;
    h.state_mut().close(0);
    h.run_steps(3);
    let sliding = dot(&h, second);
    h.run_steps(60);
    let after = dot(&h, second);
    assert!(
        (after.center().x - before.center().x).abs() > 8.0,
        "it moves: {before:?} → {after:?}"
    );
    assert!(
        (sliding.center().x - before.center().x).abs()
            < (after.center().x - before.center().x).abs() * 0.8,
        "three frames in, the dot is still on its way: {sliding:?}"
    );
    let name = h.state().tabs[0].space_name();
    let node = h
        .get_all_by_label(&name)
        .map(|n| n.rect())
        .find(|r| (r.center().y - after.center().y).abs() < 1.0)
        .expect("the dot's button");
    assert!(
        (node.center().x - after.center().x).abs() < 0.5,
        "its button is where it is drawn: {node:?} vs {after:?}"
    );
}

/// A real click at `pos` (kittest's `click()` goes to the widget directly, past whatever
/// covers it).
fn click_at(h: &mut Harness<'_, App>, pos: egui::Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
}

#[test]
fn a_fading_overlay_still_takes_clicks_and_keys_until_it_is_gone() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    let ctx = h.ctx.clone();
    crate::anim::set_reduced(&ctx, false);
    h.run_steps(30);
    let hide = h.get_by_label("Hide sidebar").rect().center();

    // The command bar fading out: the dimmed screen eats a click on the sidebar button,
    // and ⌘W does not close a file it still covers.
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(30);
    h.key_press(Key::Escape);
    h.run_steps(2);
    click_at(&mut h, hide);
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(2);
    assert!(
        h.state().show_left,
        "the click did not reach the sidebar button"
    );
    assert_eq!(h.state().tabs.len(), 2, "⌘W waited for the bar to go");
    h.run_steps(40);

    // Settings fading out: the same for a click, and a plain tool key waits.
    h.state_mut().apply(crate::menu::Action::Settings, &ctx);
    h.run_steps(30);
    h.key_press(Key::Escape);
    h.run_steps(2);
    click_at(&mut h, hide);
    h.key_press(Key::H);
    h.run_steps(2);
    assert!(
        h.state().show_left,
        "the click did not reach the sidebar button"
    );
    assert_eq!(h.state().tool, crate::canvas::Tool::Select, "H waited too");
    h.run_steps(40);

    // Once they are gone, both work again.
    h.key_press(Key::H);
    h.run_steps(2);
    assert_eq!(h.state().tool, crate::canvas::Tool::Hand);
    click_at(&mut h, hide);
    h.run_steps(2);
    assert!(!h.state().show_left);
}

#[test]
fn a_new_space_popover_pops_in_while_the_last_one_fades_out() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    let ctx = h.ctx.clone();
    crate::anim::set_reduced(&ctx, false);
    h.run_steps(30);
    h.state_mut().apply(crate::menu::Action::RenameSpace, &ctx);
    h.run_steps(30);
    // Straight from renaming to the icon picker: both on screen for a moment.
    h.state_mut().apply(crate::menu::Action::SpaceIcon, &ctx);
    h.run_steps(2);
    assert!(
        h.query_by_label("Space name").is_some(),
        "the rename popover fades out"
    );
    assert!(h.query_by_label_contains("Space icon").is_some());
    let opening = h.state().space_popover_presence(&ctx, true).expect("open");
    assert!(opening < 0.9, "the icon picker pops in: {opening}");
    h.run_steps(40);
    assert!(h.query_by_label("Space name").is_none(), "then it is gone");

    // While it fades out it still covers what is under it: the pointer there is over the
    // popover (egui sends clicks and scrolls to that layer, where everything is inert).
    h.state_mut().apply(crate::menu::Action::RenameSpace, &ctx);
    h.run_steps(30);
    let field = h.get_by_label("Space name").rect().center();
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.query_by_label("Space name").is_some(), "still fading");
    assert_eq!(
        h.ctx.layer_id_at(field).map(|l| l.order),
        Some(egui::Order::Foreground),
        "the popover's layer, not the sidebar under it"
    );
}

#[test]
fn find_highlights_fading_out_stay_with_their_own_file() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    let ctx = h.ctx.clone();
    crate::anim::set_reduced(&ctx, false);
    h.run_steps(10);
    h.state_mut().active = 1;
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(5);
    h.event(egui::Event::Text("e".into()));
    h.run_steps(5);
    assert!(
        !h.state().tabs[1].find_hits.is_empty(),
        "the second file has matches"
    );
    h.state_mut().active = 0;
    h.run_steps(5);
    assert!(!h.state().tabs[0].find_hits.is_empty(), "so has the first");
    // Closed on the first file; going to the second while it fades shows none of the
    // second file's older matches.
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(
        !h.state().tabs[0].find_hits.is_empty(),
        "fading on its own file"
    );
    h.state_mut().active = 1;
    h.run_steps(1);
    assert!(h.state().tabs[1].find_hits.is_empty(), "no stale matches");

    // The last file closing while the bar fades: it still finishes fading.
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(30);
    h.key_press(Key::Escape);
    h.run_steps(2);
    h.state_mut().close(1);
    h.state_mut().close(0);
    h.run_steps(40);
    assert!(
        !crate::anim::on_screen(&ctx, egui::Id::new("find-bar")),
        "the bar finished fading with no file open"
    );
}

#[test]
fn a_far_space_nudges_in_and_the_next_one_turns_the_page() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| {
            App::new(
                cc,
                vec![
                    fixture(FIXTURE),
                    fixture("vector.fig"),
                    fixture("sections.fig"),
                ],
            )
        });
    wait(&mut h, &mut Vec::new(), "the files to load", |a| {
        a.tabs.len() == 3
    });
    crate::anim::set_reduced(&h.ctx.clone(), false);
    h.state_mut().active = 0;
    h.run_steps(60);
    h.key_press_modifiers(Modifiers::CTRL, Key::Num3);
    h.run_steps(2);
    assert!(h.state().slide_nudge, "two spaces away: a short nudge");
    h.run_steps(60);
    h.key_press_modifiers(Modifiers::CTRL, Key::Num2);
    h.run_steps(2);
    assert!(!h.state().slide_nudge, "the one beside: a page turn");
    assert!(h.state().slide.abs() > SLIDE_CHECK, "{}", h.state().slide);
}

/// More than the far jump's nudge: a page turn starts a whole page away.
const SLIDE_CHECK: f32 = 40.0;

#[test]
fn the_command_bar_lists_the_layers_of_the_page_shown_now() {
    let mut h = harness();
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run_steps(3);
    h.event(egui::Event::Text("Component".into()));
    h.run_steps(3);
    let layers = |h: &Harness<'_, App>| {
        h.state()
            .command
            .results()
            .iter()
            .filter(|e| matches!(e.target, crate::command::Target::Layer(_)))
            .count()
    };
    assert!(layers(&h) > 0, "the page's layers are listed");
    // The page changes under the open bar (an agent can turn it): the list follows.
    h.state_mut().tabs[0].page = figstudio_fig::Guid(77, 77);
    h.run_steps(3);
    assert_eq!(layers(&h), 0, "an empty page has none");
}

/// The colour the untextured fills give the point `at` in the last frame, laid over each
/// other in paint order (tiles are textures and are left out).
fn fill_at(h: &Harness<'_, App>, at: egui::Pos2) -> egui::Color32 {
    fn walk(shape: &egui::Shape, at: egui::Pos2, acc: &mut [f32; 3]) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, at, acc)),
            egui::Shape::Rect(r)
                if r.brush.is_none() && r.blur_width == 0.0 && r.rect.contains(at) =>
            {
                // Premultiplied: what is under shows through by one minus alpha.
                let [cr, cg, cb, ca] = r.fill.to_array().map(f32::from);
                let keep = 1.0 - ca / 255.0;
                *acc = [cr + acc[0] * keep, cg + acc[1] * keep, cb + acc[2] * keep];
            }
            _ => {}
        }
    }
    let mut acc = [0.0; 3];
    for clipped in &h.output().shapes {
        if clipped.clip_rect.contains(at) {
            walk(&clipped.shape, at, &mut acc);
        }
    }
    let [r, g, b] = acc.map(|c| c.round().clamp(0.0, 255.0) as u8);
    egui::Color32::from_rgb(r, g, b)
}

/// Two files open with motion on, each frame 1/60 s, the active one drawn in full.
fn two_files_settled() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE), fixture("vector.fig")]));
    wait(&mut h, &mut Vec::new(), "both files to load", |a| {
        a.tabs.len() == 2
    });
    crate::anim::set_reduced(&h.ctx.clone(), false);
    wait(&mut h, &mut Vec::new(), "the file's tiles", |a| {
        a.tiles.count(a.tabs[a.active].doc.instance).1 > 0
    });
    h.run_steps(30);
    h
}

/// Whether the last frame drew a texture other than egui's font atlas (a tile).
fn drew_a_tile(h: &Harness<'_, App>) -> bool {
    fn walk(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(walk),
            egui::Shape::Mesh(m) => m.texture_id != egui::TextureId::default(),
            egui::Shape::Rect(r) => r.brush.is_some(),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|c| walk(&c.shape))
}

#[test]
fn another_file_fades_in_over_the_last_one_never_over_the_bare_card() {
    let mut h = two_files_settled();
    let page = |t: &crate::tab::Tab| {
        crate::canvas::to_color32(figstudio_render::page_background(&t.doc, t.page))
    };
    let from = h.state().active;
    let to = (from + 1) % 2;
    let (old, new) = (page(&h.state().tabs[from]), page(&h.state().tabs[to]));
    let old_doc = h.state().tabs[from].doc.instance;
    // Left of the middle, clear of the pills and of frame labels.
    let card = h.state().card_rect.expect("laid out");
    let at = egui::pos2(card.left() + 12.0, card.center().y);
    assert_eq!(fill_at(&h, at), old, "settled on the first file's page");
    h.state_mut().active = to;
    h.step();
    // The new file is not drawn yet; what is drawn is the last one, from its cached tiles,
    // and those count as on screen so the cache does not drop them first.
    assert!(drew_a_tile(&h), "the last file's tiles under the new one");
    assert!(
        h.state().tiles.count(old_doc).1 > 0,
        "kept as drawn this frame"
    );
    let between = |c: egui::Color32| {
        let (c, a, b) = (c.to_array(), old.to_array(), new.to_array());
        (0..3).all(|i| c[i] + 2 >= a[i].min(b[i]) && c[i] <= a[i].max(b[i]) + 2)
    };
    for frame in 0..20 {
        let c = fill_at(&h, at);
        assert!(
            between(c),
            "frame {frame}: {c:?}, not between {old:?} and {new:?}"
        );
        h.step();
    }
    assert_eq!(fill_at(&h, at), new, "settled on the second file's page");
}

#[test]
fn a_closed_file_fades_out_from_its_own_picture_then_its_tiles_go() {
    let mut h = two_files_settled();
    let active = h.state().active;
    let closed = h.state().tabs[active].doc.instance;
    h.state_mut().close(active);
    h.step();
    assert!(
        h.state().tiles.count(closed).1 > 0,
        "the closed file's picture is under the next one"
    );
    h.run_steps(30);
    assert_eq!(
        h.state().tiles.count(closed).0,
        0,
        "its tiles go after the fade"
    );
}

/// A file of two empty pages, each with its own background colour.
fn two_pages() -> (crate::tab::Tab, [figstudio_fig::Guid; 2]) {
    use figstudio_fig::{Document, Guid, Node, Obj, Val};
    let obj = |fields: &[(&str, Val)]| {
        let mut o = Obj::new();
        for (k, v) in fields {
            o.insert(k, v.clone());
        }
        Val::Obj(o)
    };
    let pages = [Guid(9, 1), Guid(9, 2)];
    let nodes = pages
        .iter()
        .zip([0.95, 0.3])
        .enumerate()
        .map(|(i, (page, grey))| {
            let colour = obj(&[
                ("r", Val::Float(grey)),
                ("g", Val::Float(grey)),
                ("b", Val::Float(grey)),
                ("a", Val::Float(1.0)),
            ]);
            let parent = obj(&[
                ("guid", Guid(0, 0).to_val()),
                ("position", Val::Str(i.to_string())),
            ]);
            Node::from_data(obj(&[
                ("guid", page.to_val()),
                ("type", Val::Str("CANVAS".into())),
                ("name", Val::Str(format!("Page {}", i + 1))),
                ("backgroundColor", colour),
                ("parentIndex", parent),
            ]))
            .expect("page")
        })
        .collect();
    let mut doc = Document::from_bytes(include_bytes!("../../../tests/fixtures/structure.fig"))
        .expect("fixture");
    doc.set_nodes(nodes).expect("nodes");
    doc.pages = pages.to_vec();
    let mut tab = crate::tab::Tab::new(
        900,
        "pages.fig".into(),
        std::sync::Arc::new(doc),
        None,
        std::time::Duration::ZERO,
    );
    tab.page = pages[0];
    (tab, pages)
}

fn internal_pages_tab() -> (crate::tab::Tab, [figstudio_fig::Guid; 2]) {
    let (mut tab, pages) = two_pages();
    let doc = std::sync::Arc::get_mut(&mut tab.doc).expect("unshared test document");
    doc.pages = vec![pages[0]];
    doc.internal_pages = vec![pages[1]];
    (tab, pages)
}

fn internal_pages_harness() -> (Harness<'static, App>, [figstudio_fig::Guid; 2]) {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, Vec::new()));
    let (tab, pages) = internal_pages_tab();
    h.state_mut().tabs.push(tab);
    h.state_mut().active = 0;
    h.run_steps(3);
    (h, pages)
}

#[test]
fn internal_pages_expand_open_and_return_to_a_normal_page() {
    let (mut h, [normal, internal]) = internal_pages_harness();
    assert!(h.query_by_label("Page 1").is_some());
    assert!(h.query_by_label("Page 2").is_none(), "collapsed initially");
    h.get_by_label("Internal pages (1)").click();
    h.run_steps(20);
    assert_eq!(
        h.state().tabs[0].page,
        normal,
        "expanding does not navigate"
    );
    h.state_mut().tabs[0].selection = Some(normal);
    h.state_mut().tabs[0].hover = Some(normal);
    h.get_by_label("Page 2").click();
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].page, internal);
    assert_eq!(h.state().tabs[0].selection, None);
    assert_eq!(h.state().tabs[0].hover, None);
    h.get_by_label("Page 1").click();
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].page, normal);
    h.get_by_label("Internal pages (1)").click();
    h.run_steps(20);
    assert!(h.query_by_label("Page 2").is_none());
    assert!(h.query_by_label("Page 1").is_some());
}

#[test]
fn internal_pages_open_on_programmatic_navigation_but_can_still_be_collapsed() {
    let navigations: [fn(&mut crate::tab::Tab, figstudio_fig::Guid); 3] = [
        crate::tab::Tab::set_page,
        |tab, page| tab.select(Some(page)),
        crate::tab::Tab::show_node,
    ];
    for navigate in navigations {
        let (mut h, [normal, internal]) = internal_pages_harness();
        navigate(&mut h.state_mut().tabs[0], internal);
        h.run_steps(20);
        assert!(
            h.query_by_role_and_label(Role::Button, "Page 2").is_some(),
            "navigation reveals the active page"
        );
        h.get_by_label("Internal pages (1)").click();
        h.run_steps(20);
        assert!(
            h.query_by_role_and_label(Role::Button, "Page 2").is_none(),
            "manual collapse is respected"
        );
        h.state_mut().tabs[0].set_page(normal);
        h.run_steps(3);
        navigate(&mut h.state_mut().tabs[0], internal);
        h.run_steps(20);
        assert!(
            h.query_by_role_and_label(Role::Button, "Page 2").is_some(),
            "later navigation reveals it again"
        );
    }
}

#[test]
fn internal_pages_start_visible_when_already_selected() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_eframe(|cc| App::new(cc, Vec::new()));
    let (mut tab, [_, internal]) = internal_pages_tab();
    tab.page = internal;
    h.state_mut().tabs.push(tab);
    h.state_mut().active = 0;
    h.run_steps(20);
    assert!(h.query_by_label("Page 2").is_some());
    assert_eq!(h.state().tabs[0].page, internal);
}

#[test]
fn internal_pages_highlight_does_not_inherit_another_lists_animation() {
    use figstudio_fig::{Guid, Node, Val};
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, Vec::new()));
    crate::anim::set_reduced(&h.ctx.clone(), false);
    let (mut tab, [first, internal]) = internal_pages_tab();
    let second = Guid(9, 3);
    let mut nodes = tab.doc.nodes.clone();
    let mut data = nodes[0].data.clone();
    let Val::Obj(ref mut fields) = data else {
        panic!("page object")
    };
    fields.insert("guid", second.to_val());
    fields.insert("name", Val::Str("Page 3".into()));
    nodes.push(Node::from_data(data).expect("page"));
    let doc = std::sync::Arc::get_mut(&mut tab.doc).expect("unshared test document");
    doc.set_nodes(nodes).expect("nodes");
    doc.pages = vec![first, second];
    doc.internal_pages = vec![internal];
    tab.page = second;
    h.state_mut().tabs.push(tab);
    h.state_mut().active = 0;
    h.run_steps(30);
    h.get_by_label("Internal pages (1)").click();
    h.run_steps(20);
    let row = h.get_by_label("Page 3").rect();
    let selected = fill_at(&h, egui::pos2(row.right() - 12.0, row.center().y));
    h.state_mut().tabs[0].set_page(internal);
    h.step();
    let row = h.get_by_label("Page 2").rect();
    assert_eq!(
        fill_at(&h, egui::pos2(row.right() - 12.0, row.center().y)),
        selected,
        "the internal list does not animate from the normal list's second row"
    );
}

#[test]
fn internal_pages_disclosure_and_rows_share_the_page_list_width() {
    let (mut h, _) = internal_pages_harness();
    let normal = h.get_by_label("Page 1").rect();
    let header = h.get_by_label("Internal pages (1)").rect();
    assert_eq!(header.x_range(), normal.x_range(), "full-row click target");
    assert_eq!(header.height(), normal.height(), "consistent row height");
    h.get_by_label("Internal pages (1)").click();
    h.run_steps(20);
    let internal = h.get_by_label("Page 2").rect();
    assert_eq!(internal.x_range(), normal.x_range(), "no extra indentation");
}

#[test]
fn internal_pages_can_be_opened_with_the_keyboard() {
    let (mut h, [_, internal]) = internal_pages_harness();
    h.get_by_label("Internal pages (1)").focus();
    h.run_steps(2);
    assert!(h.get_by_label("Internal pages (1)").is_focused());
    h.key_press(Key::Space);
    h.run_steps(20);
    h.key_press(Key::Tab);
    h.run_steps(2);
    assert!(h.get_by_label("Page 2").is_focused());
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].page, internal);
}

#[test]
fn files_without_internal_pages_have_no_disclosure() {
    let (mut h, _) = internal_pages_harness();
    h.state_mut().tabs[0] = two_pages().0;
    h.run_steps(3);
    assert!(h.query_by_label_contains("Internal pages (").is_none());
}

#[test]
fn another_page_crossfades_from_the_last_and_going_back_fades_from_what_was_shown() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, Vec::new()));
    crate::anim::set_reduced(&h.ctx.clone(), false);
    let (tab, [one, two]) = two_pages();
    let colour =
        |page| crate::canvas::to_color32(figstudio_render::page_background(&tab.doc, page));
    let (a, b) = (colour(one), colour(two));
    h.state_mut().tabs.push(tab);
    h.state_mut().active = 0;
    h.run_steps(30);
    let card = h.state().card_rect.expect("laid out");
    let at = egui::pos2(card.left() + 12.0, card.center().y);
    let towards = |c: egui::Color32, to: egui::Color32| -> u16 {
        (0..3)
            .map(|i| u16::from(c.to_array()[i].abs_diff(to.to_array()[i])))
            .sum()
    };
    assert_eq!(fill_at(&h, at), a, "settled on page 1");
    // The real path: it also asks for a fit, so the view changes on the first frame.
    h.state_mut().tabs[0].set_page(two);
    h.step();
    assert_eq!(fill_at(&h, at), a, "page 2 is not there at once");
    let mut last = a;
    for frame in 0..20 {
        h.step();
        let c = fill_at(&h, at);
        assert!(
            towards(c, b) <= towards(last, b),
            "frame {frame}: {c:?} after {last:?}"
        );
        last = c;
        if frame == 2 {
            assert!(towards(c, a) > 6 && towards(c, b) > 6, "a blend: {c:?}");
        }
    }
    assert_eq!(last, b, "settled on page 2");
    // Back to page 1 and, three frames in, to page 2 again: page 2 was nearly all that was
    // shown, so the fade starts from it, not from page 1 barely drawn.
    h.state_mut().tabs[0].set_page(one);
    h.run_steps(3);
    let shown = fill_at(&h, at);
    h.state_mut().tabs[0].set_page(two);
    h.step();
    let c = fill_at(&h, at);
    assert!(
        towards(c, b) <= towards(shown, b),
        "jumped to {c:?} from {shown:?}"
    );
}

#[test]
fn the_first_file_fades_in_over_its_own_page_colour() {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(cc, vec![fixture(FIXTURE)]));
    crate::anim::set_reduced(&h.ctx.clone(), false);
    let start = std::time::Instant::now();
    while h.state().tabs.is_empty() {
        assert!(start.elapsed().as_secs() < 20, "the file did not load");
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.step();
    }
    let tab = &h.state().tabs[0];
    let page = crate::canvas::to_color32(figstudio_render::page_background(&tab.doc, tab.page));
    let card = h.state().card_rect.expect("laid out");
    let at = egui::pos2(card.left() + 12.0, card.center().y);
    // The frame it arrived in is the first of its fade.
    for frame in 0..20 {
        let c = fill_at(&h, at);
        let near = (0..3).all(|i| c.to_array()[i].abs_diff(page.to_array()[i]) <= 2);
        assert!(near, "frame {frame}: {c:?}, not the page's {page:?}");
        h.step();
    }
}

/// Every rectangle fill painted this frame, nested shapes included.
fn rect_fills(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Color32> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) => out.push(r.fill),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in shapes {
        walk(&c.shape, &mut out);
    }
    out
}

#[test]
fn the_properties_card_and_its_sections_take_the_space_colour() {
    let mut h = harness();
    h.run_steps(3);
    let ctx = h.ctx.clone();
    let neutral = crate::theme::palette(&ctx);
    let tinted = neutral.on_tint(h.state().tint_now(&ctx));
    assert_ne!(tinted.well, neutral.well, "the space has a colour");
    let fills = rect_fills(&h.output().shapes);
    assert!(fills.contains(&tinted.card), "no tinted card in {fills:?}");
    // The document overview section (no selection) is drawn on a tinted well.
    assert!(fills.contains(&tinted.well), "no tinted well in {fills:?}");
    assert!(
        !fills.contains(&neutral.well),
        "a neutral well in {fills:?}"
    );
}
