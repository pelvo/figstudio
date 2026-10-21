//! How the tools render: fonts, and where the images of a lazily opened document come from.
use crate::screenshot::Root;
use figstudio_fig::{Document, archive::Entry};
use figstudio_render::{Frame, Renderer, Viewport, images::Images};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

/// Reads decompressed archive entries, for example from a browser's byte-backed source.
pub type EntryReader = Arc<dyn Fn(&Entry) -> Option<Vec<u8>> + Send + Sync>;

/// Render settings for one registered file. Clones share the image cache.
#[derive(Clone, Default)]
pub struct Rendering {
    /// Lay out text that has no stored glyph outlines with installed fonts.
    pub system_fonts: bool,
    /// Browser-supplied faces, shared with the canvas font snapshot.
    pub fonts: Vec<(Arc<Vec<u8>>, u32)>,
    /// Lossless encoding for images returned by this call; defaults to PNG.
    pub format: figstudio_render::encoding::ImageFormat,
    /// The registered file, read for the images a lazily opened document did not load.
    /// `None`: those images count as missing.
    pub path: Option<PathBuf>,
    /// Entry source; takes precedence over the path, including failed reads.
    pub reader: Option<EntryReader>,
    /// Decoded images, shared by every render of the file.
    pub images: Images,
    /// Held while a lazy document renders: the cache's in-flight requests are shared, so two
    /// concurrent renders would each wait on images the other one reads.
    lazy_lock: Arc<Mutex<()>>,
}

impl std::fmt::Debug for Rendering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rendering")
            .field("system_fonts", &self.system_fonts)
            .field("path", &self.path)
            .field("reader", &self.reader.as_ref().map(|_| "EntryReader"))
            .field("images", &self.images)
            .finish_non_exhaustive()
    }
}

impl Rendering {
    /// Selects output encoding without changing the shared document image cache.
    pub fn with_format(mut self, format: figstudio_render::encoding::ImageFormat) -> Self {
        self.format = format;
        self
    }

    /// Settings that read lazy images from `path` and decode them into `images`.
    pub fn new(path: PathBuf, images: Images) -> Self {
        Self {
            path: Some(path),
            images,
            ..Self::default()
        }
    }

    /// Settings that read lazy images from a byte-backed source.
    pub fn with_reader(reader: EntryReader, images: Images) -> Self {
        Self {
            reader: Some(reader),
            images,
            ..Self::default()
        }
    }

    /// The decompressed bytes of archive `entry`; `None` when its source cannot read it.
    pub fn read(&self, entry: &Entry) -> Option<Vec<u8>> {
        if let Some(reader) = &self.reader {
            return reader(entry);
        }
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
        renderer.system_fonts = self.system_fonts || !self.fonts.is_empty();
        for (font, index) in &self.fonts {
            renderer.add_font_data(font.clone(), *index);
        }
        let read = |want: &figstudio_render::images::ImageRequest| self.read(&want.entry);
        match root {
            Root::Node(id) => renderer.render_complete(doc, id, viewport, read),
            Root::Path(path) => renderer.render_path_complete(doc, path, viewport, read),
        }
    }
}
