//! Lazy images are read through the supplied readers; page rendering streams and is capped.
mod support;

use std::cell::Cell;
use std::io::Cursor;

use figstudio_diff::{
    RenderOpts, attach_render_diffs, attach_render_diffs_with_sink, diff_documents,
    render_page_pair, render_page_pair_with_readers,
};
use figstudio_fig::archive::Entry;
use figstudio_fig::{Document, Guid};
use support::{document, image_rectangle, many_page_document};

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(8, 8, |x, y| {
        image::Rgba([(x * 30) as u8, 90, (y * 30) as u8, 255])
    });
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn image_document(lazy: bool) -> Document {
    let mut doc = document(vec![image_rectangle(2, 1, HASH)]);
    if lazy {
        let entry = Entry {
            name: format!("images/{HASH}"),
            method: 0,
            compressed_size: 0,
            size: 0,
            header_offset: 0,
        };
        doc.lazy_images.insert(HASH.into(), entry);
    } else {
        doc.images.insert(HASH.into(), png());
    }
    doc
}

#[test]
fn lazy_images_render_through_the_reader() {
    let opts = RenderOpts::default();
    let page = Some(Guid(0, 1));
    let (eager, eager_png) = render_page_pair(
        &image_document(false),
        &image_document(false),
        page,
        page,
        &opts,
    )
    .unwrap();
    let (lazy, lazy_png) = (image_document(true), image_document(true));
    let (blind, blind_png) = render_page_pair(&lazy, &lazy_png, page, page, &opts).unwrap();
    let blind_stats = blind.old_stats.as_ref().unwrap();
    assert_eq!(
        blind_stats.missing_images, 1,
        "no reader: counted, not hidden"
    );

    let reads = Cell::new(0);
    let read = |_: &_| {
        reads.set(reads.get() + 1);
        Some(png())
    };
    let (fed, fed_png) =
        render_page_pair_with_readers(&lazy, &lazy_png, page, page, &opts, &read, &read).unwrap();
    assert!(reads.get() > 0);
    assert_eq!(fed.old_stats.as_ref().unwrap().images_loading, 0);
    assert_eq!(fed.old_stats.as_ref().unwrap().missing_images, 0);
    assert_eq!(
        fed_png.before, eager_png.before,
        "lazy + reader draws what the eager file draws"
    );
    assert_ne!(fed_png.before, blind_png.before);
    assert_eq!(eager.pixels_changed, Some(0));
    assert_eq!(fed.pixels_changed, Some(0));
}

#[test]
fn page_limit_streams_and_notes_the_rest() {
    let red = (1.0, 0.0, 0.0);
    let green = (0.0, 1.0, 0.0);
    let old = many_page_document(&[red, red, red]);
    let new = many_page_document(&[green, green, green]);
    let none = |_: &_| None;

    let mut report = diff_documents(&old, &new);
    assert_eq!(report.pages.len(), 3);
    let opts = RenderOpts {
        max_pages: Some(1),
        ..RenderOpts::default()
    };
    let mut seen = Vec::new();
    attach_render_diffs_with_sink(
        &old,
        &new,
        &mut report,
        &opts,
        &none,
        &none,
        &mut |id, images| {
            seen.push((id, images.diff.is_some()));
        },
    );
    assert_eq!(
        seen,
        vec![(report.pages[0].id, true)],
        "only the first page rendered"
    );
    assert!(report.pages[0].render.is_some());
    assert!(report.pages[1..].iter().all(|p| p.render.is_none()));
    let notes: Vec<_> = report
        .truncated
        .iter()
        .filter(|n| n.contains("not rendered"))
        .collect();
    assert_eq!(notes.len(), 1, "{:?}", report.truncated);
    assert!(notes[0].contains('2'), "{notes:?}");

    // The map wrapper still returns every page when under the cap.
    let mut report = diff_documents(&old, &new);
    let all = attach_render_diffs(&old, &new, &mut report, &RenderOpts::default());
    assert_eq!(all.len(), 3);
    assert!(report.truncated.iter().all(|n| !n.contains("not rendered")));
}
