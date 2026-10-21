//! The browser's Recents pictures in memory (`recents_web.rs`): which are being read from
//! IndexedDB, drawn on the pool, ready to show or shown. Apart from the IndexedDB calls, so
//! its rules are tested natively.

use std::collections::HashMap;

/// What the page should do for a picture this frame.
pub(crate) enum Look {
    /// Show this (`None`: the file has no picture).
    Texture(Option<egui::TextureHandle>),
    /// Start reading the saved picture; give the answer to `Pictures::read`.
    Read,
    /// Nothing yet.
    Wait,
}

/// The pictures by key (`recents_web::key`).
#[derive(Default)]
pub(crate) struct Pictures {
    /// `None` for none.
    textures: HashMap<String, Option<egui::TextureHandle>>,
    /// Read or drawn, waiting to become textures.
    ready: HashMap<String, Vec<u8>>,
    /// Being read, with the job of the picture placed when the read began: an answer read
    /// before a newer picture was placed is old and dropped.
    reading: HashMap<String, u64>,
    /// Each key's newest job: an older job, or one for a file removed since, keeps nothing
    /// (as on the desktop).
    wanted: HashMap<String, u64>,
    /// The job of the picture last placed for each key.
    placed: HashMap<String, u64>,
    next_job: u64,
}

impl Pictures {
    /// A new job drawing `key`'s picture; its number goes to `drawn`.
    pub fn want(&mut self, key: &str) -> u64 {
        self.next_job += 1;
        self.wanted.insert(key.to_owned(), self.next_job);
        self.next_job
    }

    /// Job `number` drew `png`. Placed (and returned, to be saved) only when it is still the
    /// newest job wanted for `key`.
    pub fn drawn(&mut self, key: &str, number: u64, png: Vec<u8>) -> Option<&[u8]> {
        if self.wanted.get(key) != Some(&number) {
            return None;
        }
        self.wanted.remove(key);
        self.textures.remove(key);
        self.placed.insert(key.to_owned(), number);
        self.ready.insert(key.to_owned(), png);
        self.ready.get(key).map(Vec::as_slice)
    }

    /// A saved picture read for `key` (`None`: there is none).
    pub fn read(&mut self, key: &str, png: Option<Vec<u8>>) {
        let Some(began) = self.reading.remove(key) else {
            return;
        };
        // A picture placed since the read began is newer than what it read.
        if self.placed.get(key).copied().unwrap_or(0) != began {
            return;
        }
        match png {
            Some(png) => {
                self.ready.insert(key.to_owned(), png);
            }
            None => {
                self.textures.insert(key.to_owned(), None);
            }
        }
    }

    /// The picture of `key`, decoding at most `budget` more this frame; `can_read`: a saved
    /// one may be read.
    pub fn look(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        budget: &mut usize,
        can_read: bool,
    ) -> Look {
        if let Some(t) = self.textures.get(key) {
            return Look::Texture(t.clone());
        }
        if self.ready.contains_key(key) {
            if *budget == 0 {
                ctx.request_repaint();
                return Look::Wait;
            }
            *budget -= 1;
            let Some(png) = self.ready.remove(key) else {
                return Look::Wait;
            };
            let texture = crate::recents::png_texture(ctx, &format!("recent-{key}"), &png);
            self.textures.insert(key.to_owned(), texture.clone());
            return Look::Texture(texture);
        }
        if can_read && !self.reading.contains_key(key) {
            let placed = self.placed.get(key).copied().unwrap_or(0);
            self.reading.insert(key.to_owned(), placed);
            return Look::Read;
        }
        Look::Wait
    }

    /// `key`'s file left the list: its picture, a job and a read for it are dropped.
    pub fn forget(&mut self, key: &str) {
        self.textures.remove(key);
        self.ready.remove(key);
        self.reading.remove(key);
        self.wanted.remove(key);
        self.placed.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(shade: u8) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([shade, shade, shade, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("a 2x2 PNG encodes");
        out.into_inner()
    }

    fn shows(pictures: &mut Pictures, ctx: &egui::Context, key: &str) -> bool {
        matches!(
            pictures.look(ctx, key, &mut 3, true),
            Look::Texture(Some(_))
        )
    }

    #[test]
    fn a_read_that_misses_after_the_picture_was_drawn_does_not_hide_it() {
        let ctx = egui::Context::default();
        let mut p = Pictures::default();
        let job = p.want("k");
        // The page asks before the picture is drawn: nothing saved yet.
        assert!(matches!(p.look(&ctx, "k", &mut 3, true), Look::Read));
        assert!(p.drawn("k", job, png(10)).is_some());
        // IndexedDB answers late, from before the picture was saved.
        p.read("k", None);
        assert!(shows(&mut p, &ctx, "k"));
    }

    #[test]
    fn an_old_saved_picture_read_late_does_not_replace_the_new_one() {
        let ctx = egui::Context::default();
        let mut p = Pictures::default();
        assert!(matches!(p.look(&ctx, "k", &mut 3, true), Look::Read));
        let job = p.want("k");
        p.drawn("k", job, png(200));
        p.read("k", Some(png(10)));
        assert_eq!(p.ready.get("k"), Some(&png(200)));
    }

    #[test]
    fn a_file_with_no_saved_picture_is_read_once() {
        let ctx = egui::Context::default();
        let mut p = Pictures::default();
        assert!(matches!(p.look(&ctx, "k", &mut 3, true), Look::Read));
        assert!(matches!(p.look(&ctx, "k", &mut 3, true), Look::Wait));
        p.read("k", None);
        assert!(matches!(
            p.look(&ctx, "k", &mut 3, true),
            Look::Texture(None)
        ));
    }

    #[test]
    fn a_saved_picture_read_is_shown() {
        let ctx = egui::Context::default();
        let mut p = Pictures::default();
        assert!(matches!(p.look(&ctx, "k", &mut 3, true), Look::Read));
        p.read("k", Some(png(10)));
        assert!(shows(&mut p, &ctx, "k"));
    }

    #[test]
    fn of_two_jobs_only_the_newest_is_placed_and_a_forgotten_one_never() {
        let mut p = Pictures::default();
        let old = p.want("k");
        let new = p.want("k");
        assert!(p.drawn("k", old, png(1)).is_none());
        assert!(p.drawn("k", new, png(2)).is_some());
        let job = p.want("gone");
        p.forget("gone");
        assert!(p.drawn("gone", job, png(3)).is_none());
    }
}
