mod support;
use figstudio_diff::{
    MAX_COMPONENT_VIEWS, RenderOpts, ScreenLimits, attach_render_diffs, diff_documents,
    render_page_pair,
};
use figstudio_fig::{Document, Guid, Node, Val};
use support::{filled_rectangle_at, node, object, two_page_document};

const COMPONENTS: u32 = MAX_COMPONENT_VIEWS as u32 + 6;
const SCREENS: u32 = 14;

fn instance(id: u32, parent: u32, x: f64, symbol: u32, new: bool) -> Node {
    let mut n = node(id, Some(parent), "INSTANCE", "a");
    n.data.set(
        "size",
        object(&[("x", Val::Float(60.0)), ("y", Val::Float(40.0))]),
    );
    n.data.set(
        "transform",
        object(&[("m02", Val::Float(x)), ("m12", Val::Float(20.0))]),
    );
    n.data.set(
        "symbolData",
        object(&[("symbolID", Guid(0, symbol).to_val())]),
    );
    n.data.set(
        "derivedSymbolData",
        Val::Arr(vec![object(&[(
            "fillGeometry",
            Val::Arr(vec![object(&[("styleID", Val::Int(i64::from(new)))])]),
        )])]),
    );
    Node::from_data(n.data).unwrap()
}

/// Component `k` is used on screens `k % 7` and `k % 7 + 7`, ten instances per screen.
fn pair(components: u32, master_size: (f64, f64)) -> (Document, Document) {
    pair_on(components, master_size, false)
}

/// [`pair`], with each screen on its own page (canvas `200 + s`) when `spread`.
fn pair_on(components: u32, master_size: (f64, f64), spread: bool) -> (Document, Document) {
    let side = |new: bool| {
        let mut page = Vec::new();
        for s in 0..SCREENS {
            if spread {
                page.push(node(200 + s, Some(0), "CANVAS", &format!("c{s:02}")));
            }
            let mut frame = filled_rectangle_at(
                100 + s,
                if spread { 200 + s } else { 1 },
                (1.0, 1.0, 1.0),
                f64::from(s) * 1500.0,
                0.0,
                1400.0,
                200.0,
            );
            frame.data.set("type", Val::Str("FRAME".into()));
            page.push(Node::from_data(frame.data).unwrap());
        }
        let mut next = 1000;
        let mut symbols = Vec::new();
        for k in 0..components {
            let symbol = 5000 + k * 2;
            let color = if new {
                (0.0, 1.0, 0.0)
            } else {
                (1.0, 0.0, 0.0)
            };
            let mut master = node(symbol, Some(11), "SYMBOL", "a");
            master.data.set(
                "size",
                object(&[
                    ("x", Val::Float(master_size.0)),
                    ("y", Val::Float(master_size.1)),
                ]),
            );
            symbols.push(Node::from_data(master.data).unwrap());
            symbols.push(filled_rectangle_at(
                symbol + 1,
                symbol,
                color,
                0.0,
                0.0,
                60.0,
                40.0,
            ));
            for screen in [k % 7, k % 7 + 7] {
                page.push(instance(
                    next,
                    100 + screen,
                    20.0 + f64::from(k / 7) * 130.0,
                    symbol,
                    new,
                ));
                next += 1;
            }
        }
        page.sort_by_key(|n| n.id);
        two_page_document(page, symbols)
    };
    (side(false), side(true))
}

#[test]
fn component_views_stop_at_the_global_count_and_the_rest_keep_their_crop() {
    let (old, new) = pair(COMPONENTS, (60.0, 40.0));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let with_view = report
        .shared_changes
        .iter()
        .filter(|s| s.component_view)
        .count();
    assert!(report.shared_changes.len() >= COMPONENTS as usize);
    assert_eq!(with_view, MAX_COMPONENT_VIEWS);
    let note = report
        .truncated
        .iter()
        .find(|n| n.contains("component view limit"))
        .unwrap();
    assert!(
        note.ends_with(
            "6 component views were not drawn: 6 cards show the changed-area crop instead"
        ),
        "{note}"
    );
    // A card without a component view keeps the representative changed-area crop.
    let plain: Vec<_> = report
        .shared_changes
        .iter()
        .filter(|s| !s.component_view)
        .collect();
    assert!(plain.len() >= 6 && plain.iter().all(|s| s.image_width.is_some()));
}

#[test]
fn a_card_past_the_view_limit_with_no_crop_kept_is_left_out_and_counted() {
    let (old, new) = pair_on(COMPONENTS, (60.0, 40.0), true);
    // Each page holds its whole-page pictures and then one screen, with no room for crops.
    let page_bytes = (0..SCREENS)
        .filter_map(|s| {
            let page = Some(Guid(0, 200 + s));
            let (_, images) = render_page_pair(&old, &new, page, page, &RenderOpts::default())?;
            let len = |p: &Option<Vec<u8>>| p.as_ref().map_or(0, Vec::len);
            Some(len(&images.before) + len(&images.after) + len(&images.diff))
        })
        .max()
        .unwrap()
        + 1;
    let opts = RenderOpts {
        screen_limits: ScreenLimits {
            page_bytes,
            ..ScreenLimits::default()
        },
        ..RenderOpts::default()
    };
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &opts);
    let note = report
        .truncated
        .iter()
        .find(|n| n.contains("component view limit"))
        .unwrap_or_else(|| panic!("{:?}", report.truncated));
    assert!(
        note.ends_with("6 component views were not drawn: 6 cards with no crop kept are left out"),
        "{note}"
    );
    assert_eq!(report.shared_changes.len(), MAX_COMPONENT_VIEWS);
    assert!(report.shared_changes.iter().all(|s| s.component_view));
}

#[test]
fn component_views_stop_at_the_global_pixel_budget() {
    // Each 4000x1000 component costs 8 MP (both sides); the budget holds 8 of the 12.
    let (old, new) = pair(12, (4000.0, 1000.0));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let with_view = report
        .shared_changes
        .iter()
        .filter(|s| s.component_view)
        .count();
    assert_eq!(
        with_view,
        figstudio_diff::MAX_COMPONENT_VIEW_PIXELS / 8_000_000
    );
    assert!(
        report
            .truncated
            .iter()
            .any(|n| n.contains("4 component views were not drawn"))
    );
}

/// One component set of `variants` variants that change alike; variant `k` is used on
/// screens `k % 7` and `k % 7 + 7`, ten per screen (a screen shows at most 12 areas).
fn variant_set(variants: u32) -> (Document, Document) {
    let side = |new: bool| {
        let mut page = Vec::new();
        for s in 0..SCREENS {
            let mut frame = filled_rectangle_at(
                100 + s,
                1,
                (1.0, 1.0, 1.0),
                f64::from(s) * 1500.0,
                0.0,
                1400.0,
                200.0,
            );
            frame.data.set("type", Val::Str("FRAME".into()));
            page.push(Node::from_data(frame.data).unwrap());
        }
        let mut set = node(20, Some(11), "FRAME", "a");
        set.data.set("isStateGroup", Val::Bool(true));
        let mut second = vec![Node::from_data(set.data).unwrap()];
        for k in 0..variants {
            let symbol = 5000 + k * 2;
            let mut master = node(symbol, Some(20), "SYMBOL", "a");
            master.data.set(
                "size",
                object(&[("x", Val::Float(60.0)), ("y", Val::Float(40.0))]),
            );
            second.push(Node::from_data(master.data).unwrap());
            // Each variant looks different, so none is dropped as a duplicate picture.
            let tone = f64::from(k) / f64::from(variants);
            let color = if new {
                (0.0, 1.0, tone)
            } else {
                (1.0, 0.0, tone)
            };
            second.push(filled_rectangle_at(
                symbol + 1,
                symbol,
                color,
                0.0,
                0.0,
                60.0,
                40.0,
            ));
            for screen in [k % 7, k % 7 + 7] {
                page.push(instance(
                    1000 + k * 2 + screen / 7,
                    100 + screen,
                    20.0 + f64::from(k / 7) * 130.0,
                    symbol,
                    new,
                ));
            }
        }
        page.sort_by_key(|n| n.id);
        two_page_document(page, second)
    };
    (side(false), side(true))
}

#[test]
fn every_variant_counts_toward_the_view_limit() {
    let variants = MAX_COMPONENT_VIEWS as u32 + 5;
    let (old, new) = variant_set(variants);
    let mut report = diff_documents(&old, &new);
    let images = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let group = &report.shared_changes[0];
    let drawn = 1 + images[&figstudio_diff::SHARED_IMAGES_ID].variants.len();
    assert!(group.component_view);
    assert_eq!(drawn, MAX_COMPONENT_VIEWS, "{group:#?}");
    let note = report
        .truncated
        .iter()
        .find(|n| n.contains("component view limit"))
        .unwrap();
    assert!(
        note.ends_with("5 component views were not drawn: 5 other variant pictures are left out"),
        "{note}"
    );
    // The card keeps its component picture; the last five variants stay listed, unpictured.
    let listed = &group.component.as_ref().unwrap().variants;
    assert_eq!(listed.len(), variants as usize);
    assert!(
        listed[MAX_COMPONENT_VIEWS..]
            .iter()
            .all(|v| v.image_width.is_none())
    );
}

#[test]
fn crop_pixels_count_toward_the_view_pixel_budget() {
    // Each 4100x1000 component costs 8.2 MP of frames, so the frames alone fit 8 views;
    // the crops cut from them must be charged too and leave room for fewer.
    let (old, new) = pair(12, (4100.0, 1000.0));
    let mut report = diff_documents(&old, &new);
    let _ = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    let with_view = report
        .shared_changes
        .iter()
        .filter(|s| s.component_view)
        .count();
    assert!(
        with_view < figstudio_diff::MAX_COMPONENT_VIEW_PIXELS / 8_200_000,
        "{with_view}"
    );
}
