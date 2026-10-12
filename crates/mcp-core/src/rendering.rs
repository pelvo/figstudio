//! How the tools render: fonts, and where the images of a lazily opened document come from.
use crate::screenshot::Root;
use figstudio_fig::{Document, archive::Entry};
use figstudio_render::{Frame, Renderer, Viewport, images::Images};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

/// Render settings for one registered file. Clones share the image cache.
#[derive(Clone, Debug, Default)]
pub struct Rendering {
    /// Lay out text that has no stored glyph outlines with installed fonts.
    pub system_fonts: bool,
    /// The registered file, read for the images a lazily opened document did not load.
    /// `None`: those images count as missing.
    pub path: Option<PathBuf>,
    /// Decoded images, shared by every render of the file.
    pub images: Images,
    /// Held while a lazy document renders: the cache's in-flight requests are shared, so two
    /// concurrent renders would each wait on images the other one reads.
    lazy_lock: Arc<Mutex<()>>,
}

impl Rendering {
    /// Settings that read lazy images from `path` and decode them into `images`.
    pub fn new(path: PathBuf, images: Images) -> Self {
        Self {
            path: Some(path),
            images,
            ..Self::default()
        }
    }

    /// The decompressed bytes of archive `entry` of the registered file; `None` when there
    /// is no file or it cannot be read.
    pub fn read(&self, entry: &Entry) -> Option<Vec<u8>> {
        let mut file = std::fs::File::open(self.path.as_ref()?).ok()?;
        figstudio_fig::archive::read_entry(&mut file, entry).ok()
    }

    /// Draws `root` with every image at the resolution `viewport` needs, reading the ones a
    /// lazy document is missing. An eager document renders once, as before.
    pub fn draw(&self, doc: &Document, root: Root<'_>, viewport: &Viewport) -> Frame {
        let _guard = (!doc.lazy_images.is_empty()).then(|| {
            self.lazy_lock
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        });
        let mut renderer = Renderer::with_images(self.images.clone());
        renderer.system_fonts = self.system_fonts;
        let read = |want: &figstudio_render::images::ImageRequest| self.read(&want.entry);
        match root {
            Root::Node(id) => renderer.render_complete(doc, id, viewport, read),
            Root::Path(path) => renderer.render_path_complete(doc, path, viewport, read),
        }
    }
}
