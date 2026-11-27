//! Text decorations (strikethrough and underline) for authoritative stored rects,
//! instance scaling and overrides, post-edit fallback shaping, paint inheritance,
//! clipping, and approximation counters.
mod common;
use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use figstudio_fig::edit::Txn;
use figstudio_fig::edit::text_runs::{TextStyleRunPatch, apply_text_range_style, commit_text_data};
use figstudio_fig::{Document, Guid, Val};
use figstudio_render::color::AlphaColor;
use figstudio_render::kurbo::Affine;
use figstudio_render::{Renderer, Viewport, node_bounds};

fn open_fixture(name: &str) -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(format!("{name}.fig")),
    )
    .unwrap()
}

fn rgba_paint(r: f64, g: f64, b: f64, a: f64, opacity: f64) -> Val {
    Val::Arr(vec![obj(&[
        ("type", Val::Str("SOLID".into())),
        ("visible", Val::Bool(true)),
        ("opacity", Val::Float(opacity)),
        (
            "color",
            obj(&[
                ("r", Val::Float(r)),
                ("g", Val::Float(g)),
                ("b", Val::Float(b)),
                ("a", Val::Float(a)),
            ]),
        ),
    ])])
}

fn dec_rect(x: f64, y: f64, w: f64, h: f64) -> Val {
    obj(&[
        ("x", Val::Float(x)),
        ("y", Val::Float(y)),
        ("w", Val::Float(w)),
        ("h", Val::Float(h)),
    ])
}

/// Thin vertical pillar outline in em units (`x: 0.0..0.2`, `y: 0.0..0.8`, y-up) so the
/// horizontal gap between consecutive glyphs has no glyph ink.
fn push_pillar_blob(doc: &mut Document) -> i64 {
    let idx = doc.blobs.len() as i64;
    let mut cmds = Vec::new();
    for (cmd, x, y) in [
        (1, 0.0f32, 0.0f32),
        (2, 0.2, 0.0),
        (2, 0.2, 0.8),
        (2, 0.0, 0.8),
    ] {
        cmds.push(cmd);
        cmds.extend(x.to_le_bytes());
        cmds.extend(y.to_le_bytes());
    }
    cmds.push(0);
    doc.blobs.push(cmds);
    idx
}

fn pillar_glyph(blob: i64, x: f64, y: f64, font_size: f64, first_char: i64, advance: f64) -> Val {
    obj(&[
        ("commandsBlob", Val::Int(blob)),
        ("fontSize", Val::Float(font_size)),
        ("position", size(x, y)),
        ("firstCharacter", Val::Int(first_char)),
        ("advance", Val::Float(advance)),
    ])
}

fn hack_font_name() -> Val {
    obj(&[
        ("family", Val::Str("Hack".into())),
        ("style", Val::Str("Regular".into())),
    ])
}

fn fallback_renderer() -> Renderer {
    let mut r = Renderer::new();
    r.add_font_data(Arc::new(epaint_default_fonts::HACK_REGULAR.to_vec()), 0);
    r
}

fn px_at(px: &[u8], w: u16, x: usize, y: usize) -> [u8; 4] {
    let i = (y * usize::from(w) + x) * 4;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

fn count_in_rect(
    px: &[u8],
    w: u16,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    pred: impl Fn([u8; 4]) -> bool,
) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if pred(px_at(px, w, x, y)) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn public_fixtures_render_stored_strikethrough_and_underline() {
    // 1. Public fixture `shaders.fig` node `1:12968`: 90%-alpha numeric "24" with stored
    // STRIKETHROUGH in `derivedTextData.decorations`.
    let shaders = open_fixture("shaders");
    let strike_id: Guid = "1:12968".parse().unwrap();
    let strike_bounds = node_bounds(&shaders, strike_id).unwrap();
    let scale = 4.0;
    let sw = (strike_bounds.width() * scale).ceil() as u16 + 4;
    let sh = (strike_bounds.height() * scale).ceil() as u16 + 4;
    let svp = Viewport {
        width: sw,
        height: sh,
        transform: Affine::scale(scale) * Affine::translate((-strike_bounds.x0, -strike_bounds.y0)),
        background: Some(AlphaColor::WHITE),
    };
    let mut r = Renderer::new();
    r.system_fonts = false;
    let direct = r.render(&shaders, strike_id, &svp);
    assert_eq!(direct.stats.text_stored_glyphs, 1);
    assert!(!direct.stats.to_string().contains("text decorations"));
    let spx = direct.pixmap.data_as_u8_slice();

    // Gap between '2' and '4' at node x ~= 6.0 (px 24), strikethrough y ~= 8.77 (px 34..37).
    let mut bar_min_r = u8::MAX;
    for y in 33..38 {
        for x in 23..26 {
            bar_min_r = bar_min_r.min(px_at(spx, sw, x, y)[0]);
        }
    }
    assert!(
        bar_min_r <= 45,
        "expected 90%-alpha strikethrough bar (~25 on white) in shaders.fig 1:12968 gap, got r={bar_min_r}"
    );
    // Above the bar in the same gap stays white.
    let gap_clear = count_in_rect(spx, sw, 23, 19, 26, 23, |p| p[0] < 245);
    assert_eq!(
        gap_clear, 0,
        "inter-digit gap outside strike band should stay white"
    );
    // Crossing uniformity: 90%-alpha black on white is ~25; double-composited crossings drop to ~2.
    let global_min_r = spx.as_chunks::<4>().0.iter().map(|p| p[0]).min().unwrap();
    assert!(
        global_min_r >= bar_min_r.saturating_sub(3),
        "translucent strikethrough crossing darkened: global_min_r={global_min_r}, bar_min_r={bar_min_r}"
    );

    #[cfg(feature = "parallel")]
    {
        let parallel = r.render_parallel(&shaders, strike_id, &svp);
        assert_eq!(spx, parallel.pixmap.data_as_u8_slice());
        let area = figstudio_render::kurbo::Rect::new(0.0, 0.0, f64::from(sw), f64::from(sh));
        let (rec, _) = r.record(&shaders, strike_id, svp.transform, area, usize::MAX);
        let (replayed, _) = rec.unwrap().render(&svp).unwrap();
        assert_eq!(spx, replayed.pixmap.data_as_u8_slice());
    }

    // 2. Public fixture `motion.fig` node `1:525`: stored UNDERLINE with skip-ink cutouts.
    let motion = open_fixture("motion");
    let ul_id: Guid = "1:525".parse().unwrap();
    let ul_bounds = node_bounds(&motion, ul_id).unwrap();
    let uw = ul_bounds.width().ceil() as u16 + 4;
    let uh = ul_bounds.height().ceil() as u16 + 4;
    let uvp = Viewport {
        width: uw,
        height: uh,
        transform: Affine::translate((-ul_bounds.x0, -ul_bounds.y0)),
        background: None,
    };
    let ul = r.render(&motion, ul_id, &uvp);
    assert_eq!(ul.stats.text_stored_glyphs, 1);
    assert!(!ul.stats.to_string().contains("text decorations"));
    let upx = ul.pixmap.data_as_u8_slice();
    let ul_ink = count_in_rect(upx, uw, 85, 77, 185, 82, |p| p[3] > 80);
    assert!(
        ul_ink > 100,
        "expected stored underline pixels in motion.fig 1:525, got {ul_ink}"
    );
    let skip_gap = count_in_rect(upx, uw, 67, 79, 72, 80, |p| p[3] > 200);
    assert_eq!(
        skip_gap, 0,
        "skip-ink gap from stored decorations must remain clear"
    );
}

#[test]
fn stored_decorations_paint_inheritance_utf16_clipping_and_stats() {
    let mut doc = document(vec![node(1, 0, "CANVAS")]);
    let blob = push_pillar_blob(&mut doc);

    // 1. Half-alpha stored bar crossing pillars inside a clipping frame (width 24):
    // uniform ~50% alpha inside frame, clipped cleanly outside x >= 24.
    let mut clip_frame = node(2, 1, "FRAME");
    clip_frame.data.set("size", size(24.0, 24.0));
    clip_frame.data.set("frameMaskDisabled", Val::Bool(false));
    let mut clipped_text = node(3, 2, "TEXT");
    clipped_text.data.set("size", size(60.0, 24.0));
    clipped_text.data.set("fontSize", Val::Float(20.0));
    clipped_text
        .data
        .set("textDecoration", Val::Enum("STRIKETHROUGH"));
    clipped_text
        .data
        .set("fillPaints", rgba_paint(0.0, 0.0, 1.0, 1.0, 0.5));
    clipped_text
        .data
        .set("textData", obj(&[("characters", Val::Str("AB".into()))]));
    clipped_text.data.set(
        "derivedTextData",
        obj(&[
            (
                "glyphs",
                Val::Arr(vec![
                    pillar_glyph(blob, 0.0, 20.0, 20.0, 0, 0.5),
                    pillar_glyph(blob, 30.0, 20.0, 20.0, 1, 0.5),
                ]),
            ),
            (
                "decorations",
                Val::Arr(vec![obj(&[
                    ("rects", Val::Arr(vec![dec_rect(0.0, 5.0, 40.0, 3.0)])),
                    ("styleID", Val::Int(0)),
                ])]),
            ),
        ]),
    );

    // 2. External library style (styleIdForText not in local tokens) with cached
    // textDecoration: NONE must STILL render authoritative stored decorations.
    let mut ext_text = node(4, 1, "TEXT");
    ext_text.data.set("size", size(60.0, 24.0));
    ext_text
        .data
        .set("transform", obj(&[("m12", Val::Float(30.0))]));
    ext_text.data.set("fontSize", Val::Float(20.0));
    ext_text.data.set("textDecoration", Val::Enum("NONE"));
    ext_text.data.set(
        "styleIdForText",
        obj(&[("key", Val::Str("external-lib-key".into()))]),
    );
    ext_text.data.set("fillPaints", paint(0.0, 0.7, 0.0));
    ext_text
        .data
        .set("textData", obj(&[("characters", Val::Str("AB".into()))]));
    ext_text.data.set(
        "derivedTextData",
        obj(&[
            (
                "glyphs",
                Val::Arr(vec![
                    pillar_glyph(blob, 0.0, 20.0, 20.0, 0, 0.5),
                    pillar_glyph(blob, 30.0, 20.0, 20.0, 1, 0.5),
                ]),
            ),
            (
                "decorations",
                Val::Arr(vec![obj(&[
                    ("rects", Val::Arr(vec![dec_rect(0.0, 8.0, 40.0, 3.0)])),
                    ("styleID", Val::Int(0)),
                ])]),
            ),
        ]),
    );

    // 3. UTF-16 vs scalar `firstCharacter` + explicit empty `textDecorationFillPaints: []`:
    // "😀12" -> UTF-16 units [0, 0, 1, 1]; glyph '1' has scalar firstCharacter=1 -> UTF-16 2 -> style 1.
    // Node has BLUE fill and GREEN textDecorationFillPaints; run 1 has RED fill and explicit []
    // textDecorationFillPaints -> must inherit RED run fill, NOT GREEN node decoration paint.
    let mut utf16_text = node(5, 1, "TEXT");
    utf16_text.data.set("size", size(90.0, 24.0));
    utf16_text
        .data
        .set("transform", obj(&[("m12", Val::Float(60.0))]));
    utf16_text.data.set("fontSize", Val::Float(20.0));
    utf16_text.data.set("fillPaints", paint(0.0, 0.0, 1.0));
    utf16_text
        .data
        .set("textDecorationFillPaints", paint(0.0, 0.8, 0.0));
    utf16_text.data.set(
        "textData",
        obj(&[
            ("characters", Val::Str("😀12".into())),
            (
                "characterStyleIDs",
                Val::Arr(vec![Val::Int(0), Val::Int(0), Val::Int(1), Val::Int(1)]),
            ),
            (
                "styleOverrideTable",
                Val::Arr(vec![obj(&[
                    ("styleID", Val::Int(1)),
                    ("fillPaints", paint(1.0, 0.0, 0.0)),
                    ("textDecoration", Val::Enum("STRIKETHROUGH")),
                    ("textDecorationFillPaints", Val::Arr(vec![])),
                ])]),
            ),
        ]),
    );
    utf16_text.data.set(
        "derivedTextData",
        obj(&[
            (
                "glyphs",
                Val::Arr(vec![
                    pillar_glyph(blob, 0.0, 20.0, 20.0, 0, 0.5),
                    pillar_glyph(blob, 20.0, 20.0, 20.0, 1, 0.5),
                    pillar_glyph(blob, 40.0, 20.0, 20.0, 2, 0.5),
                ]),
            ),
            (
                "decorations",
                Val::Arr(vec![obj(&[
                    ("rects", Val::Arr(vec![dec_rect(20.0, 10.0, 30.0, 3.0)])),
                    ("styleID", Val::Int(1)),
                ])]),
            ),
        ]),
    );

    doc.set_nodes(vec![
        node(1, 0, "CANVAS"),
        clip_frame,
        clipped_text,
        ext_text,
        utf16_text,
    ])
    .unwrap();

    let mut r = Renderer::new();
    r.system_fonts = false;
    let vp = Viewport {
        width: 90,
        height: 90,
        transform: Affine::IDENTITY,
        background: None,
    };
    let frame = r.render(&doc, id(1), &vp);
    assert!(!frame.stats.to_string().contains("text decorations"));
    let px = frame.pixmap.data_as_u8_slice();

    // 1. Half-alpha crossing uniformity + clipping at x=24.
    let bar = px_at(px, 90, 12, 6);
    let crossing = px_at(px, 90, 2, 6);
    assert!(
        (115..=140).contains(&bar[3]),
        "half-alpha bar should draw once, got {bar:?}"
    );
    assert!(
        bar[3].abs_diff(crossing[3]) <= 2,
        "clipped translucent crossing darkened: bar={bar:?} crossing={crossing:?}"
    );
    assert_eq!(
        count_in_rect(px, 90, 25, 0, 60, 24, |p| p[3] > 0),
        0,
        "clipping frame must clip decorated text beyond x=24"
    );

    // 2. External library style stored decoration stays rendered.
    let ext_bar = px_at(px, 90, 16, 39);
    assert!(
        ext_bar[1] > 140 && ext_bar[3] > 200,
        "external library stored decoration was dropped: {ext_bar:?}"
    );

    // 3. UTF-16 scalar mapping & explicit empty [] override.
    let g0 = px_at(px, 90, 2, 70);
    let g1 = px_at(px, 90, 22, 70);
    let d1 = px_at(px, 90, 32, 71);
    assert!(
        g0[2] > 200 && g0[0] < 40,
        "glyph 0 should be blue, got {g0:?}"
    );
    assert!(
        g1[0] > 200 && g1[2] < 40,
        "glyph '1' (scalar 1 -> UTF-16 2 -> style 1) should be red, got {g1:?}"
    );
    assert!(
        d1[0] > 200 && d1[1] < 40 && d1[2] < 40,
        "explicit empty textDecorationFillPaints: [] must inherit red run fill, not green node paint, got {d1:?}"
    );

    // 4. Stats policy: stored glyphs missing stored decorations, and WAVY stored decorations,
    // are counted instead of silently approximated.
    let mut missing_dec = node(6, 1, "TEXT");
    missing_dec.data.set("size", size(60.0, 24.0));
    missing_dec.data.set("fontSize", Val::Float(20.0));
    missing_dec
        .data
        .set("textDecoration", Val::Enum("STRIKETHROUGH"));
    missing_dec.data.set("fillPaints", paint(0.2, 0.2, 0.2));
    missing_dec
        .data
        .set("textData", obj(&[("characters", Val::Str("AB".into()))]));
    missing_dec.data.set(
        "derivedTextData",
        obj(&[(
            "glyphs",
            Val::Arr(vec![pillar_glyph(blob, 0.0, 20.0, 20.0, 0, 0.5)]),
        )]),
    );
    let mut wavy_dec = missing_dec.clone();
    wavy_dec.id = id(7);
    wavy_dec.data.set("guid", id(7).to_val());
    wavy_dec.data.set("textDecorationStyle", Val::Enum("WAVY"));
    wavy_dec.data.set(
        "derivedTextData",
        obj(&[
            (
                "glyphs",
                Val::Arr(vec![pillar_glyph(blob, 0.0, 20.0, 20.0, 0, 0.5)]),
            ),
            (
                "decorations",
                Val::Arr(vec![obj(&[
                    ("rects", Val::Arr(vec![dec_rect(0.0, 10.0, 20.0, 2.0)])),
                    ("styleID", Val::Int(0)),
                ])]),
            ),
        ]),
    );
    doc.set_nodes(vec![node(1, 0, "CANVAS"), missing_dec, wavy_dec])
        .unwrap();
    let stats_frame = r.render(&doc, id(1), &vp);
    assert_eq!(stats_frame.stats.approximated_text_decorations, 2);
    assert!(
        stats_frame
            .stats
            .to_string()
            .contains("2 text decorations approximated"),
        "expected 2 approximated text decorations in Stats, got: {}",
        stats_frame.stats
    );
}

#[test]
fn instance_overrides_and_uniform_scaling_on_stored_decorations() {
    let mut doc = document(vec![node(1, 0, "CANVAS")]);
    let blob = push_pillar_blob(&mut doc);

    // Local TEXT style with STRIKETHROUGH.
    let mut strike_style = node(20, 0, "STYLE");
    strike_style.data.set("styleType", Val::Enum("TEXT"));
    strike_style
        .data
        .set("textDecoration", Val::Enum("STRIKETHROUGH"));

    // Component 1: direct STRIKETHROUGH + stored glyphs + stored decorations.
    let mut sym1 = node(2, 1, "SYMBOL");
    sym1.data.set("size", size(40.0, 20.0));
    sym1.data.set("visible", Val::Bool(false));
    let mut text1 = node(3, 2, "TEXT");
    text1.data.set("size", size(40.0, 20.0));
    text1.data.set("fontSize", Val::Float(10.0));
    text1.data.set("textDecoration", Val::Enum("STRIKETHROUGH"));
    text1.data.set("fillPaints", paint(0.0, 0.6, 0.0));
    text1.data.set("fontName", hack_font_name());
    text1
        .data
        .set("textData", obj(&[("characters", Val::Str("1   1".into()))]));
    text1.data.set(
        "derivedTextData",
        obj(&[
            (
                "glyphs",
                Val::Arr(vec![
                    pillar_glyph(blob, 0.0, 10.0, 10.0, 0, 1.0),
                    pillar_glyph(blob, 15.0, 10.0, 10.0, 1, 1.0),
                ]),
            ),
            (
                "decorations",
                Val::Arr(vec![obj(&[
                    ("rects", Val::Arr(vec![dec_rect(0.0, 6.0, 25.0, 2.0)])),
                    ("styleID", Val::Int(0)),
                ])]),
            ),
        ]),
    );

    // Component 2: attached local TEXT style (STRIKETHROUGH) with stale cached textDecoration: NONE.
    let mut sym2 = node(4, 1, "SYMBOL");
    sym2.data.set("size", size(40.0, 20.0));
    sym2.data.set("visible", Val::Bool(false));
    let mut text2 = node(5, 4, "TEXT");
    text2.data.set("size", size(40.0, 20.0));
    text2.data.set("fontSize", Val::Float(10.0));
    text2.data.set("textDecoration", Val::Enum("NONE"));
    text2
        .data
        .set("styleIdForText", obj(&[("guid", id(20).to_val())]));
    text2.data.set("fillPaints", paint(0.0, 0.6, 0.0));
    text2.data.set("fontName", hack_font_name());
    text2
        .data
        .set("textData", obj(&[("characters", Val::Str("1   1".into()))]));
    text2.data.set(
        "derivedTextData",
        text1.data.get("derivedTextData").unwrap().clone(),
    );

    // Instance A (x=0, y=0): explicit NONE override on Component 1 -> removes bar, KEEPS exact stored glyphs.
    let inst_none_direct = instance(
        6,
        1,
        2,
        vec![patch(&[3], &[("textDecoration", Val::Enum("NONE"))])],
    );

    // Instance B (x=0, y=25): explicit NONE override detaching Component 2's local TEXT style ->
    // removes bar, KEEPS exact stored glyphs.
    let mut inst_none_styled = instance(
        7,
        1,
        4,
        vec![patch(&[5], &[("textDecoration", Val::Enum("NONE"))])],
    );
    inst_none_styled
        .data
        .set("transform", obj(&[("m12", Val::Float(25.0))]));

    // Instance C (x=40, y=0): uniformScaleFactor: 2.0 -> scales stored decoration rect 2x.
    let mut inst_scaled = instance(8, 1, 2, vec![]);
    inst_scaled.data.set(
        "symbolData",
        obj(&[
            ("symbolID", id(2).to_val()),
            ("uniformScaleFactor", Val::Float(2.0)),
        ]),
    );
    inst_scaled.data.set("size", size(80.0, 40.0));
    inst_scaled
        .data
        .set("transform", obj(&[("m02", Val::Float(40.0))]));

    // Instance D (x=0, y=50): override with fresh derivedSymbolData.derivedTextData -> draws fresh bar at y=2..4.
    let mut inst_fresh = instance(
        9,
        1,
        2,
        vec![patch(&[3], &[("textDecoration", Val::Enum("UNDERLINE"))])],
    );
    inst_fresh
        .data
        .set("transform", obj(&[("m12", Val::Float(50.0))]));
    inst_fresh.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[3],
            &[(
                "derivedTextData",
                obj(&[
                    (
                        "glyphs",
                        Val::Arr(vec![
                            pillar_glyph(blob, 0.0, 10.0, 10.0, 0, 1.0),
                            pillar_glyph(blob, 15.0, 10.0, 10.0, 1, 1.0),
                        ]),
                    ),
                    (
                        "decorations",
                        Val::Arr(vec![obj(&[
                            ("rects", Val::Arr(vec![dec_rect(0.0, 2.0, 25.0, 2.0)])),
                            ("styleID", Val::Int(0)),
                        ])]),
                    ),
                ]),
            )],
        )]),
    );

    // First verify Instances A-D with `system_fonts = false`: proves NONE overrides preserved exact
    // custom-font stored outlines instead of discarding them to system fonts.
    doc.set_nodes(vec![
        node(1, 0, "CANVAS"),
        strike_style,
        sym1.clone(),
        text1.clone(),
        sym2,
        text2,
        inst_none_direct,
        inst_none_styled,
        inst_scaled,
        inst_fresh,
    ])
    .unwrap();

    let mut r = Renderer::new();
    r.system_fonts = false;
    let vp = Viewport {
        width: 120,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    };
    let frame = r.render(&doc, id(1), &vp);
    assert_eq!(frame.stats.text_stored_glyphs, 4);
    assert_eq!(frame.stats.text_system_fonts, 0);
    let px = frame.pixmap.data_as_u8_slice();

    // Instance A (y=0): pillar 0 at x=0..2 stays drawn; gap x=5..12 has no bar.
    assert!(
        count_in_rect(px, 120, 0, 2, 2, 10, |p| p[3] > 200) > 8,
        "explicit NONE override must preserve stored custom-font pillar glyphs"
    );
    assert_eq!(
        count_in_rect(px, 120, 5, 4, 13, 10, |p| p[3] > 0),
        0,
        "explicit NONE override must suppress stale stored decoration"
    );

    // Instance B (y=25): detached local TEXT style -> pillar stays drawn, stale bar removed.
    assert!(
        count_in_rect(px, 120, 0, 27, 2, 35, |p| p[3] > 200) > 8,
        "explicit NONE detaching local TEXT style must preserve stored glyphs"
    );
    assert_eq!(
        count_in_rect(px, 120, 5, 29, 13, 35, |p| p[3] > 0),
        0,
        "explicit NONE detaching local TEXT style must suppress stale decoration"
    );

    // Instance C (x=40, scale 2x): stored rect (0, 6, 25, 2) scales to (40..90, 12..16).
    let scaled_bar = count_in_rect(px, 120, 48, 12, 66, 16, |p| p[1] > 100 && p[3] > 200);
    assert!(
        scaled_bar >= 60,
        "uniformScaleFactor must scale stored decoration rects, got {scaled_bar}"
    );
    assert_eq!(
        count_in_rect(px, 120, 48, 6, 66, 9, |p| p[3] > 0),
        0,
        "unscaled decoration coordinates must not be drawn"
    );

    // Instance D (y=50): fresh derived decoration renders at y=52..54, old y=56..58 is empty.
    assert!(
        count_in_rect(px, 120, 5, 52, 13, 54, |p| p[3] > 200) >= 12,
        "fresh derivedSymbolData.derivedTextData must preserve fresh stored decorations"
    );
    assert_eq!(
        count_in_rect(px, 120, 5, 56, 13, 59, |p| p[3] > 0),
        0,
        "old component decoration position must not remain after fresh derived override"
    );

    // Now verify non-NONE override (NONE -> STRIKETHROUGH) without fresh derivedTextData falls back
    // to system fonts (`text_system_fonts == 1`) and draws the requested strikethrough.
    let mut sym_plain = sym1;
    let mut text_plain = text1;
    text_plain.data.set("size", size(120.0, 36.0));
    text_plain.data.set("fontSize", Val::Float(24.0));
    text_plain.data.set("textDecoration", Val::Enum("NONE"));
    text_plain.data.set(
        "derivedTextData",
        obj(&[(
            "glyphs",
            Val::Arr(vec![pillar_glyph(blob, 0.0, 20.0, 20.0, 0, 1.0)]),
        )]),
    );
    sym_plain.data.set("size", size(120.0, 36.0));
    let inst_add = instance(
        10,
        1,
        2,
        vec![patch(
            &[3],
            &[("textDecoration", Val::Enum("STRIKETHROUGH"))],
        )],
    );
    doc.set_nodes(vec![node(1, 0, "CANVAS"), sym_plain, text_plain, inst_add])
        .unwrap();
    let mut rf = fallback_renderer();
    let add_vp = Viewport {
        width: 120,
        height: 40,
        transform: Affine::IDENTITY,
        background: None,
    };
    let add_frame = rf.render(&doc, id(1), &add_vp);
    assert_eq!(add_frame.stats.text_system_fonts, 1);
    assert!(
        count_in_rect(
            add_frame.pixmap.data_as_u8_slice(),
            120,
            20,
            8,
            52,
            21,
            |p| p[3] > 80
        ) > 20,
        "non-NONE override without fresh derivedTextData must render via fallback shaping"
    );
}

#[test]
fn edited_text_and_fallback_shaping_decorations_and_precedence() {
    // 1. Real PR #90 edit path: stored-glyph TEXT edited via `apply_text_range_style` +
    // `commit_text_data` clears `derivedTextData` and renders via fallback shaping.
    let mut edited = node(2, 1, "TEXT");
    edited.data.set("size", size(220.0, 36.0));
    edited.data.set("fontSize", Val::Float(24.0));
    edited.data.set("fillPaints", paint(0.4, 0.4, 0.4));
    edited.data.set("fontName", hack_font_name());
    edited.data.set(
        "textData",
        obj(&[("characters", Val::Str("1   1   1".into()))]),
    );
    let mut edit_doc = document(vec![node(1, 0, "CANVAS"), edited.clone()]);
    let blob = push_pillar_blob(&mut edit_doc);
    edited.data.set(
        "derivedTextData",
        obj(&[(
            "glyphs",
            Val::Arr(vec![pillar_glyph(blob, 0.0, 20.0, 24.0, 0, 0.6)]),
        )]),
    );
    edit_doc
        .set_nodes(vec![node(1, 0, "CANVAS"), edited])
        .unwrap();

    let before_td = edit_doc
        .node(id(2))
        .unwrap()
        .data
        .get("textData")
        .unwrap()
        .clone();
    let new_td = apply_text_range_style(
        &before_td,
        0,
        5,
        &TextStyleRunPatch {
            fill_hex: Some("#ff0000".into()),
            text_decoration: Some("STRIKETHROUGH".into()),
            ..TextStyleRunPatch::default()
        },
    )
    .unwrap();
    let mut txn = Txn::new("strikethrough run");
    commit_text_data(&edit_doc, &mut txn, id(2), new_td).unwrap();
    edit_doc.apply_txn(&txn).unwrap();
    assert!(
        edit_doc
            .node(id(2))
            .unwrap()
            .data
            .get("derivedTextData")
            .is_none(),
        "real text edit must clear derivedTextData"
    );

    let mut r = fallback_renderer();
    let edit_vp = Viewport {
        width: 220,
        height: 40,
        transform: Affine::IDENTITY,
        background: None,
    };
    let edit_frame = r.render(&edit_doc, id(1), &edit_vp);
    assert_eq!(edit_frame.stats.text_system_fonts, 1);
    let epx = edit_frame.pixmap.data_as_u8_slice();
    assert!(
        count_in_rect(epx, 220, 20, 8, 52, 21, |p| p[0] > 180 && p[3] > 80) > 20,
        "edited run 0..5 must render red strikethrough bar"
    );
    assert_eq!(
        count_in_rect(epx, 220, 78, 8, 110, 21, |p| p[3] > 20),
        0,
        "unedited suffix run 5..9 must stay clear"
    );

    // 2. Fallback shaping cases: local TEXT style precedence over stale cached NONE, explicit
    // underline_offset = 0, NONE hole override, tracked adjacent decorated runs (letterSpacing),
    // DOTTED Stats count, and 90-degree rotation.
    let mut strike_style = node(30, 0, "STYLE");
    strike_style.data.set("styleType", Val::Enum("TEXT"));
    strike_style
        .data
        .set("textDecoration", Val::Enum("STRIKETHROUGH"));

    let make_text = |n: u32, y: f64| {
        let mut t = node(n, 1, "TEXT");
        t.data.set("size", size(260.0, 36.0));
        t.data.set("transform", obj(&[("m12", Val::Float(y))]));
        t.data.set("fontSize", Val::Float(24.0));
        t.data.set("fillPaints", paint(0.5, 0.5, 0.5));
        t.data.set("fontName", hack_font_name());
        t.data
            .set("textData", obj(&[("characters", Val::Str("1   1".into()))]));
        t
    };

    // y=0: attached local TEXT style overrides stale cached textDecoration: NONE.
    let mut styled = make_text(2, 0.0);
    styled.data.set("textDecoration", Val::Enum("NONE"));
    styled
        .data
        .set("styleIdForText", obj(&[("guid", id(30).to_val())]));

    // y=40: explicit underline offset 0 px sits at baseline (~y=22) instead of default (~y=26+).
    let mut ul_zero = make_text(3, 40.0);
    ul_zero.data.set("textDecoration", Val::Enum("UNDERLINE"));
    ul_zero.data.set(
        "textUnderlineOffset",
        obj(&[("units", Val::Enum("PIXELS")), ("value", Val::Float(0.0))]),
    );
    // y=80: default underline for direct comparison with explicit 0 px.
    let mut ul_default = make_text(4, 80.0);
    ul_default
        .data
        .set("textDecoration", Val::Enum("UNDERLINE"));

    // y=120: node STRIKETHROUGH with middle NONE override punching a hole.
    let mut hole = make_text(5, 120.0);
    hole.data.set("textDecoration", Val::Enum("STRIKETHROUGH"));
    hole.data.set(
        "textData",
        obj(&[
            ("characters", Val::Str("1   1   1   1".into())),
            (
                "characterStyleIDs",
                Val::Arr(
                    [0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0]
                        .into_iter()
                        .map(Val::Int)
                        .collect(),
                ),
            ),
            (
                "styleOverrideTable",
                Val::Arr(vec![obj(&[
                    ("styleID", Val::Int(1)),
                    ("textDecoration", Val::Enum("NONE")),
                ])]),
            ),
        ]),
    );

    // y=160: adjacent decorated runs with large positive letterSpacing (12 px) must connect
    // continuously across the run boundary (no unstroked crack!).
    let mut tracked = make_text(6, 160.0);
    tracked.data.set(
        "letterSpacing",
        obj(&[("units", Val::Enum("PIXELS")), ("value", Val::Float(12.0))]),
    );
    tracked.data.set(
        "textData",
        obj(&[
            ("characters", Val::Str("11".into())),
            (
                "characterStyleIDs",
                Val::Arr(vec![Val::Int(1), Val::Int(2)]),
            ),
            (
                "styleOverrideTable",
                Val::Arr(vec![
                    obj(&[
                        ("styleID", Val::Int(1)),
                        ("fillPaints", paint(1.0, 0.0, 0.0)),
                        ("textDecoration", Val::Enum("STRIKETHROUGH")),
                    ]),
                    obj(&[
                        ("styleID", Val::Int(2)),
                        ("fillPaints", paint(0.0, 0.0, 1.0)),
                        ("textDecoration", Val::Enum("STRIKETHROUGH")),
                    ]),
                ]),
            ),
        ]),
    );

    // y=200: DOTTED decoration draws fallback bar and increments approximated_text_decorations.
    let mut dotted = make_text(7, 200.0);
    dotted
        .data
        .set("textDecoration", Val::Enum("STRIKETHROUGH"));
    dotted.data.set("textDecorationStyle", Val::Enum("DOTTED"));

    // 90-degree rotated text at x=230, y=5 (local (x, y) -> page (230 - y, 5 + x)).
    let mut rotated = make_text(8, 0.0);
    rotated.data.set("size", size(100.0, 36.0));
    rotated
        .data
        .set("textDecoration", Val::Enum("STRIKETHROUGH"));
    rotated.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(0.0)),
            ("m01", Val::Float(-1.0)),
            ("m02", Val::Float(230.0)),
            ("m10", Val::Float(1.0)),
            ("m11", Val::Float(0.0)),
            ("m12", Val::Float(5.0)),
        ]),
    );

    let doc = document(vec![
        node(1, 0, "CANVAS"),
        strike_style,
        styled,
        ul_zero,
        ul_default,
        hole,
        tracked,
        dotted,
        rotated,
    ]);
    let vp = Viewport {
        width: 260,
        height: 240,
        transform: Affine::IDENTITY,
        background: None,
    };
    let frame = r.render(&doc, id(1), &vp);
    assert_eq!(frame.stats.approximated_text_decorations, 1);
    assert!(
        frame
            .stats
            .to_string()
            .contains("1 text decorations approximated"),
        "DOTTED fallback decoration must be counted in Stats, got: {}",
        frame.stats
    );
    let px = frame.pixmap.data_as_u8_slice();

    // Attached local TEXT style beat stale cached NONE.
    assert!(
        count_in_rect(px, 260, 20, 8, 52, 21, |p| p[3] > 80) > 20,
        "attached local TEXT style must override stale cached textDecoration: NONE"
    );

    // Explicit underline_offset = 0 sits visibly higher than default underline.
    let first_bar_row = |y_base: usize| -> usize {
        (0..34)
            .find(|&dy| {
                count_in_rect(px, 260, 20, y_base + dy, 52, y_base + dy + 1, |p| p[3] > 80) > 20
            })
            .expect("underline bar row")
    };
    let zero_row = first_bar_row(40);
    let def_row = first_bar_row(80);
    assert!(
        zero_row + 2 <= def_row,
        "explicit 0 px underline offset ({zero_row}) must sit higher than default ({def_row})"
    );

    // NONE hole override on node 5 (y=120).
    assert!(count_in_rect(px, 260, 20, 128, 52, 141, |p| p[3] > 80) > 20);
    assert_eq!(count_in_rect(px, 260, 78, 128, 110, 141, |p| p[3] > 20), 0);
    assert!(count_in_rect(px, 260, 136, 128, 168, 141, |p| p[3] > 80) > 20);

    // Tracked adjacent decorated runs (y=160): char 0 advance ~14.45 + 12px spacing -> gap at x=16..24
    // must carry red strikethrough continuously up to char 1.
    assert!(
        count_in_rect(px, 260, 16, 168, 24, 181, |p| p[0] > 180 && p[3] > 80) >= 8,
        "tracked letterSpacing between adjacent decorated runs must not leave a crack"
    );

    // Rotated text bar at page x=209..222, y=25..57.
    assert!(
        count_in_rect(px, 260, 209, 25, 222, 57, |p| p[3] > 80) > 20,
        "rotated text must rotate its strikethrough bar into page coordinates"
    );

    #[cfg(feature = "parallel")]
    {
        let par = r.render_parallel(&doc, id(1), &vp);
        assert_eq!(px, par.pixmap.data_as_u8_slice());
    }
}
