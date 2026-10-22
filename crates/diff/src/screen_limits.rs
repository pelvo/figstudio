//! Work caps for standalone screen rendering: a DoS guard that real design files stay under.
use crate::ScreenImages;

/// Caps on the standalone screens drawn per page and per document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenLimits {
    /// Screens drawn per page; later ones keep their model-only status.
    pub page_screens: usize,
    /// Pixels (both sides) drawn for standalone screens across the whole document.
    pub document_pixels: u64,
    /// Encoded picture bytes one page may hold in memory before the sink takes them.
    pub page_bytes: usize,
}

pub const MAX_PAGE_SCREENS: usize = 2_048;
pub const MAX_DOCUMENT_SCREEN_PIXELS: u64 = 4_294_967_296;
pub const MAX_PAGE_PICTURE_BYTES: usize = 512 * 1024 * 1024;

impl Default for ScreenLimits {
    fn default() -> Self {
        Self {
            page_screens: MAX_PAGE_SCREENS,
            document_pixels: MAX_DOCUMENT_SCREEN_PIXELS,
            page_bytes: MAX_PAGE_PICTURE_BYTES,
        }
    }
}

/// Running totals for one page; `doc_pixels` is shared by all pages of the comparison.
pub(crate) struct PageTally<'a> {
    limits: ScreenLimits,
    doc_pixels: &'a mut u64,
    drawn: usize,
    bytes: usize,
    pub(crate) skipped: usize,
}

impl<'a> PageTally<'a> {
    pub(crate) fn new(limits: ScreenLimits, doc_pixels: &'a mut u64) -> Self {
        Self {
            limits,
            doc_pixels,
            drawn: 0,
            bytes: 0,
            skipped: 0,
        }
    }

    /// Reserves one screen of `cost` pixels; `false` (counted as skipped) when over a cap.
    pub(crate) fn admit(&mut self, cost: u64) -> bool {
        let over = self.drawn >= self.limits.page_screens
            || self.bytes >= self.limits.page_bytes
            || self.doc_pixels.saturating_add(cost) > self.limits.document_pixels;
        if over {
            self.skipped += 1;
        } else {
            self.drawn += 1;
            *self.doc_pixels += cost;
        }
        !over
    }

    /// Whether the pictures this page holds have reached the byte cap.
    pub(crate) fn bytes_full(&self) -> bool {
        self.bytes >= self.limits.page_bytes
    }

    /// Counts the whole-page pictures already held for this page.
    pub(crate) fn keep_page(&mut self, images: &crate::RenderImages) {
        let len = |v: &Option<Vec<u8>>| v.as_ref().map_or(0, Vec::len);
        self.bytes += len(&images.before) + len(&images.after) + len(&images.diff);
    }

    /// Counts the encoded bytes a drawn screen keeps in memory.
    pub(crate) fn keep(&mut self, pictures: &ScreenImages) {
        let len = |v: &Option<Vec<u8>>| v.as_ref().map_or(0, Vec::len);
        self.bytes += len(&pictures.before)
            + len(&pictures.after)
            + pictures
                .regions
                .iter()
                .map(|r| r.before.len() + r.after.len())
                .sum::<usize>();
    }

    pub(crate) fn note(&self, page: impl std::fmt::Display) -> Option<String> {
        (self.skipped > 0).then(|| {
            let ScreenLimits {
                page_screens,
                document_pixels,
                page_bytes,
            } = self.limits;
            format!(
                "page {page} screen limit reached ({page_screens} screens per page, {document_pixels} pixels per document or {page_bytes} picture bytes per page); {} screens were not rendered",
                self.skipped
            )
        })
    }
}
