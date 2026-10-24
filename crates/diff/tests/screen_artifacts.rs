mod support;
use figstudio_diff::{
    DiffReport, RenderOpts, attach_render_diffs_with_sink, diff_documents, write_render_images,
};
use figstudio_fig::{Guid, Node, Val};
use support::{document, filled_rectangle_at};

#[test]
fn screen_json_roundtrip_legacy_defaults_and_streamed_png_links() {
    let side = |color, kind: &str| {
        let mut frame = filled_rectangle_at(2, 1, (1.0, 1.0, 1.0), 0.0, 0.0, 300.0, 200.0);
        frame.data.set("type", Val::Str(kind.into()));
        document(vec![
            Node::from_data(frame.data).unwrap(),
            filled_rectangle_at(3, 2, color, 80.0, 70.0, 20.0, 20.0),
        ])
    };
    let (old, new) = (
        side((1.0, 0.0, 0.0), "FRAME"),
        side((0.0, 1.0, 0.0), "FRAME"),
    );
    let dir = std::env::temp_dir().join(format!("figstudio-screen-links-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("render")).unwrap();
    let mut report = diff_documents(&old, &new);
    let mut links = Vec::new();
    attach_render_diffs_with_sink(
        &old,
        &new,
        &mut report,
        &RenderOpts::default(),
        &|_| None,
        &|_| None,
        &mut |id, images| links.push((id, write_render_images(&dir, id, images).unwrap())),
    );
    for (id, links) in links {
        links.attach(&mut report, id);
    }
    let s = &report.pages[0].screens[0];
    assert_eq!(s.thumb_old.as_deref(), Some("render/0-1-s0-before.png"));
    assert_eq!(s.thumb_new.as_deref(), Some("render/0-1-s0-after.png"));
    assert_eq!(
        s.regions[0].new_png.as_deref(),
        Some("render/0-1-s0-a0-after.png")
    );
    for name in [
        s.thumb_old.as_ref().unwrap(),
        s.thumb_new.as_ref().unwrap(),
        s.regions[0].old_png.as_ref().unwrap(),
        s.regions[0].new_png.as_ref().unwrap(),
    ] {
        let png = image::open(dir.join(name)).unwrap();
        assert!(png.width() > 0 && png.height() > 0);
    }
    assert_eq!(
        serde_json::from_str::<DiffReport>(&report.to_json().unwrap()).unwrap(),
        report
    );
    let mut region = serde_json::to_value(&s.regions[0]).unwrap();
    region.as_object_mut().unwrap().remove("nodes");
    assert!(
        serde_json::from_value::<figstudio_diff::Region>(region)
            .unwrap()
            .nodes
            .is_empty()
    );
    let mut json = serde_json::to_value(&report).unwrap();
    assert!(json["pages"][0]["render"].get("regions").is_none());
    assert!(json["pages"][0]["render"].get("moved_pure").is_none());
    assert!(
        json["pages"][0]["render"]
            .get("pixels_changed_effective")
            .is_none()
    );
    for key in [
        "screens",
        "screens_unchanged",
        "screens_moved",
        "other_nodes",
    ] {
        json["pages"][0].as_object_mut().unwrap().remove(key);
    }
    let legacy: DiffReport = serde_json::from_value(json).unwrap();
    assert!(legacy.pages[0].screens.is_empty());
    assert_eq!(
        (
            legacy.pages[0].screens_unchanged,
            legacy.pages[0].screens_moved
        ),
        (0, 0)
    );
    assert_eq!(legacy.pages[0].id, Guid(0, 1));
    std::fs::remove_dir_all(dir).unwrap();
}
