use super::*;
use crate::session_store::Store;
use crate::ui_tests::Memory;
use eframe::App as _;

#[test]
fn workspace_private_store_replaces_insecure_existing_permissions() {
    let dir = tempfile::tempdir().expect("private workspace folder");
    let path = dir.path().join("workspace.json");
    let mut store = Store::at(path.clone());
    let first = Session {
        files: vec![PathBuf::from("first.fig")],
        active: Some(PathBuf::from("first.fig")),
    };
    store.save(&first);
    assert!(store.take_problem().is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("workspace metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("simulate an older insecure file");
    }
    let second = Session {
        files: vec![PathBuf::from("second.fig")],
        active: Some(PathBuf::from("second.fig")),
    };
    store.save(&second);
    assert!(store.take_problem().is_none());
    let loaded = Store::at(path.clone()).read(None);
    assert_eq!(loaded.files, second.files);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("replaced metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn workspace_corruption_is_isolated_from_other_preferences_and_kept_for_repair() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let path = dir.path().join("workspace.json");
    let damaged = b"{\"files\":17}";
    std::fs::write(&path, damaged).expect("damaged workspace fixture");
    let mut memory = Memory::default();
    memory.0.insert(
        crate::UI_STATE_KEY.to_owned(),
        "(show_left:false,show_right:true,pages_share:0.3,prefs:(renderer:Cpu,frame_labels:false))"
            .to_owned(),
    );
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&memory);
    let mut app = App::with_workspace_store(&cc, Vec::new(), Store::at(path.clone()));
    assert!(!app.show_left && !app.prefs.frame_labels);
    assert_eq!(app.prefs.renderer, crate::settings::Renderer::Cpu);
    let warning = app.workspace_store.take_problem().expect("read warning");
    assert!(
        warning.contains("Workspace changes in this session will not be saved."),
        "warning must say this session's edits are lost, not deferred until restart: {warning}"
    );
    assert!(
        warning.contains("repair") && warning.contains("restart the app"),
        "warning must explain repair and restart: {warning}"
    );
    let changed = absolute(&PathBuf::from("session-only.fig"));
    app.loading.begin(&changed);
    assert_eq!(Session::capture(&app).files, [changed]);
    app.save(&mut memory);
    assert!(
        memory.0.values().all(|v| !v.contains("session-only.fig")),
        "this session's changes cannot leak into legacy preferences"
    );
    assert_eq!(
        std::fs::read(&path).expect("original damaged file"),
        damaged
    );
    let saved: crate::UiState =
        eframe::get_value(&memory, crate::UI_STATE_KEY).expect("unrelated preferences still save");
    assert!(!saved.show_left && !saved.prefs.frame_labels);
    std::fs::remove_file(&path).expect("repair by removing damaged metadata");
    app.save(&mut memory);
    assert!(
        !path.exists(),
        "this process must not resume private writes"
    );
    let mut restarted = Store::at(path.clone());
    assert!(
        restarted.read(Some(&memory)).files.is_empty(),
        "restart must not recover workspace edits from the disabled session"
    );
    restarted.save(&Session::default());
    assert!(path.is_file(), "a fresh store can save after repair");
}

#[test]
fn workspace_unreadable_store_keeps_recovery_guidance_when_legacy_is_limited() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let path = dir.path().join("workspace.json");
    let damaged = b"{\"files\":17}";
    std::fs::write(&path, damaged).expect("damaged workspace fixture");
    let files: Vec<_> = (0..MAX_SAVED_FILES + 3)
        .map(|i| PathBuf::from(format!("fallback-{i}.fig")))
        .collect();
    let active = files.last().cloned();
    let mut memory = Memory::default();
    eframe::set_value(
        &mut memory,
        crate::UI_STATE_KEY,
        &LegacyPreferences {
            show_left: false,
            show_right: true,
            pages_share: 0.4,
            prefs: crate::Prefs::default(),
            workspace: Session { files, active },
        },
    );
    let mut store = Store::at(path.clone());
    let loaded = store.read(Some(&memory));
    assert_eq!(loaded.files.len(), MAX_SAVED_FILES);
    assert_eq!(loaded.active, Some(PathBuf::from("fallback-502.fig")));
    assert_eq!(store.legacy_backup().files.len(), MAX_SAVED_FILES + 3);
    let warning = store.take_problem().expect("combined workspace warning");
    assert!(
        warning.contains("Saved workspace is unreadable."),
        "the limit must not replace the unreadable-store explanation: {warning}"
    );
    assert!(
        warning.contains("Workspace changes in this session will not be saved."),
        "the combined warning must disclose current-session loss: {warning}"
    );
    assert!(
        warning.contains("repair") && warning.contains("restart the app"),
        "the combined warning must retain recovery guidance: {warning}"
    );
    assert!(
        warning.contains("3 saved files were skipped beyond the 500-file limit"),
        "the combined warning must also disclose the cap: {warning}"
    );
    store.save(&Session::default());
    assert_eq!(
        std::fs::read(&path).expect("damaged metadata stays untouched"),
        damaged
    );
}

#[test]
fn workspace_store_caps_saved_and_loaded_paths_without_spawning_threads() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let path = dir.path().join("workspace.json");
    let files: Vec<_> = (0..600)
        .map(|i| PathBuf::from(format!("file-{i}.fig")))
        .collect();
    let session = Session {
        active: files.last().cloned(),
        files,
    };
    std::fs::write(
        &path,
        serde_json::to_vec(&session).expect("oversized list fixture"),
    )
    .expect("workspace fixture");
    let mut store = Store::at(path.clone());
    let loaded = store.read(None);
    assert_eq!(loaded.files.len(), 500);
    assert_eq!(
        loaded.active, session.active,
        "keep the selected space within the cap"
    );
    assert!(
        loaded
            .files
            .contains(loaded.active.as_ref().expect("active space"))
    );
    assert!(
        store.take_problem().is_some(),
        "skipped metadata is announced"
    );
    store.save(&session);
    let written: Session = serde_json::from_slice(&std::fs::read(&path).expect("bounded snapshot"))
        .expect("session JSON");
    assert_eq!(written.files.len(), 500);
    assert_eq!(written.active, session.active);
}

#[test]
fn workspace_store_rejects_oversized_bytes_without_overwriting_them() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let path = dir.path().join("workspace.json");
    std::fs::File::create(&path)
        .expect("large metadata fixture")
        .set_len(4 * 1024 * 1024 + 1)
        .expect("bounded sparse fixture");
    let mut store = Store::at(path.clone());
    assert!(store.read(None).files.is_empty());
    assert!(store.take_problem().is_some());
    store.save(&Session::default());
    assert_eq!(
        std::fs::metadata(&path).expect("preserved metadata").len(),
        4 * 1024 * 1024 + 1
    );
}

#[test]
fn workspace_legacy_metadata_migrates_privately_and_leaves_preferences_path_free() {
    #[derive(serde::Serialize)]
    struct Legacy {
        show_left: bool,
        show_right: bool,
        pages_share: f32,
        prefs: crate::Prefs,
        workspace: Session,
    }
    let dir = tempfile::tempdir().expect("workspace folder");
    let path = dir.path().join("workspace.json");
    let file = crate::ui_tests::fixture("structure.fig");
    let mut memory = Memory::default();
    eframe::set_value(
        &mut memory,
        crate::UI_STATE_KEY,
        &Legacy {
            show_left: false,
            show_right: true,
            pages_share: 0.4,
            prefs: crate::Prefs {
                frame_labels: false,
                ..Default::default()
            },
            workspace: Session {
                files: vec![file.clone()],
                active: Some(file.clone()),
            },
        },
    );
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&memory);
    let mut app = App::with_workspace_store(&cc, Vec::new(), Store::at(path.clone()));
    assert!(!app.show_left && !app.prefs.frame_labels);
    assert!(
        app.workspace_store.take_migration(),
        "first frame requests legacy cleanup"
    );
    assert!(!app.workspace_store.take_migration());
    assert_eq!(Store::at(path).read(None).files, [file]);
    app.save(&mut memory);
    assert!(memory.0.values().all(|v| !v.contains("structure.fig")));
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LegacyPreferences {
    show_left: bool,
    show_right: bool,
    pages_share: f32,
    prefs: crate::Prefs,
    #[serde(default)]
    workspace: Session,
}

fn legacy_preferences() -> (Memory, PathBuf) {
    let file = crate::ui_tests::fixture("structure.fig");
    let mut memory = Memory::default();
    eframe::set_value(
        &mut memory,
        crate::UI_STATE_KEY,
        &LegacyPreferences {
            show_left: false,
            show_right: true,
            pages_share: 0.4,
            prefs: crate::Prefs::default(),
            workspace: Session {
                files: vec![file.clone()],
                active: Some(file.clone()),
            },
        },
    );
    (memory, file)
}

#[test]
fn workspace_re_review_unreadable_private_store_keeps_legacy_preferences() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let path = dir.path().join("workspace.json");
    std::fs::create_dir(&path).expect("non-file private metadata");
    let (mut memory, file) = legacy_preferences();
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&memory);
    let mut app = App::with_workspace_store(&cc, Vec::new(), Store::at(path.clone()));
    app.show_left = true;
    app.save(&mut memory);
    let saved: LegacyPreferences =
        eframe::get_value(&memory, crate::UI_STATE_KEY).expect("preferences still save");
    assert_eq!(
        saved.workspace.files,
        [file],
        "retain the only readable workspace copy"
    );
    assert!(saved.show_left, "unrelated preferences still update");
    assert!(!app.workspace_store.take_migration(), "no unsafe cleanup");
    assert!(path.is_dir(), "unreadable private metadata stays untouched");
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&memory);
    let reopened = App::with_workspace_store(&cc, Vec::new(), Store::at(path));
    assert!(reopened.show_left);
    assert_eq!(
        Session::capture(&reopened).files.len(),
        1,
        "legacy remains restorable after restart"
    );
}

#[cfg(unix)]
#[test]
fn workspace_re_review_failed_private_import_keeps_legacy_until_retry_succeeds() {
    let dir = tempfile::tempdir().expect("workspace folder");
    let link = dir.path().join("config");
    std::os::unix::fs::symlink(dir.path().join("missing"), &link)
        .expect("dangling config directory blocks creation even for root");
    let path = link.join("workspace.json");
    assert_eq!(
        std::fs::metadata(&path)
            .expect_err("private snapshot missing")
            .kind(),
        std::io::ErrorKind::NotFound
    );
    let (mut memory, file) = legacy_preferences();
    let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    cc.storage = Some(&memory);
    let mut app = App::with_workspace_store(&cc, Vec::new(), Store::at(path.clone()));
    app.show_left = true;
    let extra = absolute(&PathBuf::from("new-file.fig"));
    app.loading.begin(&extra);
    app.save(&mut memory);
    let saved: LegacyPreferences =
        eframe::get_value(&memory, crate::UI_STATE_KEY).expect("preferences still save");
    assert!(
        memory.0.values().all(|v| !v.contains("new-file.fig")),
        "never add new paths to legacy preferences"
    );
    assert_eq!(
        saved.workspace.files.as_slice(),
        std::slice::from_ref(&file),
        "failed import must retain legacy paths"
    );
    assert!(saved.show_left);
    assert!(!app.workspace_store.take_migration());
    assert!(app.workspace_store.take_problem().is_some());
    std::fs::remove_file(&link).expect("repair config directory");
    std::fs::create_dir(&link).expect("writable config directory");
    app.save(&mut memory);
    assert_eq!(Store::at(path).read(None).files, [file, extra]);
    assert!(
        memory.0.values().all(|v| !v.contains("structure.fig")),
        "cleanup only after private success"
    );
}

#[cfg(unix)]
#[test]
fn workspace_failed_serialization_keeps_the_previous_private_snapshot() {
    use std::os::unix::ffi::OsStringExt as _;
    let dir = tempfile::tempdir().expect("workspace folder");
    let path = dir.path().join("workspace.json");
    let mut store = Store::at(path.clone());
    store.save(&Session::default());
    let original = std::fs::read(&path).expect("valid old snapshot");
    let invalid = PathBuf::from(std::ffi::OsString::from_vec(b"invalid-\xff.fig".to_vec()));
    store.save(&Session {
        files: vec![invalid.clone()],
        active: Some(invalid),
    });
    assert!(store.take_problem().is_some());
    assert_eq!(
        std::fs::read(&path).expect("snapshot after failure"),
        original
    );
}
