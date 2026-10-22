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

/// Why the session draws CJK with KR instead of the saved choice. KR is the only face with
/// Hangul, and it also has kana and ideographs.
#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the browser fetches a CJK font")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KoreanFor {
    /// The saved choice is in use.
    #[default]
    Nothing,
    /// An open file's only CJK is Hangul.
    Document,
    /// A Recents name has Hangul and no file asked for a face.
    Names,
}

#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the browser fetches a CJK font")
)]
impl KoreanFor {
    pub fn on(self) -> bool {
        self != Self::Nothing
    }

    /// A file asked for a CJK face: KR for Hangul only, else the saved choice wins over KR
    /// that only the Recents names asked for.
    pub fn after_document(self, hangul_only: bool) -> Self {
        match (hangul_only, self) {
            (true, _) => Self::Document,
            (false, Self::Names) => Self::Nothing,
            (false, other) => other,
        }
    }

    /// A Recents name has Hangul. It never replaces a file's reason.
    pub fn after_names(self) -> Self {
        match self {
            Self::Nothing => Self::Names,
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
    fn a_file_takes_back_the_face_that_recents_names_switched_to_kr() {
        use KoreanFor::*;
        assert_eq!(Nothing.after_names(), Names, "a Hangul name on Recents");
        assert_eq!(
            Names.after_document(false),
            Nothing,
            "a Japanese file opens next"
        );
        assert_eq!(
            Names.after_document(true),
            Document,
            "a Hangul-only file opens next"
        );
        assert_eq!(
            Document.after_names(),
            Document,
            "names never replace a file's reason"
        );
        assert_eq!(
            Document.after_document(false),
            Document,
            "unchanged from before"
        );
        assert_eq!(Nothing.after_document(false), Nothing);
        assert!(Names.on() && Document.on() && !Nothing.on());
    }
}
