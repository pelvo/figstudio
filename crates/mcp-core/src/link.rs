use crate::{
    CoreError,
    node_id::{NodeId, parse_bare, parse_guid},
};
use figstudio_fig::Guid;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use url::Url;

/// Everything except `A-Za-z0-9-._~` is percent-encoded, so any file key survives a URL.
const ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Largest absolute value of a region coordinate.
const MAX_COORD: f64 = 1e7;

/// Which file a link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileHint {
    /// The file key of a `figstudio://` link: a registered file's stem.
    Exact(String),
    /// A figma.com link: the file key and the URL slug, which may match a local file's name.
    Figma {
        /// The `<key>` segment after `/design/`, `/file/` or `/board/`.
        key: String,
        /// The `<slug>` segment after the key, when present.
        slug: Option<String>,
    },
}

/// A parsed `figstudio://` or figma.com link.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// The file the link names; `None` when a figma.com URL has no file part.
    pub file: Option<FileHint>,
    /// `node-id`.
    pub node: Option<NodeId>,
    /// `page` of a region link.
    pub page: Option<Guid>,
    /// `rect` of a region link: `[x, y, w, h]` in page coordinates.
    pub rect: Option<[f64; 4]>,
}

/// Parse a link.
///
/// - `figstudio://<key>?node-id=1-42` or `figstudio://<key>?page=0-1&rect=x,y,w,h`; the key is
///   percent-encoded. Exactly one of `node-id` and `page`+`rect`; other parameters are rejected.
/// - `http(s)://(www.)figma.com/(design|file|board)/<key>/<slug>?node-id=…`. The file part may
///   be missing or unrecognised (then `file` is `None`); other parameters are ignored.
///
/// # Errors
/// [`CoreError`] (invalid link) for any other scheme or host, a malformed or duplicated
/// parameter, or a rect that is not 4 finite numbers with `w`, `h` > 0 and `|v|` ≤ 1e7.
pub fn parse_link(input: &str) -> Result<Link, CoreError> {
    let url = Url::parse(input).map_err(|_| CoreError::invalid_link())?;
    match url.scheme() {
        "figstudio" => figstudio(&url),
        "http" | "https" => figma(&url),
        _ => Err(CoreError::invalid_link()),
    }
}

/// `figstudio://<key>?node-id=…` for a node or an instance path.
pub fn format_node_link(key: &str, node: &NodeId) -> String {
    let id = match node {
        NodeId::Node(guid) => dashed(*guid),
        NodeId::InstancePath(path) => {
            let guids = std::iter::once(path.instance()).chain(path.keys().iter().copied());
            format!("I{}", guids.map(dashed).collect::<Vec<_>>().join(";"))
        }
    };
    format!("figstudio://{}?node-id={}", encode(key), encode(&id))
}

/// `figstudio://<key>?page=…&rect=x,y,w,h`, with each number rounded to 2 decimals.
///
/// # Errors
/// [`CoreError`] (invalid link) when the rounded rect is not one [`parse_link`] accepts.
pub fn format_region_link(key: &str, page: Guid, rect: [f64; 4]) -> Result<String, CoreError> {
    let rect = rect.map(number).join(",");
    parse_rect(&rect)?;
    Ok(format!(
        "figstudio://{}?page={}&rect={rect}",
        encode(key),
        dashed(page)
    ))
}

fn figstudio(url: &Url) -> Result<Link, CoreError> {
    let bad = CoreError::invalid_link;
    let key = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(bad)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_none()
        || url
            .query_pairs()
            .any(|(name, _)| !matches!(&*name, "node-id" | "page" | "rect"))
    {
        return Err(bad());
    }
    let file = percent_decode_str(key).decode_utf8().map_err(|_| bad())?;
    let (node, page, rect) = (
        param(url, "node-id")?,
        param(url, "page")?,
        param(url, "rect")?,
    );
    let link = match (node, page, rect) {
        (Some(node), None, None) => Link {
            node: Some(parse_bare(&node).map_err(|_| bad())?),
            page: None,
            rect: None,
            file: None,
        },
        (None, Some(page), Some(rect)) => Link {
            node: None,
            page: Some(parse_guid(&page).map_err(|_| bad())?),
            rect: Some(parse_rect(&rect)?),
            file: None,
        },
        _ => return Err(bad()),
    };
    Ok(Link {
        file: Some(FileHint::Exact(file.into_owned())),
        ..link
    })
}

fn figma(url: &Url) -> Result<Link, CoreError> {
    if !matches!(url.host_str(), Some("figma.com" | "www.figma.com")) {
        return Err(CoreError::invalid_link());
    }
    let node = param(url, "node-id")?
        .map(|id| parse_bare(&id).map_err(|_| CoreError::invalid_link()))
        .transpose()?;
    let mut segments = url.path_segments().into_iter().flatten().map(decode);
    let file = match (segments.next().as_deref(), segments.next()) {
        (Some("design" | "file" | "board"), Some(key)) if !key.is_empty() => {
            Some(FileHint::Figma {
                key,
                slug: segments.next().filter(|slug| !slug.is_empty()),
            })
        }
        _ => None,
    };
    Ok(Link {
        file,
        node,
        page: None,
        rect: None,
    })
}

/// The one value of query parameter `name`; two of the same name are an error.
fn param(url: &Url, name: &str) -> Result<Option<String>, CoreError> {
    let mut values = url.query_pairs().filter(|(key, _)| key == name);
    let value = values.next().map(|(_, value)| value.into_owned());
    match values.next() {
        Some(_) => Err(CoreError::invalid_link()),
        None => Ok(value),
    }
}

fn parse_rect(input: &str) -> Result<[f64; 4], CoreError> {
    let bad = CoreError::invalid_link;
    let parts = input
        .split(',')
        .map(|part| part.parse::<f64>().map_err(|_| bad()))
        .collect::<Result<Vec<_>, _>>()?;
    let rect: [f64; 4] = parts.try_into().map_err(|_| bad())?;
    let valid = rect.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
        && rect[2] > 0.0
        && rect[3] > 0.0;
    valid.then_some(rect).ok_or_else(bad)
}

/// Two decimals without trailing zeros; never `-0`.
fn number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text == "-0" { "0" } else { text }.to_owned()
}

fn dashed(Guid(session, local): Guid) -> String {
    format!("{session}-{local}")
}

fn encode(text: &str) -> String {
    utf8_percent_encode(text, ENCODE).to_string()
}

fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}
