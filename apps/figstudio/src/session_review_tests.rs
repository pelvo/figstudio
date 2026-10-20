use std::sync::Arc;
use std::time::Duration;

use super::*;

fn restore(paths: &[PathBuf], active: &Path, last_active: u64) -> Restore {
    let mut restore = Restore::new(Session {
        files: paths.to_vec(),
        active: Some(active.to_path_buf()),
    });
    restore.waiting.clear();
    restore.last_active = Some(last_active);
    restore
}

fn tab(id: u64, path: &Path, doc: &Arc<figstudio_fig::Document>) -> Tab {
    Tab::new(
        id,
        path.to_path_buf(),
        Arc::clone(doc),
        None,
        Duration::ZERO,
    )
}

#[test]
fn workspace_interim_close_preserves_saved_focus_without_erasing_a_manual_choice() {
    let h = crate::ui_tests::harness();
    let doc = Arc::clone(&h.state().tabs[0].doc);
    let paths = ["neighbor.fig", "interim.fig", "target.fig"].map(PathBuf::from);
    for manual in [false, true] {
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = App::new(&cc, Vec::new());
        app.tabs = vec![tab(10, &paths[0], &doc), tab(20, &paths[1], &doc)];
        app.active = if manual { 0 } else { 1 };
        app.loading.begin(&paths[2]);
        app.restoring = Some(restore(&paths, &paths[2], 20));
        app.close(1);
        let expected = if manual { &paths[0] } else { &paths[2] };
        assert_eq!(
            Session::capture(&app).active,
            Some(absolute(expected)),
            "early save after close"
        );
        let previous = app.tabs.get(app.active).map(|t| t.id);
        app.loading.finish(&paths[2]);
        app.tabs.push(tab(30, &paths[2], &doc));
        app.active = app.tabs.len() - 1;
        app.workspace_finished(&paths[2], previous, None);
        assert_eq!(
            &app.tabs[app.active].path, expected,
            "focus when the saved target arrives"
        );
    }
}

#[test]
fn workspace_failed_saved_active_file_records_the_displayed_fallback_during_restore() {
    let h = crate::ui_tests::harness();
    let doc = Arc::clone(&h.state().tabs[0].doc);
    let paths = ["visible.fig", "failed.fig", "pending.fig"].map(PathBuf::from);
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut app = App::new(&cc, Vec::new());
    app.tabs.push(tab(10, &paths[0], &doc));
    app.loading.begin(&paths[2]);
    app.restoring = Some(restore(&paths, &paths[1], 10));
    app.workspace_finished(&paths[1], Some(10), None);
    assert!(app.restoring.is_some(), "another file is still pending");
    assert_eq!(Session::capture(&app).active, Some(absolute(&paths[0])));
}

#[test]
fn workspace_re_review_manual_alias_keeps_focus_while_automatic_alias_preserves_saved_intent() {
    let h = crate::ui_tests::harness();
    let doc = Arc::clone(&h.state().tabs[0].doc);
    let paths = ["chosen.fig", "interim.fig", "target.fig"].map(PathBuf::from);
    let alias = PathBuf::from("alias.fig");
    let third = PathBuf::from("other.fig");
    for (manual, select_third) in [(true, false), (false, false), (true, true), (false, true)] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = App::new(&cc, Vec::new());
        app.tabs = vec![tab(10, &paths[0], &doc), tab(20, &paths[1], &doc)];
        let real = app.tabs[0].real_path.clone();
        app.active = 1;
        app.loading.begin(&paths[2]);
        let mut restore = restore(&paths, &paths[2], 20);
        if !manual {
            restore.session.files.push(alias.clone());
        }
        app.restoring = Some(restore);
        assert!(app.reuse_loaded_file(&ctx, &alias, Some(&real)));
        let expected = if manual { &paths[0] } else { &paths[2] };
        assert_eq!(
            Session::capture(&app).active,
            Some(absolute(expected)),
            "early save after alias completion"
        );
        let expected = if select_third {
            app.tabs.push(tab(40, &third, &doc));
            app.active = app.tabs.len() - 1;
            assert_eq!(Session::capture(&app).active, Some(absolute(&third)));
            &third
        } else {
            expected
        };
        let previous = app.tabs.get(app.active).map(|t| t.id);
        app.loading.finish(&paths[2]);
        app.tabs.push(tab(30, &paths[2], &doc));
        app.active = app.tabs.len() - 1;
        app.workspace_finished(&paths[2], previous, None);
        assert_eq!(
            &app.tabs[app.active].path, expected,
            "focus after remaining restore completes"
        );
        assert_eq!(
            app.tabs.len(),
            if select_third { 4 } else { 3 },
            "alias must not create a duplicate"
        );
    }
}

#[test]
fn workspace_standard_path_comparison_already_ignores_trailing_separators() {
    let path = PathBuf::from("example.fig");
    let trailing = PathBuf::from("example.fig/");
    assert_eq!(path, trailing);
}
