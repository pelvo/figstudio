//! The order in which the canvas falls back through egui's fonts.
use std::sync::Arc;

/// egui's faces in fallback priority: each family's list in turn (proportional first, the
/// UI's main text), then any face no family names. `font_data` is a map sorted by name,
/// whose order would put a monospace or icon face ahead of the proportional ones.
pub fn by_priority(definitions: &egui::FontDefinitions) -> Vec<Arc<egui::FontData>> {
    let mut families: Vec<_> = definitions.families.iter().collect();
    families.sort_by_key(|(family, _)| *family != &egui::FontFamily::Proportional);
    let named = families.into_iter().flat_map(|(_, names)| names.iter());
    let mut seen = std::collections::HashSet::new();
    named
        .chain(definitions.font_data.keys())
        .filter(|name| seen.insert(name.as_str()))
        .filter_map(|name| definitions.font_data.get(name).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proportional_faces_come_before_monospace_and_unnamed_ones() {
        let mut definitions = egui::FontDefinitions::default();
        let face = |byte: u8| Arc::new(egui::FontData::from_owned(vec![byte]));
        definitions.font_data.clear();
        definitions.families.clear();
        for (name, byte) in [("a-mono", 1), ("b-unnamed", 2), ("z-sans", 3), ("y-cjk", 4)] {
            definitions.font_data.insert(name.into(), face(byte));
        }
        definitions.families.insert(
            egui::FontFamily::Monospace,
            vec!["a-mono".into(), "z-sans".into()],
        );
        definitions.families.insert(
            egui::FontFamily::Proportional,
            vec!["z-sans".into(), "y-cjk".into()],
        );
        let order: Vec<u8> = by_priority(&definitions)
            .iter()
            .map(|f| f.font[0])
            .collect();
        assert_eq!(order, [3, 4, 1, 2]);
    }
}
