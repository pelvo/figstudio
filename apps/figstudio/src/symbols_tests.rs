use super::*;

#[test]
fn a_document_opened_during_a_failed_fetch_gets_exactly_one_retry() {
    let mut state = LoadState::default();
    assert!(state.begin(Content::Document(1)));
    assert!(!state.begin(Content::Document(2)));
    assert!(!state.begin(Content::Document(2)));
    assert!(state.finish(false), "first failure warns");
    assert!(state.pending, "overlapping document must start a retry");
    assert!(!state.finish(false), "retry failure does not warn twice");
    assert!(!state.pending, "no tight retry loop");
    assert!(!state.begin(Content::Document(2)));
    assert!(
        state.begin(Content::Document(3)),
        "a later document may retry again"
    );
}

#[test]
fn a_successful_pending_fetch_satisfies_overlapping_documents() {
    let mut state = LoadState::default();
    assert!(state.begin(Content::Document(1)));
    assert!(!state.begin(Content::Document(2)));
    assert!(!state.finish(true));
    assert!(!state.pending);
    assert!(!state.begin(Content::Document(3)));
}

#[test]
fn symbols_and_emoji_need_the_font_but_latin_and_cjk_do_not() {
    for name in [
        "└ layer",
        "→ next",
        "⚠ status",
        "📄 page",
        "∑ formula",
        "⠁ braille",
    ] {
        assert!(needs(name), "{name}");
    }
    for name in [
        "",
        "Page 12",
        "Latin abc",
        "漢字",
        "かな",
        "한글",
        "。",
        "\u{200d}\u{fe0f}",
    ] {
        assert!(!needs(name), "{name}");
    }
}

fn tick(ctx: &egui::Context) {
    ctx.run_ui(Default::default(), |_| {})
        .textures_delta
        .clear();
}

#[test]
fn fallback_preserves_cjk_in_either_load_order_and_rejects_bad_fonts() {
    for cjk_first in [true, false] {
        let ctx = egui::Context::default();
        tick(&ctx);
        let faces = std::array::from_fn(|_| epaint_default_fonts::EMOJI_ICON.to_vec());
        if cjk_first {
            crate::theme::add_cjk_font(&ctx, epaint_default_fonts::HACK_REGULAR.to_vec());
            tick(&ctx);
        }
        install(&ctx, faces).unwrap();
        tick(&ctx);
        if !cjk_first {
            crate::theme::add_cjk_font(&ctx, epaint_default_fonts::HACK_REGULAR.to_vec());
            tick(&ctx);
        }
        ctx.fonts(|fonts| {
            for family in [
                FontFamily::Proportional,
                FontFamily::Monospace,
                FontFamily::Name(crate::theme::SEMIBOLD.into()),
            ] {
                let names = &fonts.definitions().families[&family];
                let cjk = names.iter().position(|n| n == "cjk").unwrap();
                assert!(
                    names
                        .iter()
                        .position(|n| n.starts_with("web-symbols-"))
                        .unwrap()
                        > cjk
                );
            }
        });
        assert!(install(&ctx, Default::default()).is_err());
        tick(&ctx);
    }
}

#[test]
fn recents_label_changes_share_fetch_retry_and_toast_state_without_a_document() {
    let ctx = egui::Context::default();
    tick(&ctx);
    let mut state = LoadState::default();
    assert!(!state.begin_labels(&ctx, &["Latin title".into()]));
    let first = vec!["└ title".into()];
    assert!(state.begin_labels(&ctx, &first));
    assert!(!state.begin_labels(&ctx, &first));
    let later = vec!["└ title".into(), "└ another".into()];
    assert!(!state.begin_labels(&ctx, &later));
    assert!(!state.begin_labels(&ctx, &later.iter().cloned().rev().collect::<Vec<_>>()));
    assert!(state.finish(false), "one toast");
    assert!(state.pending, "one queued retry");
    assert!(!state.finish(false), "not a second toast");
    assert!(!state.pending);
    assert!(!state.begin_labels(&ctx, &later), "not a retry loop");
    assert!(
        state.begin(Content::Document(1)),
        "a later document may retry"
    );
    assert!(!state.finish(true));
    assert!(
        !state.begin_labels(&ctx, &first),
        "installed faces serve every label"
    );
}

#[test]
fn covered_symbols_do_not_trigger_a_download() {
    let ctx = egui::Context::default();
    tick(&ctx);
    let doc =
        figstudio_fig::Document::from_bytes(include_bytes!("../../../tests/fixtures/corners.fig"))
            .unwrap();
    assert!(!missing(&ctx, &doc, "Latin page"));
    assert!(missing(&ctx, &doc, "📄 page └"));
    assert!(!missing(&ctx, &doc, "★"));
    assert!(
        !missing(&ctx, &doc, "😀"),
        "bundled emoji must not fetch fonts"
    );
    assert!(missing(&ctx, &doc, "└"));
}

#[test]
fn property_and_find_text_is_scanned_for_missing_symbols() {
    let ctx = egui::Context::default();
    tick(&ctx);
    let mut doc =
        figstudio_fig::Document::from_bytes(include_bytes!("../../../tests/fixtures/corners.fig"))
            .unwrap();
    doc.nodes[0].data.set(
        "textData",
        figstudio_fig::Val::Obj({
            let mut text = figstudio_fig::Obj::new();
            text.insert("characters", figstudio_fig::Val::Str("└".into()));
            text
        }),
    );
    assert!(missing(&ctx, &doc, "Latin page"));
}

#[test]
fn failed_symbol_load_retries_only_for_a_new_document_and_warns_once() {
    let mut state = LoadState::default();
    assert!(state.begin(Content::Document(1)));
    assert!(!state.begin(Content::Document(1)));
    assert!(state.finish(false));
    for _ in 0..10 {
        assert!(!state.begin(Content::Document(1)));
    }
    assert!(state.begin(Content::Document(2)));
    assert!(!state.finish(false));
    for _ in 0..10 {
        assert!(!state.begin(Content::Document(2)));
    }
    assert!(state.begin(Content::Document(3)));
    assert!(!state.finish(true));
    assert!(!state.begin(Content::Document(4)));
}
