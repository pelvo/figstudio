mod support;

use figstudio_diff::{Category, diff_documents};
use figstudio_fig::Val;
use support::{document, object, rectangle};

#[test]
fn actual_field_families_have_categories() {
    let fields = [
        ("fontSize", Category::Font),
        ("fillPaints", Category::Fill),
        ("strokeWeight", Category::Stroke),
        ("transform", Category::Geometry),
        ("stackHorizontalPadding", Category::Layout),
        ("effects", Category::Effect),
        ("componentPropAssignments", Category::Component),
        ("characters", Category::Text),
        ("unrecognizedField", Category::Other),
    ];
    let mut after = rectangle(2, 1);
    for (field, _) in fields {
        after.data.set(
            field,
            if field == "unrecognizedField" {
                object(&[])
            } else {
                Val::Int(1)
            },
        );
    }
    let report = diff_documents(&document(vec![rectangle(2, 1)]), &document(vec![after]));
    let changes = &report.pages[0].changed[0].changes;
    assert_eq!(changes.len(), fields.len());
    for (field, expected) in fields {
        assert_eq!(
            changes.iter().find(|c| c.field == field).unwrap().category,
            expected,
            "{field}"
        );
    }
}
