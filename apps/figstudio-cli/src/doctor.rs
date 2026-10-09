//! `doctor`: a structure report that is safe to share.
//!
//! Prints counts, Figma type and field names (which come from Figma's schema, not from the
//! document), timings and memory. It never prints layer names, text, font names, image data
//! or hashes, colours, the file name or the path. Error messages are scrubbed the same way.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use figstudio_fig::{Document, Guid, Val};
use figstudio_render::kurbo::{Affine, Point};
use figstudio_render::{Renderer, Viewport, content_bounds, hit_test, page_background};

/// Counts keyed by schema names, printed largest first.
#[derive(Default)]
struct Tally(BTreeMap<String, usize>);

impl Tally {
    fn add(&mut self, key: &str) {
        *self.0.entry(key.to_string()).or_default() += 1;
    }

    fn line(&self, limit: usize) -> String {
        if self.0.is_empty() {
            return "none".into();
        }
        let mut items: Vec<(&String, &usize)> = self.0.iter().collect();
        items.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut out: Vec<String> = items
            .iter()
            .take(limit)
            .map(|(k, n)| format!("{k} {}", num(**n)))
            .collect();
        if items.len() > limit {
            out.push(format!("… {} more", items.len() - limit));
        }
        out.join(" · ")
    }
}

fn num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn secs(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 0.1 {
        format!("{:.1} ms", s * 1000.0)
    } else {
        format!("{s:.2} s")
    }
}

/// Peak resident memory of this process, if the platform reports it.
/// The only unsafe code in the workspace: a plain libc call with no safe std equivalent.
#[allow(unsafe_code)]
fn peak_memory_mb() -> Option<f64> {
    #[cfg(unix)]
    {
        // SAFETY: getrusage only writes into the struct we pass.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
            return None;
        }
        let raw = usage.ru_maxrss as f64;
        // macOS reports bytes, Linux and the BSDs kilobytes.
        let bytes = if cfg!(target_vendor = "apple") {
            raw
        } else {
            raw * 1024.0
        };
        Some(bytes / 1_048_576.0)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Replaces anything that looks like a SHA-1 (image names inside the archive) with `<hash>`.
fn scrub(msg: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    for c in msg.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_hexdigit() {
            run.push(c);
            continue;
        }
        if run.len() >= 16 {
            out.push_str("<hash>");
        } else {
            out.push_str(&run);
        }
        run.clear();
        out.push(c);
    }
    out.pop();
    out
}

fn image_kind(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, ..] => "jpeg",
        [b'G', b'I', b'F', ..] => "gif",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => "webp",
        _ if bytes.len() < 16 => "placeholder",
        _ => "other",
    }
}

pub fn run(file: &Path) -> Result<(), String> {
    // Panic messages could carry document data; report the location only.
    std::panic::set_hook(Box::new(|info| {
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!(
            "figstudio doctor: internal error at {at} (details hidden to keep the report private)"
        );
    }));
    let mut r = String::new();
    let w = &mut r;
    let _ = writeln!(
        w,
        "figstudio doctor {} ({} {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(
        w,
        "No layer names, text, fonts, images, colours or paths below.\n"
    );

    let t = Instant::now();
    let mut bytes = Vec::new();
    figstudio_fig::read_limited_file(file, &mut bytes)
        .map_err(|_| "cannot read the file (unavailable or too large)".to_string())?;
    let read_time = t.elapsed();
    let t = Instant::now();
    let doc = Document::from_bytes(&bytes)
        .map_err(|_| "cannot open the file (invalid or unsupported .fig data)".to_string())?;
    let decode_time = t.elapsed();
    let opened_memory = peak_memory_mb();
    let container = if bytes.starts_with(b"PK") {
        "zip"
    } else {
        "raw canvas"
    };
    let file_len = bytes.len();
    drop(bytes);

    let _ = writeln!(w, "file");
    let _ = writeln!(w, "  size          {} bytes ({container})", num(file_len));
    let _ = writeln!(w, "  format        fig-kiwi v{}", doc.format_version);
    let _ = writeln!(
        w,
        "  thumbnail     {}",
        if doc.thumbnail.is_some() { "yes" } else { "no" }
    );
    let mut kinds = Tally::default();
    let image_bytes: usize = doc.images.values().map(Vec::len).sum();
    for data in doc.images.values() {
        kinds.add(image_kind(data));
    }
    let _ = writeln!(
        w,
        "  images        {} files, {} bytes ({})",
        num(doc.images.len()),
        num(image_bytes),
        kinds.line(8)
    );
    let blob_bytes: usize = doc.blobs.iter().map(Vec::len).sum();
    let _ = writeln!(
        w,
        "  geometry      {} blobs, {} bytes",
        num(doc.blobs.len()),
        num(blob_bytes)
    );

    // Structure.
    let mut types = Tally::default();
    let mut fields = Tally::default();
    let mut paints = Tally::default();
    let mut effects = Tally::default();
    let mut blends = Tally::default();
    let mut text_layers = 0;
    let mut text_outlines = 0;
    let mut families: HashSet<String> = HashSet::new();
    let mut hidden = 0;
    let mut masks = 0;
    let mut auto_layout = 0;
    let mut overrides = 0;
    let mut image_fills = 0;
    let mut missing_image_fills = 0;
    for n in &doc.nodes {
        types.add(&n.kind);
        if let Val::Obj(o) = &n.data {
            for k in o.keys() {
                fields.add(k);
            }
        }
        let d = &n.data;
        if d.flag("visible") == Some(false) {
            hidden += 1;
        }
        if d.flag("mask") == Some(true) {
            masks += 1;
        }
        if d.text("stackMode").is_some_and(|m| m != "NONE") {
            auto_layout += 1;
        }
        if let Some(mode) = d
            .text("blendMode")
            .filter(|m| *m != "PASS_THROUGH" && *m != "NORMAL")
        {
            blends.add(mode);
        }
        for key in ["fillPaints", "strokePaints"] {
            for p in d.get(key).map(Val::items).unwrap_or_default() {
                let kind = p.text("type").unwrap_or("?");
                paints.add(kind);
                if kind == "IMAGE" {
                    image_fills += 1;
                    let hash = p
                        .path(&["image", "hash"])
                        .and_then(Val::as_bytes)
                        .map(figstudio_fig::hex);
                    if hash.is_none_or(|h| !doc.images.contains_key(&h)) {
                        missing_image_fills += 1;
                    }
                }
            }
        }
        for e in d.get("effects").map(Val::items).unwrap_or_default() {
            let kind = e.text("type").unwrap_or("?");
            let op = e.text("blurOpType").filter(|o| *o != "NORMAL");
            match op {
                Some(op) => effects.add(&format!("{kind}/{op}")),
                None => effects.add(kind),
            }
        }
        if n.kind == "TEXT" {
            text_layers += 1;
            if d.path(&["derivedTextData", "glyphs"])
                .is_some_and(|g| !g.items().is_empty())
            {
                text_outlines += 1;
            }
            if let Some(f) = d.path(&["fontName", "family"]).and_then(Val::as_str) {
                families.insert(f.to_string());
            }
        }
        if n.kind == "INSTANCE" {
            overrides += d
                .path(&["symbolData", "symbolOverrides"])
                .map_or(0, |o| o.items().len());
        }
    }

    // Tree shape.
    let mut max_depth = 0;
    let mut max_children = 0;
    let mut stack: Vec<(Guid, usize)> = doc.root.map(|r| (r, 0)).into_iter().collect();
    let mut reached = 0;
    while let Some((id, depth)) = stack.pop() {
        reached += 1;
        max_depth = max_depth.max(depth);
        let kids = doc.children(id);
        max_children = max_children.max(kids.len());
        stack.extend(kids.iter().map(|c| (*c, depth + 1)));
    }
    let orphans = doc.nodes.len().saturating_sub(reached);

    let _ = writeln!(w, "\nnodes");
    let _ = writeln!(
        w,
        "  total         {} ({} hidden, {} not under the document root)",
        num(doc.nodes.len()),
        num(hidden),
        num(orphans)
    );
    let _ = writeln!(
        w,
        "  pages         {} visible, {} internal",
        doc.pages.len(),
        doc.internal_pages.len()
    );
    for (i, page) in doc.pages.iter().enumerate().take(50) {
        let _ = writeln!(
            w,
            "    page {:<3}    {} layers",
            i + 1,
            num(doc.descendant_count(*page))
        );
    }
    if doc.pages.len() > 50 {
        let _ = writeln!(w, "    … {} more pages", doc.pages.len() - 50);
    }
    let _ = writeln!(
        w,
        "  depth         {max_depth} levels, at most {} children under one layer",
        num(max_children)
    );
    let _ = writeln!(w, "  types         {}", types.line(40));

    let _ = writeln!(w, "\nfeatures");
    let symbols = doc.nodes.iter().filter(|n| n.kind == "SYMBOL").count();
    let instances = doc.nodes.iter().filter(|n| n.kind == "INSTANCE").count();
    let _ = writeln!(
        w,
        "  components    {} main, {} instances, {} overrides",
        num(symbols),
        num(instances),
        num(overrides)
    );
    let _ = writeln!(w, "  auto layout   {} frames", num(auto_layout));
    let _ = writeln!(w, "  masks         {}", num(masks));
    let _ = writeln!(w, "  paints        {}", paints.line(20));
    let _ = writeln!(
        w,
        "  image fills   {} ({} point at images missing from the file)",
        num(image_fills),
        num(missing_image_fills)
    );
    let _ = writeln!(w, "  effects       {}", effects.line(20));
    let _ = writeln!(w, "  blend modes   {}", blends.line(20));
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let installed: HashSet<String> = db
        .faces()
        .flat_map(|f| f.families.iter().map(|(n, _)| n.to_lowercase()))
        .collect();
    let missing_fonts = families
        .iter()
        .filter(|f| !installed.contains(&f.to_lowercase()))
        .count();
    let _ = writeln!(
        w,
        "  text          {} layers, {} with stored outlines; {} font families, {} not installed here",
        num(text_layers),
        num(text_outlines),
        families.len(),
        missing_fonts
    );

    // Rendering: every page fitted to at most 2048 px, then one 1400x880 window at 100%.
    let _ = writeln!(w, "\nrender");
    let mut renderer = Renderer::new();
    let mut slowest: Option<(usize, Duration)> = None;
    for (i, page) in doc.pages.iter().enumerate().take(20) {
        let Some(b) = content_bounds(&doc, *page) else {
            let _ = writeln!(w, "  page {:<3}      empty", i + 1);
            continue;
        };
        let scale = (2048.0 / b.width().max(b.height()).max(1.0)).min(1.0);
        let t = Instant::now();
        let frame =
            renderer.render_fit(&doc, *page, scale, 0.0, Some(page_background(&doc, *page)));
        let took = t.elapsed();
        if slowest.is_none_or(|(_, d)| took > d) {
            slowest = Some((i, took));
        }
        match frame {
            Some(f) => {
                let _ = writeln!(
                    w,
                    "  page {:<3} fit  {} at {:.3}x: {}",
                    i + 1,
                    secs(took),
                    scale,
                    f.stats
                );
            }
            None => {
                let _ = writeln!(w, "  page {:<3} fit  nothing visible", i + 1);
            }
        }
    }
    if doc.pages.len() > 20 {
        let _ = writeln!(w, "  … {} more pages not rendered", doc.pages.len() - 20);
    }
    if let Some((i, _)) = slowest {
        let page = doc.pages[i];
        if let Some(b) = content_bounds(&doc, page) {
            let (vw, vh) = (1400u16, 880u16);
            let c = b.center();
            let transform =
                Affine::translate((f64::from(vw) / 2.0 - c.x, f64::from(vh) / 2.0 - c.y));
            let t = Instant::now();
            let f = renderer.render(
                &doc,
                page,
                &Viewport {
                    width: vw,
                    height: vh,
                    transform,
                    background: None,
                },
            );
            let _ = writeln!(
                w,
                "  page {:<3} 100% {} for a {vw}x{vh} window at the centre: {}",
                i + 1,
                secs(t.elapsed()),
                f.stats
            );
            let t = Instant::now();
            let hits = 200;
            for k in 0..hits {
                let p = Point::new(b.x0 + b.width() * f64::from(k) / f64::from(hits), c.y);
                std::hint::black_box(hit_test(&doc, page, p));
            }
            let _ = writeln!(w, "  hit test      {} each", secs(t.elapsed() / hits));
        }
    }

    // Writing back (in memory only).
    let _ = writeln!(w, "\nwrite back (in memory, nothing is saved)");
    let t = Instant::now();
    match doc.to_bytes() {
        Ok(out) => {
            let took = t.elapsed();
            let same = Document::from_bytes(&out).is_ok_and(|again| {
                again.nodes.len() == doc.nodes.len()
                    && again
                        .nodes
                        .iter()
                        .zip(&doc.nodes)
                        .all(|(a, b)| a.id == b.id && a.data == b.data)
            });
            let _ = writeln!(
                w,
                "  encode        {}, {} bytes, reads back identical: {}",
                secs(took),
                num(out.len()),
                if same { "yes" } else { "NO" }
            );
        }
        Err(e) => {
            let _ = writeln!(w, "  encode        FAILED: {}", scrub(&e.to_string()));
        }
    }

    let _ = writeln!(w, "\ntimings");
    let _ = writeln!(w, "  read file     {}", secs(read_time));
    let _ = writeln!(w, "  decode        {}", secs(decode_time));
    let _ = writeln!(w, "\nmemory");
    match (opened_memory, peak_memory_mb()) {
        (Some(open), Some(peak)) => {
            let _ = writeln!(
                w,
                "  after opening {open:.0} MB (what the app needs to show the file)"
            );
            let _ = writeln!(
                w,
                "  peak          {peak:.0} MB (includes rendering and the write-back check)"
            );
        }
        _ => {
            let _ = writeln!(w, "  not measured on this platform");
        }
    }
    let _ = writeln!(w, "\nnode fields used (Figma schema names)");
    let _ = writeln!(w, "  {}", fields.line(400));
    print!("{r}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_hides_hashes_only() {
        assert_eq!(
            scrub("cannot read images/93e8eeb27e934c4b9ae9e7929c7df9e96a6ec90c: bad"),
            "cannot read images/<hash>: bad"
        );
        assert_eq!(scrub("node 12:34 bad"), "node 12:34 bad");
    }

    #[test]
    fn numbers_get_separators() {
        assert_eq!(num(0), "0");
        assert_eq!(num(1234), "1,234");
        assert_eq!(num(1_234_567), "1,234,567");
    }
}
