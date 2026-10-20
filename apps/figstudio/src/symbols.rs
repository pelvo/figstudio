//! Lazy browser fallback for symbols and outline emoji in UI names.
use egui::{FontFamily, FontId};

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
    let names = std::iter::once(file_name)
        .chain(doc.file_name.as_deref())
        .chain(doc.nodes.iter().map(|n| n.name.as_str()));
    ctx.fonts_mut(|fonts| {
        names
            .filter(|s| needs(s))
            .flat_map(str::chars)
            .filter(|&c| is_symbol(c))
            .any(|c| !fonts.has_glyph(&FontId::proportional(13.0), c))
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

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub struct Loader {
    started: bool,
    tx: std::sync::mpsc::Sender<anyhow::Result<Faces>>,
    rx: std::sync::mpsc::Receiver<anyhow::Result<Faces>>,
}

#[cfg(target_arch = "wasm32")]
impl Default for Loader {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            started: false,
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
        if self.started || !missing(ctx, doc, file_name) {
            return;
        }
        self.started = true;
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
        self.rx
            .try_recv()
            .ok()?
            .and_then(|faces| install(ctx, faces))
            .err()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn covered_symbols_do_not_trigger_a_download() {
        let ctx = egui::Context::default();
        tick(&ctx);
        let doc = figstudio_fig::Document::from_bytes(include_bytes!(
            "../../../tests/fixtures/corners.fig"
        ))
        .unwrap();
        assert!(!missing(&ctx, &doc, "Latin page"));
        assert!(missing(&ctx, &doc, "📄 page └"));
        assert!(!missing(&ctx, &doc, "★"));
    }
}
