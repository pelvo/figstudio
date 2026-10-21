//! Lazy browser fallback for symbols and outline emoji in UI names.
use egui::FontFamily;

const FILES: [&str; 4] = ["symbols.ttf", "symbols2.ttf", "math.ttf", "emoji.ttf"];
type Faces = [Vec<u8>; 4];

pub fn needs(text: &str) -> bool {
    text.chars().any(is_symbol)
}

fn is_symbol(c: char) -> bool {
    matches!(c as u32, 0x20A0..=0x20CF | 0x2100..=0x214F | 0x2190..=0x2BFF
        | 0x1F000..=0x1FAFF | 0x1FB00..=0x1FBFF)
}

fn missing(ctx: &egui::Context, doc: &figstudio_fig::Document, file_name: &str) -> bool {
    let text = std::iter::once(file_name)
        .chain(doc.file_name.as_deref())
        .chain(doc.nodes.iter().flat_map(|n| {
            std::iter::once(n.name.as_str()).chain(
                n.data
                    .path(&["textData", "characters"])
                    .and_then(figstudio_fig::Val::as_str),
            )
        }));
    ctx.fonts(|fonts| {
        let definitions = fonts.definitions();
        // has_glyph misses egui's bundled emoji; query the actual proportional chain's cmap.
        let faces: Vec<_> = definitions
            .families
            .get(&FontFamily::Proportional)
            .into_iter()
            .flatten()
            .filter_map(|name| definitions.font_data.get(name))
            .filter_map(|data| ttf_parser::Face::parse(&data.font, data.index).ok())
            .collect();
        text.filter(|s| needs(s))
            .flat_map(str::chars)
            .filter(|&c| is_symbol(c))
            .any(|c| !faces.iter().any(|face| face.glyph_index(c).is_some()))
    })
}

fn install(ctx: &egui::Context, faces: Faces) -> anyhow::Result<()> {
    for bytes in &faces {
        anyhow::ensure!(bytes.len() <= 2 * 1024 * 1024, "symbol font is too large");
        let face = ttf_parser::Face::parse(bytes, 0)
            .map_err(|_| anyhow::anyhow!("invalid symbol font"))?;
        anyhow::ensure!(
            face.tables().glyf.is_some() || face.tables().cff.is_some(),
            "symbol font has no outlines"
        );
    }
    let mut fonts = ctx.fonts(|fonts| fonts.definitions().clone());
    let names: Vec<_> = FILES
        .iter()
        .map(|file| format!("web-symbols-{file}"))
        .collect();
    for (name, bytes) in names.iter().zip(faces) {
        fonts
            .font_data
            .insert(name.clone(), egui::FontData::from_owned(bytes).into());
    }
    for family in [
        FontFamily::Proportional,
        FontFamily::Monospace,
        FontFamily::Name(crate::theme::SEMIBOLD.into()),
    ] {
        let entries = fonts.families.entry(family).or_default();
        entries.retain(|name| !name.starts_with("web-symbols-"));
        entries.extend(names.iter().cloned());
    }
    ctx.set_fonts(fonts);
    Ok(())
}

#[derive(Debug, Default)]
struct LoadState {
    document: Option<u64>,
    pending: bool,
    installed: bool,
    warned: bool,
    retry_after_pending: bool,
}

impl LoadState {
    fn begin(&mut self, document: u64) -> bool {
        if self.installed || self.document == Some(document) {
            return false;
        }
        self.document = Some(document);
        if self.pending {
            self.retry_after_pending = true;
            return false;
        }
        self.pending = true;
        true
    }

    /// Complete an attempt and return whether its failure needs the session's one toast.
    fn finish(&mut self, success: bool) -> bool {
        self.pending = !success && std::mem::take(&mut self.retry_after_pending);
        self.retry_after_pending = false;
        self.installed = success;
        if success || self.warned {
            return false;
        }
        self.warned = true;
        true
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub struct Loader {
    state: LoadState,
    tx: std::sync::mpsc::Sender<anyhow::Result<Faces>>,
    rx: std::sync::mpsc::Receiver<anyhow::Result<Faces>>,
}

#[cfg(target_arch = "wasm32")]
impl Default for Loader {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            state: LoadState::default(),
            tx,
            rx,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Loader {
    pub fn load_if_needed(
        &mut self,
        ctx: &egui::Context,
        doc: &figstudio_fig::Document,
        file_name: &str,
    ) {
        if self.state.installed || !missing(ctx, doc, file_name) || !self.state.begin(doc.instance)
        {
            return;
        }
        self.fetch(ctx);
    }

    fn fetch(&self, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let mut faces: Faces = Default::default();
                for (bytes, file) in faces.iter_mut().zip(FILES) {
                    *bytes = crate::web::fetch_bytes(&format!("fonts/{file}")).await?;
                }
                Ok(faces)
            }
            .await;
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Option<anyhow::Error> {
        let result = self
            .rx
            .try_recv()
            .ok()?
            .and_then(|faces| install(ctx, faces));
        let warn = self.state.finish(result.is_ok());
        if self.state.pending {
            self.fetch(ctx);
        }
        if warn { result.err() } else { None }
    }
}

#[cfg(test)]
#[path = "symbols_tests.rs"]
mod tests;
