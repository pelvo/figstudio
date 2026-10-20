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
    assert!(app.workspace_store.take_problem().is_some());
    app.save(&mut memory);
    assert_eq!(
        std::fs::read(&path).expect("original damaged file"),
        damaged
    );
    let saved: crate::UiState =
        eframe::get_value(&memory, crate::UI_STATE_KEY).expect("unrelated preferences still save");
    assert!(!saved.show_left && !saved.prefs.frame_labels);
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
