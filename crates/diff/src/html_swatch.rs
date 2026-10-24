//! Colour swatch rendering for the HTML changelog.

use figstudio_fig::Val;

use crate::html_changes::num;

pub(crate) type Channels<'a> = [Option<&'a Val>; 4];

/// Swatch plus hex for one side; a channel missing here falls back to the other side's value.
pub(crate) fn swatch(side: &Channels, other: &Channels) -> String {
    if side.iter().all(Option::is_none) {
        return "(none)".into();
    }
    let get = |i: usize, default: f64| num(side[i]).or_else(|| num(other[i])).unwrap_or(default);
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b, a) = (
        byte(get(0, 0.0)),
        byte(get(1, 0.0)),
        byte(get(2, 0.0)),
        get(3, 1.0),
    );
    let alpha = if (a - 1.0).abs() > f64::EPSILON {
        format!(" α{:.2}", a.clamp(0.0, 1.0))
    } else {
        String::new()
    };
    format!(
        "<i class=\"sw\" style=\"background:rgba({r},{g},{b},{:.3})\"></i>#{r:02X}{g:02X}{b:02X}{alpha}",
        a.clamp(0.0, 1.0)
    )
}
