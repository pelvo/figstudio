//! The result of a tool call, independent of any MCP SDK.
//!
//! The server adapter turns an [`Output`] into its protocol type; the app can show one directly.
use serde_json::Value;

/// One item of a tool reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// Plain text.
    Text(String),
    /// A base64 image.
    Image {
        /// Base64 bytes.
        data_b64: String,
        /// MIME type, for example `image/png`.
        mime: String,
    },
    /// Base64 bytes of a type that is not an image.
    Blob {
        /// Base64 bytes.
        data_b64: String,
        /// Resource URI.
        uri: String,
        /// MIME type.
        mime: String,
    },
}

impl Block {
    /// A text block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// An image block.
    pub fn image(data_b64: impl Into<String>, mime: impl Into<String>) -> Self {
        Self::Image {
            data_b64: data_b64.into(),
            mime: mime.into(),
        }
    }

    /// The text, when this is a text block.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }
}

/// A finished tool reply: content blocks, optional structured JSON, and an error flag.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// The reply items, in order.
    pub content: Vec<Block>,
    /// Structured JSON, when the tool has a schema-shaped reply.
    pub structured: Option<Value>,
    /// Whether the tool ran and failed.
    pub is_error: bool,
}

impl Output {
    /// A successful reply.
    pub fn success(content: Vec<Block>) -> Self {
        Self {
            content,
            structured: None,
            is_error: false,
        }
    }

    /// A failed reply with caller-visible content.
    pub fn error(content: Vec<Block>) -> Self {
        Self {
            is_error: true,
            ..Self::success(content)
        }
    }

    /// A failed reply with one text block.
    pub fn fault(message: impl Into<String>) -> Self {
        Self::error(vec![Block::text(message)])
    }

    /// A successful reply whose only text is the JSON text of `value`, which is also the structured part.
    pub fn structured(value: Value) -> Self {
        Self {
            content: vec![Block::text(value.to_string())],
            structured: Some(value),
            is_error: false,
        }
    }
}

/// The same blocks with every text block before every image or blob, so a summary reaches the
/// reader even when the reply is cut after the media.
pub fn text_first(content: Vec<Block>) -> Vec<Block> {
    let (mut text, media): (Vec<_>, Vec<_>) =
        content.into_iter().partition(|b| b.as_text().is_some());
    text.extend(media);
    text
}
