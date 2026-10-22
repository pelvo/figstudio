//! Browser font snapshots shared with render and thumbnail workers.
use std::sync::{Arc, Mutex};

type Font = Arc<Vec<u8>>;

#[derive(Default)]
struct Fonts {
    generation: u64,
    faces: Vec<(Arc<egui::FontData>, Font)>,
}

static FONTS: Mutex<Fonts> = Mutex::new(Fonts {
    generation: 0,
    faces: Vec::new(),
});

/// Called after egui applies pending font definitions. Never block the browser UI.
pub fn sync(ctx: &egui::Context) -> bool {
    let definitions = ctx.fonts(|fonts| fonts.definitions().clone());
    let Ok(mut fonts) = FONTS.try_lock() else {
        ctx.request_repaint();
        return false;
    };
    let faces: Vec<_> = definitions.font_data.values().cloned().collect();
    if faces.len() == fonts.faces.len()
        && faces
            .iter()
            .zip(&fonts.faces)
            .all(|(a, (b, _))| Arc::ptr_eq(a, b))
    {
        return false;
    }
    fonts.faces = faces
        .into_iter()
        .map(|face| {
            let bytes = fonts
                .faces
                .iter()
                .find(|(old, _)| Arc::ptr_eq(old, &face))
                .map_or_else(|| Arc::new(face.font.to_vec()), |(_, bytes)| bytes.clone());
            (face, bytes)
        })
        .collect();
    fonts.generation += 1;
    true
}

/// Only render workers take the blocking lock; parsing happens after it is released.
pub fn snapshot() -> (u64, Vec<Font>) {
    let fonts = FONTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    (
        fonts.generation,
        fonts.faces.iter().map(|(_, bytes)| bytes.clone()).collect(),
    )
}

pub fn refresh(generation: &mut u64, renderer: &mut figstudio_render::Renderer) -> bool {
    let (current, faces) = snapshot();
    if *generation == current {
        return false;
    }
    let images = renderer.images();
    *renderer = configured(images, faces);
    *generation = current;
    true
}

pub fn renderer(images: figstudio_render::images::Images) -> (u64, figstudio_render::Renderer) {
    let (generation, fonts) = snapshot();
    (generation, configured(images, fonts))
}

fn configured(
    images: figstudio_render::images::Images,
    fonts: Vec<Font>,
) -> figstudio_render::Renderer {
    let mut renderer = figstudio_render::Renderer::with_images(images);
    renderer.set_emoji_source(Some(Arc::new(crate::emoji_canvas::BrowserEmoji)));
    for bytes in fonts {
        renderer.add_font_data(bytes);
    }
    renderer
}
