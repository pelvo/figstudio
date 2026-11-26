//! The Design panel's controls: a number field whose label scrubs, a colour row (swatch,
//! hex, opacity), and small helpers. Values from several selected layers that differ show as
//! `Mixed`; editing one sets them all.

use egui::{Color32, Id, Response, RichText, Sense, TextEdit, Ui, vec2};

/// The one value every selected layer has, or `None` when they differ (`Mixed`).
pub(crate) fn shared(values: &[f64]) -> Option<f64> {
    let first = *values.first()?;
    values
        .iter()
        .all(|v| (v - first).abs() < 1e-6)
        .then_some(first)
}

/// Shows `v` the way the panel does: up to two decimals, no trailing zeros.
pub(crate) fn fmt(v: f64) -> String {
    // `+ 0.0` turns -0 into 0, so a rotation does not read "-0".
    let r = (v * 100.0).round() / 100.0 + 0.0;
    if r.fract() == 0.0 {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}

/// Parses a typed value: a number, or two numbers joined by `+ - * /` (`120/2`, `8*3`). A
/// leading operator applies to `current` (`+10` adds ten).
pub(crate) fn parse(text: &str, current: Option<f64>) -> Option<f64> {
    let t = text.trim().trim_end_matches(['%', '°']).trim();
    if t.is_empty() {
        return None;
    }
    // A leading `+`, `*` or `/` applies to the current value (`-` starts a negative number).
    let relative = t.starts_with(['+', '*', '/']);
    if !relative && let Ok(v) = t.parse::<f64>() {
        return v.is_finite().then_some(v);
    }
    // shortcut: one binary operator, no precedence or parentheses; extend to a small
    // expression parser if users type longer sums.
    let (lhs, op, rhs) = t
        .char_indices()
        .skip(1)
        .find(|(_, c)| matches!(c, '+' | '-' | '*' | '/'))
        .map(|(i, c)| (t[..i].trim(), c, t[i + 1..].trim()))
        .or_else(|| {
            let c = t.chars().next()?;
            matches!(c, '+' | '-' | '*' | '/').then(|| ("", c, t[1..].trim()))
        })?;
    let a = if lhs.is_empty() {
        current?
    } else {
        lhs.parse::<f64>().ok()?
    };
    let b = rhs.parse::<f64>().ok()?;
    let v = match op {
        '+' => a + b,
        '-' => a - b,
        '*' => a * b,
        _ if b != 0.0 => a / b,
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// A number field: `label` (one or two characters) scrubs the value when dragged; the text
/// box takes a number or a small expression and commits on Enter or when it loses focus.
/// `None` shows `Mixed`. Returns the new value while scrubbing or once committed.
pub(crate) fn num_field(
    ui: &mut Ui,
    label: &str,
    name: &str,
    value: Option<f64>,
    speed: f64,
    width: f32,
) -> Option<f64> {
    num_field_or(ui, label, name, value, speed, width, "Mixed")
}

/// `num_field` whose empty value shows `hint` (such as `Auto`).
pub(crate) fn num_field_or(
    ui: &mut Ui,
    label: &str,
    name: &str,
    value: Option<f64>,
    speed: f64,
    width: f32,
    hint: &str,
) -> Option<f64> {
    let p = crate::theme::of(ui);
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let scrub =
            ui.add(egui::Label::new(RichText::new(label).color(p.text_faint)).sense(Sense::drag()));
        let scrub = scrub.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        crate::a11y::author(&scrub, format!("design.{}.scrub", crate::a11y::slug(name)));
        if scrub.dragged() {
            let d = f64::from(scrub.drag_delta().x) * speed;
            if d != 0.0 {
                out = Some(value.unwrap_or(0.0) + d);
            }
        }
        let id = Id::new(("design-field", name));
        let shown = value.map(fmt).unwrap_or_default();
        let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| shown.clone());
        let edit = ui.add(
            TextEdit::singleline(&mut text)
                .desired_width(width)
                .hint_text(if value.is_none() { hint } else { "" }),
        );
        crate::a11y::name_field(&edit, name);
        crate::a11y::author(&edit, format!("design.{}", crate::a11y::slug(name)));
        let commit = edit.lost_focus()
            || (edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if edit.has_focus() && !commit {
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
        } else {
            ui.data_mut(|d| d.remove::<String>(id));
        }
        if commit
            && text != shown
            && let Some(v) = parse(&text, value)
        {
            out = Some(v);
        }
    });
    out
}

/// A one-line text field showing `current` that keeps a draft while focused and returns the
/// new text on Enter or when it loses focus (unchanged or blank text returns nothing).
pub(crate) fn text_field(ui: &mut Ui, name: &str, current: &str, width: f32) -> Option<String> {
    let id = Id::new(("design-text", name));
    let mut text: String = ui
        .data(|d| d.get_temp(id))
        .unwrap_or_else(|| current.to_string());
    let edit = ui.add(TextEdit::singleline(&mut text).desired_width(width));
    crate::a11y::name_field(&edit, name);
    crate::a11y::author(&edit, format!("design.{}", crate::a11y::slug(name)));
    let commit =
        edit.lost_focus() || (edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    if edit.has_focus() && !commit {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    let t = text.trim();
    (commit && t != current && !t.is_empty()).then(|| t.to_string())
}

/// sRGB bytes of a `Color` value and a paint opacity.
pub(crate) fn rgba_of(color: Option<&figstudio_fig::Val>, opacity: f64) -> [u8; 4] {
    let c = |k: &str| {
        color
            .and_then(|c| c.num(k))
            .map_or(0.0, f64::from)
            .clamp(0.0, 1.0)
    };
    let b = |v: f64| (v * 255.0).round() as u8;
    [b(c("r")), b(c("g")), b(c("b")), b(opacity.clamp(0.0, 1.0))]
}

pub(crate) fn hex_of(rgba: [u8; 4]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2])
}

/// A change made in a colour row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ColourChange {
    /// `#RRGGBB`.
    Hex(String),
    /// `0..=1`.
    Opacity(f64),
}

/// Swatch (opens a picker), hex field (kept as a draft while typed, committed when complete)
/// and opacity %. `name` labels the controls for screen readers.
pub(crate) fn colour_row(ui: &mut Ui, name: &str, rgba: [u8; 4]) -> Option<ColourChange> {
    let mut out = None;
    let mut colour = Color32::from_rgb(rgba[0], rgba[1], rgba[2]);
    let swatch = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut colour,
        egui::color_picker::Alpha::Opaque,
    );
    crate::a11y::name_field(&swatch, &format!("{name} colour"));
    crate::a11y::author(
        &swatch,
        format!("design.{}.swatch", crate::a11y::slug(name)),
    );
    if swatch.changed() {
        out = Some(ColourChange::Hex(format!(
            "#{:02X}{:02X}{:02X}",
            colour.r(),
            colour.g(),
            colour.b()
        )));
    }
    // A partial hex (a deleted digit, a replacement being typed) stays while focused.
    let draft_id = Id::new(("design-hex", name));
    let mut hex: String = ui
        .data(|d| d.get_temp(draft_id))
        .unwrap_or_else(|| hex_of(rgba));
    let field = ui.add(TextEdit::singleline(&mut hex).desired_width(64.0));
    crate::a11y::name_field(&field, &format!("{name} colour hex"));
    crate::a11y::author(&field, format!("design.{}.hex", crate::a11y::slug(name)));
    if field.has_focus() {
        ui.data_mut(|d| d.insert_temp(draft_id, hex.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(draft_id));
    }
    let digits = hex.trim().trim_start_matches('#');
    if field.changed() && digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit()) {
        out = Some(ColourChange::Hex(format!(
            "#{}",
            digits.to_ascii_uppercase()
        )));
    }
    let pct = f64::from(rgba[3]) / 255.0 * 100.0;
    if let Some(v) = num_field(
        ui,
        "%",
        &format!("{name} opacity"),
        Some(pct.round()),
        1.0,
        34.0,
    ) {
        out = Some(ColourChange::Opacity((v / 100.0).clamp(0.0, 1.0)));
    }
    out
}

/// A small icon-free toggle button showing `on` state; returns the click.
pub(crate) fn toggle(ui: &mut Ui, text: &str, name: &str, on: bool) -> Response {
    let r = ui.add(egui::Button::selectable(on, text).min_size(vec2(22.0, 20.0)));
    crate::a11y::name_field(&r, name);
    crate::a11y::author(&r, format!("design.{}", crate::a11y::slug(name)));
    r
}

/// A small text button for a command.
pub(crate) fn button(ui: &mut Ui, text: &str, name: &str) -> Response {
    let r = ui.add(egui::Button::new(text).min_size(vec2(22.0, 20.0)));
    crate::a11y::name_field(&r, name);
    crate::a11y::author(&r, format!("design.{}", crate::a11y::slug(name)));
    r.on_hover_text(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_values_take_numbers_and_one_operator() {
        assert_eq!(parse("12.5", None), Some(12.5));
        assert_eq!(parse("120/2", None), Some(60.0));
        assert_eq!(parse("8*3", None), Some(24.0));
        assert_eq!(parse("+10", Some(5.0)), Some(15.0));
        assert_eq!(
            parse("-3", Some(5.0)),
            Some(-3.0),
            "a negative number, not a difference"
        );
        assert_eq!(parse("50%", None), Some(50.0));
        assert_eq!(parse("1/0", None), None);
        assert_eq!(parse("abc", None), None);
        assert_eq!(parse("*2", None), None, "needs a current value");
    }

    #[test]
    fn shared_values_or_mixed() {
        assert_eq!(shared(&[3.0, 3.0]), Some(3.0));
        assert_eq!(shared(&[3.0, 4.0]), None);
        assert_eq!(shared(&[]), None);
    }
}
