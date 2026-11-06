//! `prototype`: the flows of a file, the screens they reach and every link on them.
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use figstudio_fig::{Document, Guid};
use figstudio_render::Renderer;
use figstudio_render::encoding::{ImageFormat, encode_pixmap};
use figstudio_render::prototype::{self, Flow, Link};

/// Most screens listed or rendered.
const MAX_SCREENS: usize = 500;

pub struct Options<'a> {
    pub flow: Option<&'a str>,
    pub page: Guid,
    pub out: Option<&'a Path>,
    pub scale: f64,
    pub format: ImageFormat,
    pub system_fonts: bool,
}

pub fn run(doc: &Document, options: &Options<'_>) -> Result<()> {
    let flows = prototype::flows(doc);
    let starts = match options.flow {
        Some(wanted) => vec![pick_flow(&flows, wanted)?.start],
        None if flows.is_empty() => {
            let start = prototype::start(doc, options.page, None)
                .context("this page has no flows and no frames")?;
            println!("no flows; starting at the page's first frame");
            vec![start]
        }
        None => flows.iter().map(|f| f.start).collect(),
    };
    if !flows.is_empty() {
        println!("flows:");
        for (i, flow) in flows.iter().enumerate() {
            println!("  {}. {} → {}", i + 1, flow.name, named(doc, flow.start));
        }
    }
    let mut renderer = Renderer::new();
    renderer.system_fonts = options.system_fonts;
    let (screens, truncated) = prototype::reachable(&mut renderer, doc, &starts, MAX_SCREENS);
    println!(
        "screens ({}{}):",
        screens.len(),
        if truncated { ", more not listed" } else { "" }
    );
    let mut skipped = 0;
    for screen in &screens {
        println!("  {}", named(doc, screen.frame));
        for link in &screen.links {
            println!(
                "    {} {:?}  {}",
                link.layer,
                link.name,
                describe(doc, link)
            );
        }
        if screen.truncated {
            println!("    … more links not listed");
        }
        skipped += screen.skipped();
    }
    println!(
        "{skipped} links not played (Preview plays navigate, back and change-to on click, hover, press, mouse enter/leave/down/up, after delay and drag)"
    );
    if let Some(dir) = options.out {
        std::fs::create_dir_all(dir).map_err(|e| anyhow!("{}: {e}", dir.display()))?;
        for (i, screen) in screens.iter().enumerate() {
            let page = doc.page_of(screen.frame).unwrap_or(options.page);
            let Some(frame) = renderer.render_fit(
                doc,
                screen.frame,
                options.scale,
                0.0,
                Some(prototype::background(doc, page)),
            ) else {
                println!("{} has nothing visible; not rendered", screen.frame);
                continue;
            };
            let id = screen.frame.to_string().replace(':', "-");
            let out = dir.join(format!("{:02}-{id}.{}", i + 1, options.format.extension()));
            std::fs::write(&out, encode_pixmap(frame.pixmap, options.format)?)
                .map_err(|e| anyhow!("{}: {e}", out.display()))?;
            println!("{}", out.display());
        }
    }
    Ok(())
}

/// A flow by number (from 1) or by name.
fn pick_flow<'f>(flows: &'f [Flow], wanted: &str) -> Result<&'f Flow> {
    if let Ok(n) = wanted.parse::<usize>() {
        return flows
            .get(n.wrapping_sub(1))
            .with_context(|| format!("no flow {n}; the file has {}", flows.len()));
    }
    match flows.iter().find(|f| f.name == wanted) {
        Some(flow) => Ok(flow),
        None => bail!("no flow named {wanted:?}"),
    }
}

fn named(doc: &Document, id: Guid) -> String {
    match doc.node(id) {
        Some(node) => format!("{id} {:?}", node.name),
        None => format!("{id} (missing)"),
    }
}

/// `on click → navigate to 1:4 "Name"`, with `(not played)` when Preview does not play it.
fn describe(doc: &Document, link: &Link) -> String {
    let trigger = link.trigger.to_lowercase().replace('_', " ");
    let action = link.action.to_lowercase().replace('_', " ");
    let target = match (&link.url, link.destination) {
        (Some(url), _) => format!(" {url}"),
        (None, Some(to)) => format!(" {}", named(doc, to)),
        (None, None) => String::new(),
    };
    let played = if link.step.is_none() {
        " (not played)"
    } else {
        ""
    };
    format!("{trigger} → {action}{target}{played}")
}
