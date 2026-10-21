//! Supplied fonts work without installed-font discovery, including late arrival.
mod common;
use common::*;
use figstudio_fig::Val;
use figstudio_render::Renderer;
#[cfg(not(feature = "system-fonts"))]
use {figstudio_render::Viewport, std::sync::Arc};

fn text_doc(count: u32) -> figstudio_fig::Document {
    let mut nodes = vec![node(1, 0, "CANVAS")];
    for i in 0..count {
        let mut text = node(i + 2, 1, "TEXT");
        text.data.set("size", size(200.0, 40.0));
        text.data.set("fontSize", Val::Float(28.0));
        text.data.set("fillPaints", paint(1.0, 0.0, 0.0));
        text.data.set(
            "textData",
            obj(&[("characters", Val::Str("Sample".into()))]),
        );
        text.data.set(
            "fontName",
            obj(&[
                ("family", Val::Str("Unavailable family".into())),
                ("style", Val::Str("Regular".into())),
                ("postscript", Val::Str("Unavailable-Regular".into())),
            ]),
        );
        nodes.push(text);
    }
    document(nodes)
}

#[test]
#[cfg(not(feature = "system-fonts"))]
fn registered_fonts_replace_missing_text_in_pixels_and_recordings() {
    let doc = text_doc(1);
    let viewport = Viewport {
        width: 200,
        height: 40,
        transform: figstudio_render::kurbo::Affine::IDENTITY,
        background: None,
    };
    let mut renderer = Renderer::new();
    let absent = renderer.render(&doc, id(1), &viewport);
    assert_eq!(absent.stats.text_missing, 1);
    assert!(absent.pixmap.data_as_u8_slice().iter().all(|&b| b == 0));
    renderer.add_font_data(Arc::new(epaint_default_fonts::HACK_REGULAR.to_vec()));
    let present = renderer.render(&doc, id(1), &viewport);
    assert_eq!(present.stats.text_missing, 0);
    assert_eq!(present.stats.text_system_fonts, 1);
    assert!(
        present
            .pixmap
            .data_as_u8_slice()
            .chunks_exact(4)
            .filter(|p| p[0] > 50 && p[3] > 50)
            .count()
            > 100
    );
    #[cfg(feature = "parallel")]
    {
        let recorded = renderer.render_parallel(&doc, id(1), &viewport);
        assert_eq!(
            present.pixmap.data_as_u8_slice(),
            recorded.pixmap.data_as_u8_slice()
        );
        assert_eq!(recorded.stats.text_missing, 0);
    }
}

#[test]
#[ignore = "manual timing and browser input generation"]
fn outline_less_text_measurement() {
    let doc = text_doc(500);
    if let Ok(path) = std::env::var("FIGSTUDIO_TEXT_FIXTURE") {
        let mut fixture = text_doc(1);
        let mut nodes = fixture.nodes.clone();
        nodes[0].data.set("type", Val::Enum("CANVAS"));
        nodes[1].data.set("type", Val::Enum("TEXT"));
        let mut paints = paint(1.0, 0.0, 0.0);
        if let Val::Arr(paints) = &mut paints {
            paints[0].set("type", Val::Enum("SOLID"));
        }
        nodes[1].data.set("fillPaints", paints);
        let root = figstudio_fig::Node::from_data(obj(&[
            ("guid", id(0).to_val()),
            ("type", Val::Enum("DOCUMENT")),
        ]))
        .unwrap();
        nodes.push(root);
        nodes[1].data.set(
            "textData",
            obj(&[("characters", Val::Str("あいうえお".into()))]),
        );
        fixture.set_nodes(nodes).unwrap();
        std::fs::write(path, fixture.to_bytes().unwrap()).unwrap();
    }
    let mut renderer = Renderer::new();
    let mut samples = Vec::new();
    for i in 0..16 {
        let start = std::time::Instant::now();
        let frame = renderer.render_fit(&doc, id(1), 1.0, 0.0, None).unwrap();
        std::hint::black_box(frame);
        if i > 0 {
            samples.push(start.elapsed());
        }
    }
    samples.sort();
    println!(
        "500 outline-less labels, warm median of 15: {:?}",
        samples[7]
    );
}
