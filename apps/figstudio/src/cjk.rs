//! Browser CJK fallback selection; language mapping also tested on native builds.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Jp,
    Sc,
    Tc,
    Hk,
    Kr,
}

impl Variant {
    pub const ALL: [Self; 5] = [Self::Jp, Self::Sc, Self::Tc, Self::Hk, Self::Kr];

    pub fn code(self) -> &'static str {
        match self {
            Self::Jp => "jp",
            Self::Sc => "sc",
            Self::Tc => "tc",
            Self::Hk => "hk",
            Self::Kr => "kr",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Jp => "Japanese (JP)",
            Self::Sc => "Simplified Chinese (SC)",
            Self::Tc => "Traditional Chinese (TC)",
            Self::Hk => "Hong Kong (HK)",
            Self::Kr => "Korean (KR)",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.code() == code)
    }
}

/// What decides the session's CJK face. KR is the only face with Hangul, and it also has
/// kana and ideographs; any other reason means the saved choice.
#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the browser fetches a CJK font")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FaceFor {
    /// Nothing has asked for a CJK face.
    #[default]
    Unasked,
    /// An open file needs the saved choice.
    File,
    /// A View menu pick.
    Picked,
    /// A file whose only CJK is Hangul: KR until a menu pick or a reload.
    HangulFile,
    /// A Recents name has Hangul and nothing else asked: KR.
    HangulName,
}

#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the browser fetches a CJK font")
)]
impl FaceFor {
    pub fn korean(self) -> bool {
        matches!(self, Self::HangulFile | Self::HangulName)
    }

    /// A file asked for a CJK face. A file's need wins over the Recents names; KR for a
    /// Hangul-only file and a menu pick keep their place, as before.
    pub fn after_document(self, hangul_only: bool) -> Self {
        match (hangul_only, self) {
            (true, _) => Self::HangulFile,
            (false, Self::HangulFile | Self::Picked) => self,
            (false, _) => Self::File,
        }
    }

    /// A Recents name has Hangul. It decides only when nothing else has.
    pub fn after_names(self) -> Self {
        match self {
            Self::Unasked => Self::HangulName,
            other => other,
        }
    }
}

/// The session's CJK face state: why the face is what it is, and the most the Recents
/// names asked for so far. Kept apart from the browser calls so every path is tested.
#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the browser fetches a CJK font")
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub reason: FaceFor,
    names: crate::theme::CjkNeed,
    /// The files that asked for the saved face, by path.
    files: std::collections::HashSet<std::path::PathBuf>,
}

impl Default for Face {
    fn default() -> Self {
        Self {
            reason: FaceFor::Unasked,
            names: crate::theme::CjkNeed::None,
            files: std::collections::HashSet::new(),
        }
    }
}

#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the browser fetches a CJK font")
)]
impl Face {
    /// Whether KR is in use instead of the saved choice.
    pub fn korean(&self) -> bool {
        self.reason.korean()
    }

    /// The CJK need (never `None`) of the file opened at `path`.
    pub fn document(&mut self, hangul_only: bool, path: &std::path::Path) {
        self.reason = self.reason.after_document(hangul_only);
        if !hangul_only {
            self.files.insert(path.to_owned());
        }
    }

    /// A View menu pick.
    pub fn pick(&mut self) {
        self.reason = FaceFor::Picked;
    }

    /// One paint of the Recents page with these names' `need`, while the files at `open`
    /// are open; true when a CJK font should be fetched. Each level acts once, so a menu
    /// pick is not undone on the next frame. Once none of the files that asked for the saved
    /// face is open, that request ends and the names ask again. A Hangul-only file's KR and
    /// a menu pick stay after their file closes.
    pub fn recents<'a>(
        &mut self,
        need: crate::theme::CjkNeed,
        open: impl IntoIterator<Item = &'a std::path::Path>,
    ) -> bool {
        use crate::theme::{CjkNeed, names_rank};
        if self.reason == FaceFor::File && !open.into_iter().any(|p| self.files.contains(p)) {
            self.reason = FaceFor::Unasked;
            self.names = CjkNeed::None;
            self.files.clear();
        }
        if names_rank(need) <= names_rank(self.names) {
            return false;
        }
        self.names = need;
        if need == CjkNeed::HangulOnly {
            self.reason = self.reason.after_names();
        }
        true
    }
}

pub fn from_language(language: &str) -> Variant {
    let language = language.to_ascii_lowercase();
    let mut parts = language.split('-');
    match parts.next() {
        Some("ja") => Variant::Jp,
        Some("ko") => Variant::Kr,
        Some("zh") => {
            let parts: Vec<_> = parts.collect();
            if parts.contains(&"hk") || parts.contains(&"mo") {
                Variant::Hk
            } else if parts.contains(&"tw") || parts.contains(&"hant") {
                Variant::Tc
            } else {
                Variant::Sc
            }
        }
        _ => Variant::Jp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_language_selects_regional_glyphs() {
        for (languages, expected) in [
            (&["ja", "ja-JP"][..], Variant::Jp),
            (&["ko", "ko-KR"], Variant::Kr),
            (&["zh-HK", "zh-MO", "zh-Hant-HK", "ZH-hant-mo"], Variant::Hk),
            (&["zh-TW", "zh-Hant", "zh-Hant-TW"], Variant::Tc),
            (
                &["zh", "zh-CN", "zh-SG", "zh-Hans", "zh-Hans-CN"],
                Variant::Sc,
            ),
            (&["", "en-US", "fr", "zho", "kok"], Variant::Jp),
        ] {
            for language in languages {
                assert_eq!(from_language(language), expected, "{language}");
            }
        }
        for variant in Variant::ALL {
            assert_eq!(Variant::from_code(variant.code()), Some(variant));
            assert!(!variant.label().is_empty());
        }
        assert_eq!(Variant::from_code("invalid"), None);
    }

    #[test]
    fn files_decide_the_face_while_open_and_recents_names_after() {
        use crate::theme::CjkNeed::{Any, HangulOnly, None as NoNames};
        use std::path::Path;
        let (a, b) = (Path::new("a.fig"), Path::new("b.fig"));
        // Recents with a Hangul name, a Japanese file, then Recents after it closed.
        let mut face = Face::default();
        assert!(face.recents(HangulOnly, []) && face.korean());
        face.document(false, a);
        assert!(!face.korean(), "the open file takes the saved face back");
        assert!(
            !face.recents(HangulOnly, [a]),
            "Recents opened with + while it is open"
        );
        assert!(!face.korean());
        assert!(face.recents(HangulOnly, []), "closed: the names ask again");
        assert!(face.korean());
        assert!(!face.recents(HangulOnly, []), "and only once");

        // A Japanese file before Recents ever asked: Recents opened with + keeps its face,
        // and after it closes the names decide.
        let mut face = Face::default();
        face.document(false, a);
        assert!(face.recents(HangulOnly, [a]), "the names are new");
        assert!(!face.korean(), "but the open file's face stays");
        assert!(face.recents(HangulOnly, []) && face.korean());

        // The Japanese file closes while another file without CJK stays open.
        let mut face = Face::default();
        face.document(false, a);
        assert!(!face.recents(NoNames, [a, b]));
        assert!(face.recents(HangulOnly, [b]) && face.korean());

        // Kana first, Hangul later: KR, which also has kana.
        let mut face = Face::default();
        assert!(face.recents(Any, []) && !face.korean());
        assert!(face.recents(HangulOnly, []) && face.korean());

        // A menu pick is not undone by names, with or without files.
        let mut face = Face::default();
        face.pick();
        assert!(face.recents(HangulOnly, []) && !face.korean());
        assert!(!face.recents(HangulOnly, []) && !face.korean());

        // A Hangul-only file keeps KR, also after it closes, until a pick.
        let mut face = Face::default();
        face.document(true, a);
        face.document(false, a);
        assert!(face.korean(), "unchanged from before");
        face.recents(Any, []);
        assert!(face.korean());
        face.pick();
        assert!(!face.korean());
    }
}
