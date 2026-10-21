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

#[test]
fn workspace_recents_explicit_loads_record_the_loaded_tab_after_saved_focus_and_sort() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let bytes = std::fs::read(fixture).unwrap();
    for order in [[0, 1], [1, 0]] {
        let dir = tempfile::tempdir().unwrap();
        let paths = ["saved.fig", "later.fig"].map(|name| dir.path().join(name));
        for path in &paths {
            std::fs::write(path, &bytes).unwrap();
        }
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| App::new(cc, Vec::new()));
        let mut restoring = Restore::new(Session {
            files: paths.to_vec(),
            active: Some(paths[0].clone()),
        });
        // Deliver one real worker result at a time, in either completion order.
        restoring.waiting.clear();
        for path in &paths {
            h.state_mut().loading.begin(path);
        }
        h.state_mut().restoring = Some(restoring);
        for (loaded, &index) in order.iter().enumerate() {
            // A user opening an already pending path makes it an explicit open.
            h.state_mut().open(paths[index].clone());
            h.state()
                .workers
                .load_bytes(paths[index].clone(), bytes.clone());
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while h.state().tabs.len() != loaded + 1 {
                assert!(std::time::Instant::now() < deadline, "restored file loads");
                h.step();
                std::thread::yield_now();
            }
        }
        let app = h.state();
        assert!(app.restoring.is_none());
        assert_eq!(Session::capture(app).files, paths);
        assert_eq!(app.tabs[app.active].path, paths[0]);
        let recent: Vec<_> = app
            .recents
            .entries()
            .iter()
            .map(|entry| entry.path.file_name().unwrap().to_owned())
            .collect();
        let expected: Vec<_> = order
            .iter()
            .rev()
            .map(|&index| paths[index].file_name().unwrap().to_owned())
            .collect();
        assert_eq!(
            recent, expected,
            "record the loaded tab even when a different saved tab stays active"
        );
    }
}

#[test]
fn workspace_recents_automatic_restore_preserves_history_and_pictures() {
    let bytes = std::fs::read(crate::ui_tests::fixture("structure.fig")).unwrap();
    for listed in [false, true] {
        for order in [[0, 1], [1, 0]] {
            let dir = tempfile::tempdir().unwrap();
            let paths = ["saved.fig", "later.fig"].map(|name| dir.path().join(name));
            for path in &paths {
                std::fs::write(path, &bytes).unwrap();
            }
            let history: Vec<_> = (0..60)
                .map(|i| crate::recents::Entry {
                    path: if listed && i == 59 {
                        paths[0].clone()
                    } else {
                        dir.path().join(format!("history-{i}.fig"))
                    },
                    opened: 60 - i,
                    key: Some("kept".into()),
                })
                .collect();
            let before = serde_json::to_vec(&history).unwrap();
            let file = dir.path().join("recents.json");
            std::fs::write(&file, &before).unwrap();
            let thumbs = dir.path().join("thumbs");
            std::fs::create_dir(&thumbs).unwrap();
            let recents = crate::recents::Recents::at(file.clone(), Some(thumbs.clone()));
            let picture = recents.thumb_file(&history.last().unwrap().path).unwrap();
            std::fs::write(&picture, b"kept picture").unwrap();
            let mut h = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1000.0, 700.0))
                .build_eframe(|cc| App::new(cc, Vec::new()));
            h.state_mut().recents = recents;
            let mut restoring = Restore::new(Session {
                files: paths.to_vec(),
                active: Some(paths[0].clone()),
            });
            restoring.waiting.clear();
            for path in &paths {
                h.state_mut().loading.begin(path);
            }
            h.state_mut().restoring = Some(restoring);
            for (loaded, &index) in order.iter().enumerate() {
                h.state()
                    .workers
                    .load_bytes(paths[index].clone(), bytes.clone());
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                while h.state().tabs.len() != loaded + 1 {
                    assert!(std::time::Instant::now() < deadline, "restored file loads");
                    h.step();
                    std::thread::yield_now();
                }
            }
            assert!(h.state().restoring.is_none());
            assert_eq!(Session::capture(h.state()).files, paths);
            assert_eq!(h.state().tabs[h.state().active].path, paths[0]);
            assert_eq!(
                h.state().recents.entries(),
                history.as_slice(),
                "automatic restore must not replace or reorder user-open history"
            );
            assert_eq!(std::fs::read(file).unwrap(), before, "no history rewrite");
            assert_eq!(std::fs::read(picture).unwrap(), b"kept picture");
            assert_eq!(std::fs::read_dir(thumbs).unwrap().count(), 1);
        }
    }
}

#[test]
fn workspace_recents_explicit_startup_records_an_already_saved_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chosen.fig");
    std::fs::copy(crate::ui_tests::fixture("structure.fig"), &path).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(|cc| App::new(cc, Vec::new()));
    h.state_mut().restore_workspace(
        Session {
            files: vec![path.clone()],
            active: Some(path.clone()),
        },
        vec![path.clone()],
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while h.state().tabs.is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "explicit startup file loads"
        );
        h.step();
        std::thread::yield_now();
    }
    assert_eq!(h.state().recents.entries().len(), 1);
    assert_eq!(h.state().recents.entries()[0].path, path);
}
