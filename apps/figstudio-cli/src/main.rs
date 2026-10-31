//! Headless figstudio: inspect `.fig` files and render lossless PNG/WebP images.

mod compare;
mod doctor;
mod image_output;
mod synth;

use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use figstudio_fig::{Document, Guid};
use figstudio_render::color::{AlphaColor, Srgb};
use figstudio_render::encoding::{ImageFormat, encode_pixmap};
use figstudio_render::{Renderer, page_background};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

/// Largest SVG `render` writes; a bigger one is an error, not a cut file.
const MAX_SVG_BYTES: usize = 256 * 1024 * 1024;

const USAGE: &str = "figstudio-cli — headless tools for Figma .fig files

USAGE:
  figstudio-cli info <file.fig>
  figstudio-cli tree <file.fig> [--page <n|name>] [--depth <n>]
  figstudio-cli render <file.fig> [-o <out.png|out.webp|out.svg>] [--format png|webp|svg] [--page <n|name>] [--node <id>]
                     [--scale <x>] [--background page|transparent|#rrggbb] [--no-system-fonts]
  figstudio-cli thumbnail <file.fig> -o <out.png|out.webp> [--format png|webp]
  figstudio-cli doctor <file.fig>
      Structure report safe to share: counts, features, timings and memory only.
      No layer names, text, images or file paths are printed.
  figstudio-cli resave <file.fig> -o <out.fig>
      Read and write the file back unchanged (tests the .fig writer).
  figstudio-cli synth <file.fig> -o <out.fig> [--copies <n>] [--pages <n>]
      Stress-test file: the first page's layers copied n times on each of n pages.
  figstudio-cli compare <old.fig> <new.fig> -o <dir> [--scale <x>] [--no-render]
                        [--json-only] [--no-system-fonts] [--format png|webp]
      Version report: report.json, report.html and lossless WebP images under <dir>/render.
      Compare defaults to WebP; render/thumbnail infer .webp (render also .svg) from -o, otherwise PNG.

Pages are numbered from 1. Node ids look like 12:34 (see `tree`).";

/// Marks an error as a usage problem: `main` prints the help text and exits with 2.
#[derive(Debug)]
struct UsageError(String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

struct Args {
    command: String,
    file: PathBuf,
    file2: Option<PathBuf>,
    out: Option<PathBuf>,
    page: Option<String>,
    node: Option<Guid>,
    scale: f64,
    depth: usize,
    background: String,
    system_fonts: bool,
    copies: usize,
    pages: usize,
    no_render: bool,
    json_only: bool,
    format: Option<ImageFormat>,
    /// `--format svg`: `render` writes SVG (also inferred from an `.svg` output).
    svg: bool,
}

fn command_error(command: &str, has_output: bool) -> Option<String> {
    if ![
        "info",
        "tree",
        "render",
        "thumbnail",
        "doctor",
        "resave",
        "synth",
        "compare",
    ]
    .contains(&command)
    {
        return Some(format!("unknown command {command}"));
    }
    match (command, has_output) {
        ("thumbnail", false) => Some("thumbnail needs -o <out.png>".into()),
        ("resave", false) => Some("resave needs -o <out.fig>".into()),
        ("synth", false) => Some("synth needs -o <out.fig>".into()),
        ("compare", false) => Some("compare needs -o <dir>".into()),
        _ => None,
    }
}

/// `compare` runs on private files: its argument errors are fixed text and echo no user value.
fn parse(raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let raw: Vec<String> = raw.collect();
    let compare = raw.first().is_some_and(|c| c == "compare");
    parse_args(raw.into_iter()).map_err(|e| {
        if compare {
            "invalid compare arguments".into()
        } else {
            e
        }
    })
}

fn parse_args(mut raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let command = raw.next().ok_or("missing command")?;
    if command == "-h" || command == "--help" || command == "help" {
        return Err(String::new());
    }
    let mut args = Args {
        command,
        file: PathBuf::new(),
        file2: None,
        out: None,
        page: None,
        node: None,
        scale: 1.0,
        depth: 3,
        background: "page".into(),
        system_fonts: true,
        copies: 100,
        pages: 1,
        no_render: false,
        json_only: false,
        format: None,
        svg: false,
    };
    let mut file = None;
    while let Some(a) = raw.next() {
        let mut value = |name: &str| raw.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "-o" | "--out" => args.out = Some(value("-o")?.into()),
            "--page" => args.page = Some(value("--page")?),
            "--node" => args.node = Some(value("--node")?.parse()?),
            "--scale" => {
                args.scale = value("--scale")?
                    .parse()
                    .map_err(|_| "--scale must be a number")?;
                if !(0.01..=32.0).contains(&args.scale) {
                    return Err("--scale must be between 0.01 and 32".into());
                }
            }
            "--depth" => {
                args.depth = value("--depth")?
                    .parse()
                    .map_err(|_| "--depth must be a number")?
            }
            "--background" => args.background = value("--background")?,
            "--copies" | "--pages" => {
                let name = a.clone();
                let n: usize = value(&name)?
                    .parse()
                    .map_err(|_| format!("{name} must be a whole number"))?;
                if !(1..=1_000_000).contains(&n) {
                    return Err(format!("{name} must be between 1 and 1000000"));
                }
                if name == "--copies" {
                    args.copies = n
                } else {
                    args.pages = n
                }
            }
            "--no-system-fonts" => args.system_fonts = false,
            "--no-render" => args.no_render = true,
            "--json-only" => args.json_only = true,
            "--format" => match value("--format")?.as_str() {
                "svg" => args.svg = true,
                other => {
                    args.format = Some(
                        other
                            .parse()
                            .map_err(|e: std::io::Error| format!("{e} (or svg for render)"))?,
                    )
                }
            },
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            path if file.is_none() => file = Some(PathBuf::from(path)),
            path if args.command == "compare" && args.file2.is_none() => {
                args.file2 = Some(PathBuf::from(path))
            }
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    command_error(&args.command, args.out.is_some()).map_or(Ok(()), Err)?;
    args.file = file.ok_or("missing .fig file")?;
    if args.command == "compare" && args.file2.is_none() {
        return Err("compare needs two files: <old.fig> <new.fig>".into());
    }
    Ok(args)
}

fn pick_page(doc: &Document, wanted: Option<&str>) -> Result<Guid> {
    let Some(wanted) = wanted else {
        return doc.pages.first().copied().context("document has no pages");
    };
    if let Ok(n) = wanted.parse::<usize>() {
        return doc
            .pages
            .get(n.wrapping_sub(1))
            .copied()
            .ok_or_else(|| anyhow!("page {n} does not exist ({} pages)", doc.pages.len()));
    }
    doc.pages
        .iter()
        .copied()
        .find(|id| doc.node(*id).is_some_and(|n| n.name == wanted))
        .ok_or_else(|| anyhow!("no page named {wanted}"))
}

fn parse_background(spec: &str, doc: &Document, page: Guid) -> Result<Option<AlphaColor<Srgb>>> {
    match spec {
        "page" => Ok(Some(page_background(doc, page))),
        "transparent" | "none" => Ok(None),
        hex if hex.starts_with('#') && hex.len() == 7 => {
            let v = u32::from_str_radix(&hex[1..], 16).map_err(|_| anyhow!("bad colour {hex}"))?;
            let c = |shift: u32| ((v >> shift) & 0xff) as f32 / 255.0;
            Ok(Some(AlphaColor::new([c(16), c(8), c(0), 1.0])))
        }
        other => bail!("--background must be page, transparent or #rrggbb, got {other}"),
    }
}

fn print_tree(doc: &Document, id: Guid, indent: usize, depth: usize) {
    let mut stack = vec![(id, indent)];
    while let Some((id, indent)) = stack.pop() {
        let Some(node) = doc.node(id) else { continue };
        let hidden = if node.data.flag("visible") == Some(false) {
            " (hidden)"
        } else {
            ""
        };
        println!(
            "{}{} {} {:?}{hidden}",
            "  ".repeat(indent),
            node.id,
            node.kind,
            node.name
        );
        if indent >= depth {
            let n = doc.descendant_count(id);
            if n > 0 {
                println!("{}… {n} more", "  ".repeat(indent + 1));
            }
            continue;
        }
        stack.extend(
            doc.children(id)
                .iter()
                .rev()
                .map(|child| (*child, indent + 1)),
        );
    }
}

mod emoji_report;

fn emoji_counts(doc: &Document, system_fonts: bool) -> figstudio_render::Stats {
    let mut renderer = Renderer::new();
    renderer.system_fonts = system_fonts;
    renderer.emoji_counts(doc)
}

fn run(args: Args) -> Result<()> {
    if args.command == "doctor" {
        // Opens the file itself, to time it and to keep the path out of any error.
        return doctor::run(&args.file);
    }
    if args.command == "compare" {
        // -o is checked by the parser; this keeps every compare path out of the shared open.
        let out = args.out.clone().context("compare needs -o <dir>")?;
        let new_file = args.file2.as_deref().unwrap_or(&args.file);
        return compare::run(
            &args.file,
            new_file,
            &out,
            args.scale,
            args.system_fonts,
            !args.no_render,
            (!args.json_only, args.format.unwrap_or(ImageFormat::Webp)),
        );
    }
    let doc = Document::open(&args.file)?;
    debug!(
        command = args.command.as_str(),
        nodes = doc.nodes.len(),
        pages = doc.pages.len(),
        "document opened"
    );
    match args.command.as_str() {
        "info" => {
            println!("file:     {}", args.file.display());
            if let Some(name) = &doc.file_name {
                println!("name:     {name}");
            }
            println!("format:   {} v{}", doc.prelude, doc.format_version);
            println!("nodes:    {}", doc.nodes.len());
            println!("images:   {}", doc.images.len());
            println!("pages:");
            for (i, id) in doc.pages.iter().enumerate() {
                let node = doc.node(*id).unwrap();
                println!(
                    "  {}. {} ({}, {} layers)",
                    i + 1,
                    node.name,
                    id,
                    doc.descendant_count(*id)
                );
            }
            let components = doc.nodes.iter().filter(|n| n.kind == "SYMBOL").count();
            println!("components: {components}");
            let counts = emoji_counts(&doc, args.system_fonts);
            println!("emoji:    {}", emoji_report::summary(&counts));
        }
        "tree" => {
            let page = pick_page(&doc, args.page.as_deref())?;
            print_tree(&doc, page, 0, args.depth);
        }
        "render" => {
            let (root, page) = match args.node {
                Some(id) => {
                    let node = doc.node(id).ok_or_else(|| anyhow!("no node {id}"))?;
                    if node.kind == "DOCUMENT" {
                        bail!("pick a page or a layer, not the document");
                    }
                    (
                        id,
                        doc.page_of(id)
                            .ok_or_else(|| anyhow!("{id} is not on a page"))?,
                    )
                }
                None => {
                    let page = pick_page(&doc, args.page.as_deref())?;
                    (page, page)
                }
            };
            let background = parse_background(&args.background, &doc, page)?;
            let mut renderer = Renderer::new();
            renderer.system_fonts = args.system_fonts;
            renderer.shaders = figstudio_shader::Fills::for_file(&args.file)
                .map(|f| f as std::sync::Arc<dyn figstudio_render::shader::ShaderFills>);
            let svg = args.svg
                || args
                    .out
                    .as_ref()
                    .and_then(|o| o.extension())
                    .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
            let out = args.out.clone().unwrap_or_else(|| {
                let stem = args
                    .file
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or("out".into());
                let ext = if svg {
                    "svg"
                } else {
                    args.format.unwrap_or_default().extension()
                };
                PathBuf::from(format!("{stem}.{ext}"))
            });
            // A node is cropped like Figma's export (its drawn pixels, at least a frame's box);
            // a page keeps its padding.
            let node = args.node.is_some();
            let padding = 16.0 * args.scale.min(2.0);
            if svg {
                let drawn = if node {
                    renderer.render_export_svg_complete(
                        &doc,
                        root,
                        args.scale,
                        background,
                        MAX_SVG_BYTES,
                        |_| None,
                    )
                } else {
                    renderer.render_fit_svg_complete(
                        &doc,
                        root,
                        args.scale,
                        padding,
                        background,
                        MAX_SVG_BYTES,
                        |_| None,
                    )
                }
                .context("nothing visible to render")?;
                if drawn.over_limit {
                    bail!("the SVG is over 256 MiB; render a smaller node");
                }
                std::fs::write(&out, &drawn.markup)
                    .map_err(|e| anyhow!("{}: {e}", out.display()))?;
                println!("{} {} bytes of SVG", out.display(), drawn.markup.len());
                println!("{}", drawn.stats);
                if drawn.approximated > 0 {
                    println!("{} drawing steps SVG cannot express", drawn.approximated);
                }
                return Ok(());
            }
            let frame = if node {
                renderer.render_export_complete(&doc, root, args.scale, background, |_| None)
            } else {
                renderer.render_fit(&doc, root, args.scale, padding, background)
            }
            .context("nothing visible to render")?;
            let (w, h) = (frame.pixmap.width(), frame.pixmap.height());
            let bytes = encode_pixmap(frame.pixmap, image_output::format(args.format, &out))?;
            std::fs::write(&out, bytes).map_err(|e| anyhow!("{}: {e}", out.display()))?;
            info!(width = w, height = h, "page rendered");
            println!("{} {w}x{h}", out.display());
            println!("{}", frame.stats);
        }
        "thumbnail" => {
            let out = args.out.context("thumbnail needs -o <out.png>")?;
            let thumb = doc
                .thumbnail
                .as_ref()
                .context("this file has no stored thumbnail")?;
            let format = image_output::format(args.format, &out);
            let bytes = if format == ImageFormat::Png {
                thumb.to_vec()
            } else {
                let rgba = image::load_from_memory(thumb)?.to_rgba8();
                figstudio_render::encoding::encode(&rgba, rgba.width(), rgba.height(), format)?
            };
            std::fs::write(&out, bytes).map_err(|e| anyhow!("{}: {e}", out.display()))?;
            println!("{}", out.display());
        }
        "resave" => {
            let out = args.out.context("resave needs -o <out.fig>")?;
            doc.save(&out)?;
            let again = Document::open(&out)
                .map_err(|e| anyhow!("written file does not read back: {e}"))?;
            let same = again.nodes.len() == doc.nodes.len()
                && again
                    .nodes
                    .iter()
                    .zip(&doc.nodes)
                    .all(|(a, b)| a.id == b.id && a.data == b.data);
            println!(
                "{} ({} nodes, {})",
                out.display(),
                again.nodes.len(),
                if same {
                    "reads back identical"
                } else {
                    "DIFFERS after reading back"
                }
            );
            if !same {
                bail!("round trip changed the document");
            }
        }
        "synth" => {
            let out = args.out.context("synth needs -o <out.fig>")?;
            let start = std::time::Instant::now();
            let big =
                synth::synth(doc, args.copies, args.pages).context("build stress document")?;
            big.save(&out)?;
            let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!(
                "{} {} nodes, {} pages, {:.1} MB, {:.1} s",
                out.display(),
                big.nodes.len(),
                big.pages.len(),
                size as f64 / 1e6,
                start.elapsed().as_secs_f64()
            );
        }
        other => return Err(UsageError(format!("unknown command {other}")).into()),
    }
    Ok(())
}

/// Logs go to stderr only (stdout is data). Default level is `warn`; `RUST_LOG` overrides.
fn init_tracing() {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .from_env_lossy();
    // Ignore the error: a subscriber may already be installed, which is fine.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .try_init();
}

fn main() -> ExitCode {
    // Started to run shader code for a figstudio process (`figstudio_shader::child`).
    if figstudio_shader::serve_if_worker() {
        return ExitCode::SUCCESS;
    }
    init_tracing();
    // Usage problems (exit 2) print the help text; failures on a valid command (exit 1) do not.
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) if msg.is_empty() => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("figstudio-cli: {msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.downcast_ref::<UsageError>().is_some() => {
            eprintln!(
                "figstudio-cli: {}\n\n{USAGE}",
                figstudio_fig::one_line(e.as_ref())
            );
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("figstudio-cli: {}", figstudio_fig::one_line(e.as_ref()));
            ExitCode::FAILURE
        }
    }
}
