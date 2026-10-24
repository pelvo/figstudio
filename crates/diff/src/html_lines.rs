//! Escaped, per-area change lines, collapsed in first-seen report order.
use crate::html::escape;
use figstudio_fig::Guid;
use std::fmt::Write;

pub(crate) struct ChangeLine {
    pub id: Guid,
    pub sign: &'static str,
    pub class: &'static str,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub rows: Vec<String>,
}

fn groups<'a>(lines: impl Iterator<Item = &'a ChangeLine>) -> Vec<(&'a ChangeLine, usize)> {
    let mut groups: Vec<(&ChangeLine, usize)> = Vec::new();
    for line in lines {
        if let Some((_, count)) = groups.iter_mut().find(|(l, _)| {
            l.sign == line.sign && l.name == line.name && l.kind == line.kind && l.rows == line.rows
        }) {
            *count += 1;
        } else {
            groups.push((line, 1));
        }
    }
    groups
}

pub(crate) fn count_lines<'a>(lines: impl Iterator<Item = &'a ChangeLine>) -> usize {
    groups(lines).len()
}

pub(crate) fn render_lines<'a>(out: &mut String, lines: impl Iterator<Item = &'a ChangeLine>) {
    out.push_str("<ul class=\"lines\">");
    for (line, count) in groups(lines) {
        let _ = write!(
            out,
            "<li class=\"ln {}\" title=\"{}\"><span class=\"sg\">{}</span><span class=\"hd\"><span class=\"nm\">{}</span><span class=\"kd\">{}</span>",
            line.class,
            escape(&line.path),
            line.sign,
            escape(&line.name),
            escape(&line.kind)
        );
        if count > 1 {
            let _ = write!(out, "<span class=\"ct\">×{count}</span>");
        }
        out.push_str("</span>");
        if !line.rows.is_empty() {
            let _ = write!(out, "<span class=\"dt\">{}</span>", line.rows.join(""));
        }
        out.push_str("</li>");
    }
    out.push_str("</ul>");
}
