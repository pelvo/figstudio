//! Shared lossless image argument for native MCP tools.
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ImageFormat {
    #[default]
    Png,
    Webp,
}

impl From<ImageFormat> for figstudio_render::encoding::ImageFormat {
    fn from(value: ImageFormat) -> Self {
        match value {
            ImageFormat::Png => Self::Png,
            ImageFormat::Webp => Self::Webp,
        }
    }
}
