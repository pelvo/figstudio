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

    /// The last file closed: what it asked for no longer holds.
    pub fn after_files_closed(self) -> Self {
        match self {
            Self::File => Self::Unasked,
            other => other,
        }
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
        use FaceFor::*;
        assert_eq!(
            Unasked.after_names(),
            HangulName,
            "Recents with a Hangul name first"
        );
        assert_eq!(
            HangulName.after_document(false),
            File,
            "a Japanese file takes it back"
        );
        assert_eq!(File.after_names(), File, "Recents while that file is open");
        assert_eq!(File.after_files_closed(), Unasked, "the file closed");
        assert_eq!(
            Unasked.after_names(),
            HangulName,
            "so Recents asks for KR again"
        );
        assert_eq!(HangulName.after_document(true), HangulFile);
        assert_eq!(HangulFile.after_names(), HangulFile);
        assert_eq!(
            HangulFile.after_document(false),
            HangulFile,
            "unchanged from before"
        );
        assert_eq!(
            HangulFile.after_files_closed(),
            HangulFile,
            "until a pick or a reload"
        );
        assert_eq!(
            Picked.after_names(),
            Picked,
            "a pick is not undone by names"
        );
        assert_eq!(Picked.after_document(false), Picked);
        assert_eq!(Picked.after_files_closed(), Picked);
        assert!(HangulName.korean() && HangulFile.korean());
        assert!(!Unasked.korean() && !File.korean() && !Picked.korean());
    }
}
