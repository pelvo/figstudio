//! An attached paint style wins over the node's stored (possibly stale) paints.
mod common;
use common::*;
use figstudio_fig::{Node, Val};

fn style(n: u32, key: &str, r: f64, g: f64, b: f64) -> Node {
    let mut s = node(n, 0, "RECTANGLE");
    s.data.set("styleType", Val::Str("FILL".into()));
    s.data.set("key", Val::Str(key.into()));
    s.data.set("version", Val::Str("v1".into()));
    s.data.set("fillPaints", paint(r, g, b));
    s
}

fn styled_rect(reference: Val) -> Node {
    let mut rect = node(3, 2, "RECTANGLE");
    rect.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    rect.data.set("styleIdForFill", reference);
    rect
}

fn render(rect: Node, patches: Vec<Val>) -> Vec<u8> {
    pixels(&document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        rect,
        style(7, "green", 0.0, 1.0, 0.0),
        style(8, "blue", 0.0, 0.0, 1.0),
        instance(4, 1, 2, patches),
    ]))
}

#[test]
fn fill_styles_by_guid_or_library_asset_replace_stale_stored_paints() {
    let by_guid = styled_rect(obj(&[("guid", id(7).to_val())]));
    assert_eq!(pixel(&render(by_guid, vec![]), 10, 10), &[0, 255, 0, 255]);

    let asset = obj(&[(
        "assetRef",
        obj(&[
            ("key", Val::Str("blue".into())),
            ("version", Val::Str("v1".into())),
        ]),
    )]);
    let by_asset = styled_rect(asset);
    assert_eq!(pixel(&render(by_asset, vec![]), 10, 10), &[0, 0, 255, 255]);

    // Detached sentinel or an unknown style keeps the stored paints.
    let detached = styled_rect(obj(&[(
        "guid",
        figstudio_fig::Guid(u32::MAX, u32::MAX).to_val(),
    )]));
    assert_eq!(
        pixel(&render(detached, vec![]), 10, 10),
        &[128, 128, 128, 255]
    );
}

#[test]
fn an_instance_paint_override_without_a_style_beats_the_inherited_style() {
    let rect = styled_rect(obj(&[("guid", id(7).to_val())]));
    let red = patch(&[3], &[("fillPaints", paint(1.0, 0.0, 0.0))]);
    assert_eq!(
        pixel(&render(rect.clone(), vec![red]), 10, 10),
        &[255, 0, 0, 255]
    );
    // An override that switches the style itself uses the new style.
    let restyle = patch(
        &[3],
        &[("styleIdForFill", obj(&[("guid", id(8).to_val())]))],
    );
    assert_eq!(
        pixel(&render(rect, vec![restyle]), 10, 10),
        &[0, 0, 255, 255]
    );
}
