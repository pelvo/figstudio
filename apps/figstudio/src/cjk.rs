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
}
