//! Image delivery: native WebP custom formats, egui PNG, browser downloads.
use crate::region_png::{Done, message};
use figstudio_render::encoding::ImageFormat;
use std::cell::RefCell;

/// Kept alive for the app lifetime: X11 owns clipboard contents through its context.
#[derive(Default)]
pub struct Clipboard {
    #[cfg(not(target_arch = "wasm32"))]
    context: Option<clipboard_rs::ClipboardContext>,
}

impl std::fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clipboard").finish_non_exhaustive()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Clipboard {
    fn webp(&mut self, bytes: Vec<u8>, png: Vec<u8>) -> anyhow::Result<()> {
        use clipboard_rs::{Clipboard as _, ClipboardContext};
        if self.context.is_none() {
            self.context = Some(ClipboardContext::new().map_err(|e| anyhow::anyhow!("{e}"))?);
        }
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No clipboard"))?;
        let contents = contents(bytes, png);
        context.set(contents).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// WebP under its platform types plus a PNG, in one write, so apps that cannot read WebP
/// still paste the PNG. (`clipboard-rs` without its `image` feature only takes raw types.)
#[cfg(not(target_arch = "wasm32"))]
fn contents(webp: Vec<u8>, png: Vec<u8>) -> Vec<clipboard_rs::ClipboardContent> {
    use clipboard_rs::ClipboardContent::Other;
    #[cfg(target_os = "macos")]
    return vec![
        Other("org.webmproject.webp".into(), webp.clone()),
        Other("public.webp".into(), webp),
        Other("public.png".into(), png),
    ];
    #[cfg(target_os = "windows")]
    return vec![Other("image/webp".into(), webp), Other("PNG".into(), png)];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return vec![
        Other("image/webp".into(), webp),
        Other("image/png".into(), png),
    ];
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use clipboard_rs::ContentFormat;
    use clipboard_rs::common::ContentData;

    #[test]
    fn a_webp_copy_also_carries_the_png() {
        let contents = super::contents(b"webp".to_vec(), b"png".to_vec());
        let named = |data: &[u8]| {
            contents
                .iter()
                .filter(|c| c.as_bytes() == data)
                .map(|c| match c.get_format() {
                    ContentFormat::Other(name) => name,
                    _ => panic!("not a raw clipboard type"),
                })
                .collect::<Vec<_>>()
        };
        assert!(
            named(b"webp")
                .iter()
                .all(|n| n.to_lowercase().contains("webp"))
        );
        assert!(!named(b"webp").is_empty());
        let png = named(b"png");
        assert_eq!(png.len(), 1);
        assert!(png[0].to_lowercase().contains("png"), "{png:?}");
    }
}

/// Returns the toast and whether delivery failed; never claims a failed write succeeded.
pub fn finish(ctx: &egui::Context, done: Done, clipboard: &RefCell<Clipboard>) -> (String, bool) {
    #[cfg(not(target_arch = "wasm32"))]
    if done.format == ImageFormat::Webp {
        return match done
            .encoded
            .zip(done.png_twin)
            .ok_or_else(|| anyhow::anyhow!("Could not encode WebP"))
            .and_then(|(webp, png)| clipboard.borrow_mut().webp(webp, png))
        {
            Ok(()) => (
                format!(
                    "Copied WebP ({} × {} px){}",
                    done.width,
                    done.height,
                    if done.gaps.is_empty() {
                        String::new()
                    } else {
                        format!("\n{}", done.gaps)
                    }
                ),
                false,
            ),
            Err(e) => (
                format!(
                    "Could not copy WebP: {}",
                    figstudio_fig::one_line(e.as_ref())
                ),
                true,
            ),
        };
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = clipboard;
        if done.format == ImageFormat::Webp || !can_copy_image() {
            let file = format!("{}.{}", done.name, done.format.extension());
            return match done
                .encoded
                .as_deref()
                .map(|p| crate::web::download(&file, done.format.mime(), p))
            {
                Some(Ok(())) => (
                    if done.format == ImageFormat::Webp {
                        format!("Browsers cannot copy WebP; downloaded {file}")
                    } else {
                        format!("This browser cannot copy images here; downloaded {file}")
                    },
                    false,
                ),
                Some(Err(e)) => (figstudio_fig::one_line(e.as_ref()), true),
                None => ("Could not make the image".into(), true),
            };
        }
    }
    let image = egui::ColorImage::from_rgba_premultiplied([done.width, done.height], &done.rgba);
    ctx.copy_image(image);
    let text = message(done.width, done.height, &done.gaps);
    #[cfg(not(target_arch = "wasm32"))]
    let text = text + "\nIf pasting gives nothing, this system cannot copy images.";
    (text, false)
}

/// Safari may still refuse a PNG write later if its user gesture has expired.
#[cfg(target_arch = "wasm32")]
fn can_copy_image() -> bool {
    use wasm_bindgen::JsValue;
    let global = js_sys::global();
    let secure = js_sys::Reflect::get(&global, &JsValue::from_str("isSecureContext"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    secure && js_sys::Reflect::has(&global, &JsValue::from_str("ClipboardItem")).unwrap_or(false)
}
