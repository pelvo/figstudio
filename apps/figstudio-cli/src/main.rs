//! Headless figstudio: inspect `.fig` files and render pages or nodes to PNG.

mod doctor;
mod synth;

use std::path::PathBuf;
use std::process::ExitCode;

use figstudio_fig::{Document, Guid};
use figstudio_render::color::{AlphaColor, Srgb};
use figstudio_render::{Renderer, page_background};

const USAGE: &str = "figstudio-cli — headless tools for Figma .fig files

USAGE:
  figstudio-cli info <file.fig>
  figstudio-cli tree <file.fig> [--page <n|name>] [--depth <n>]
  figstudio-cli render <file.fig> [-o <out.png>] [--page <n|name>] [--node <id>]
                     [--scale <x>] [--background page|transparent|#rrggbb] [--no-system-fonts]
  figstudio-cli thumbnail <file.fig> -o <out.png>
  figstudio-cli doctor <file.fig>
      Structure report safe to share: counts, features, timings and memory only.
      No layer names, text, images or file paths are printed.
  figstudio-cli resave <file.fig> -o <out.fig>
      Read and write the file back unchanged (tests the .fig writer).
  figstudio-cli synth <file.fig> -o <out.fig> [--copies <n>] [--pages <n>]
      Stress-test file: the first page's layers copied n times on each of n pages.

Pages are numbered from 1. Node ids look like 12:34 (see `tree`).";

struct Args {
    command: String,
    file: PathBuf,
    out: Option<PathBuf>,
    page: Option<String>,
    node: Option<Guid>,
    scale: f64,
    depth: usize,
    background: String,
    system_fonts: bool,
    copies: usize,
    pages: usize,
}

fn parse(mut raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let command = raw.next().ok_or("missing command")?;
    if command == "-h" || command == "--help" || command == "help" {
        return Err(String::new());
    }
    let mut args = Args {
        command,
        file: PathBuf::new(),
        out: None,
        page: None,
        node: None,
        scale: 1.0,
        depth: 3,
        background: "page".into(),
        system_fonts: true,
        copies: 100,
        pages: 1,
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
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            path if file.is_none() => file = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    args.file = file.ok_or("missing .fig file")?;
    Ok(args)
}

fn pick_page(doc: &Document, wanted: Option<&str>) -> Result<Guid, String> {
    let Some(wanted) = wanted else {
        return doc
            .pages
            .first()
            .copied()
            .ok_or_else(|| "document has no pages".into());
    };
    if let Ok(n) = wanted.parse::<usize>() {
        return doc.pages.get(n.wrapping_sub(1)).copied().ok_or(format!(
            "page {n} does not exist ({} pages)",
            doc.pages.len()
        ));
    }
    doc.pages
        .iter()
        .copied()
        .find(|id| doc.node(*id).is_some_and(|n| n.name == wanted))
        .ok_or(format!("no page named {wanted}"))
}

fn parse_background(
    spec: &str,
    doc: &Document,
    page: Guid,
) -> Result<Option<AlphaColor<Srgb>>, String> {
    match spec {
        "page" => Ok(Some(page_background(doc, page))),
        "transparent" | "none" => Ok(None),
        hex if hex.starts_with('#') && hex.len() == 7 => {
            let v = u32::from_str_radix(&hex[1..], 16).map_err(|_| format!("bad colour {hex}"))?;
            let c = |shift: u32| ((v >> shift) & 0xff) as f32 / 255.0;
            Ok(Some(AlphaColor::new([c(16), c(8), c(0), 1.0])))
        }
        other => Err(format!(
            "--background must be page, transparent or #rrggbb, got {other}"
        )),
    }
}

fn print_tree(doc: &Document, id: Guid, indent: usize, depth: usize) {
    let Some(node) = doc.node(id) else { return };
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
        return;
    }
    for child in doc.children(id) {
        print_tree(doc, *child, indent + 1, depth);
    }
}

fn run(args: Args) -> Result<(), String> {
    if args.command == "doctor" {
        // Opens the file itself, to time it and to keep the path out of any error.
        return doctor::run(&args.file);
    }
    let doc = Document::open(&args.file).map_err(|e| e.to_string())?;
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
        }
        "tree" => {
            let page = pick_page(&doc, args.page.as_deref())?;
            print_tree(&doc, page, 0, args.depth);
        }
        "render" => {
            let (root, page) = match args.node {
                Some(id) => {
                    let node = doc.node(id).ok_or(format!("no node {id}"))?;
                    if node.kind == "DOCUMENT" {
                        return Err("pick a page or a layer, not the document".into());
                    }
                    (id, doc.page_of(id).ok_or(format!("{id} is not on a page"))?)
                }
                None => {
                    let page = pick_page(&doc, args.page.as_deref())?;
                    (page, page)
                }
            };
            let background = parse_background(&args.background, &doc, page)?;
            let mut renderer = Renderer::new();
            renderer.system_fonts = args.system_fonts;
            let frame = renderer
                .render_fit(
                    &doc,
                    root,
                    args.scale,
                    16.0 * args.scale.min(2.0),
                    background,
                )
                .ok_or("nothing visible to render")?;
            let out = args.out.clone().unwrap_or_else(|| {
                let stem = args
                    .file
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or("out".into());
                PathBuf::from(format!("{stem}.png"))
            });
            let (w, h) = (frame.pixmap.width(), frame.pixmap.height());
            let png = frame
                .pixmap
                .into_png()
                .map_err(|e| format!("PNG encode failed: {e}"))?;
            std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("{} {w}x{h}", out.display());
            println!("{}", frame.stats);
        }
        "thumbnail" => {
            let out = args.out.ok_or("thumbnail needs -o <out.png>")?;
            let thumb = doc
                .thumbnail
                .as_ref()
                .ok_or("this file has no stored thumbnail")?;
            std::fs::write(&out, thumb).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("{}", out.display());
        }
        "resave" => {
            let out = args.out.ok_or("resave needs -o <out.fig>")?;
            doc.save(&out).map_err(|e| e.to_string())?;
            let again = Document::open(&out)
                .map_err(|e| format!("written file does not read back: {e}"))?;
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
                return Err("round trip changed the document".into());
            }
        }
        "synth" => {
            let out = args.out.ok_or("synth needs -o <out.fig>")?;
            let start = std::time::Instant::now();
            let big = synth::synth(doc, args.copies, args.pages)?;
            big.save(&out).map_err(|e| e.to_string())?;
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
        other => return Err(format!("unknown command {other}")),
    }
    Ok(())
}

fn main() -> ExitCode {
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
        Err(msg) if msg.starts_with("unknown command") => {
            eprintln!("figstudio-cli: {msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(msg) => {
            eprintln!("figstudio-cli: {msg}");
            ExitCode::FAILURE
        }
    }
}
