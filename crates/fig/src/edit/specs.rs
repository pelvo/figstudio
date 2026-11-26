//! Typed paint, effect and stroke specs shared by the edit ops, MCP JSON and the GUI.
//!
//! A spec converts to the node's Kiwi value with only the fields it sets; a patch changes
//! named fields of an existing item and keeps every other field (ground rule 7).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::edit::space::Xform;
use crate::{FigError, Obj, Val, intern};

/// `#RRGGBB` or `#RRGGBBAA` as Figma's `Color` (alpha in `a`).
pub fn color_val(hex: &str) -> Result<Val, FigError> {
    let s = hex.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    let bytes = if s.is_ascii() {
        hex::decode(s).ok()
    } else {
        None
    };
    let (r, g, b, a) = match bytes.as_deref() {
        Some([r, g, b]) => (*r, *g, *b, 255),
        Some([r, g, b, a]) => (*r, *g, *b, *a),
        _ => return Err(FigError::malformed(format!("invalid hex colour {hex:?}"))),
    };
    let c = |v: u8| Val::Float(f64::from(v) / 255.0);
    Ok(Val::Obj(Obj::from([
        ("r", c(r)),
        ("g", c(g)),
        ("b", c(b)),
        ("a", c(a)),
    ])))
}

fn unit(v: f64, what: &str) -> Result<f64, FigError> {
    if v.is_finite() {
        Ok(v.clamp(0.0, 1.0))
    } else {
        Err(FigError::malformed(format!(
            "{what} must be a finite number"
        )))
    }
}

fn finite(v: f64, what: &str) -> Result<f64, FigError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(FigError::malformed(format!(
            "{what} must be a finite number"
        )))
    }
}

/// Which paint list of a node an op edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintSlot {
    Fill,
    Stroke,
}

impl PaintSlot {
    /// The node field holding the list.
    pub fn field(self) -> &'static str {
        match self {
            PaintSlot::Fill => "fillPaints",
            PaintSlot::Stroke => "strokePaints",
        }
    }

    /// The node field that binds the list to a style (dropped when the list is edited).
    pub fn style_field(self) -> &'static str {
        match self {
            PaintSlot::Fill => "styleIdForFill",
            PaintSlot::Stroke => "styleIdForStrokeFill",
        }
    }
}

/// A gradient's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientKind {
    Linear,
    Radial,
    Angular,
    Diamond,
}

impl GradientKind {
    fn paint_type(self) -> &'static str {
        match self {
            GradientKind::Linear => "GRADIENT_LINEAR",
            GradientKind::Radial => "GRADIENT_RADIAL",
            GradientKind::Angular => "GRADIENT_ANGULAR",
            GradientKind::Diamond => "GRADIENT_DIAMOND",
        }
    }
}

/// One gradient colour stop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StopSpec {
    /// `0..=1` along the gradient.
    pub position: f64,
    /// `#RRGGBB` or `#RRGGBBAA`.
    pub hex: String,
}

/// How an image fills its layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageScale {
    #[default]
    Fill,
    Fit,
    /// Figma's "Crop" (stored as `STRETCH` with a crop transform).
    Crop,
    Tile,
}

impl ImageScale {
    fn stored(self) -> &'static str {
        match self {
            ImageScale::Fill => "FILL",
            ImageScale::Fit => "FIT",
            ImageScale::Crop => "STRETCH",
            ImageScale::Tile => "TILE",
        }
    }
}

/// A paint to put in a fill or stroke list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaintSpec {
    Solid {
        /// `#RRGGBB`, or `#RRGGBBAA` whose alpha becomes the paint opacity.
        hex: String,
        #[serde(default)]
        opacity: Option<f64>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        blend_mode: Option<String>,
    },
    Gradient {
        kind: GradientKind,
        stops: Vec<StopSpec>,
        /// Start handle in the layer's unit square (default `[0, 0.5]`).
        #[serde(default)]
        from: Option<[f64; 2]>,
        /// End handle in the layer's unit square (default `[1, 0.5]`).
        #[serde(default)]
        to: Option<[f64; 2]>,
        #[serde(default)]
        opacity: Option<f64>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        blend_mode: Option<String>,
    },
    Image {
        /// 40-hex SHA-1 of an image in the file (`import_image` adds one).
        hash: String,
        #[serde(default)]
        scale: ImageScale,
        /// Quarter turns are what Figma offers; any angle is stored.
        #[serde(default)]
        rotation: Option<f64>,
        /// Tile size factor (`tile` only).
        #[serde(default)]
        tile_scale: Option<f64>,
        /// Image adjustments (`exposure`, `contrast`, `saturation`, `temperature`, `tint`,
        /// `highlights`, `shadows`), each `-1..=1`.
        #[serde(default)]
        filters: Option<BTreeMap<String, f64>>,
        #[serde(default)]
        opacity: Option<f64>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        blend_mode: Option<String>,
    },
}

/// Gradient `transform` for handles `from`→`to` in the unit square: the stored matrix maps the
/// layer's unit square into gradient space, where the gradient runs `(0, .5)`→`(1, .5)`.
fn gradient_transform(from: [f64; 2], to: [f64; 2]) -> Result<Val, FigError> {
    let [fx, fy] = from;
    let [tx, ty] = to;
    let (dx, dy) = (tx - fx, ty - fy);
    // Gradient space → unit square: x along from→to, y along its perpendicular.
    let to_unit = Xform {
        m00: dx,
        m01: -dy,
        m02: fx + dy * 0.5,
        m10: dy,
        m11: dx,
        m12: fy - dx * 0.5,
    };
    to_unit
        .inverse()
        .map(Xform::to_val)
        .ok_or_else(|| FigError::malformed("gradient handles must not coincide"))
}

fn stops_val(stops: &[StopSpec]) -> Result<Val, FigError> {
    if stops.is_empty() {
        return Err(FigError::malformed("a gradient needs at least one stop"));
    }
    let mut out = Vec::with_capacity(stops.len());
    for s in stops {
        out.push(Val::Obj(Obj::from([
            ("color", color_val(&s.hex)?),
            ("position", Val::Float(unit(s.position, "stop position")?)),
        ])));
    }
    Ok(Val::Arr(out))
}

fn common(
    o: &mut Obj,
    opacity: Option<f64>,
    visible: Option<bool>,
    blend: Option<&str>,
) -> Result<(), FigError> {
    if let Some(v) = opacity {
        o.insert("opacity", Val::Float(unit(v, "opacity")?));
    }
    o.insert("visible", Val::Bool(visible.unwrap_or(true)));
    o.insert("blendMode", Val::Enum(intern(blend.unwrap_or("NORMAL"))));
    Ok(())
}

fn filters_val(filters: &BTreeMap<String, f64>) -> Result<Val, FigError> {
    let mut o = Obj::new();
    for (k, v) in filters {
        let key = match k.as_str() {
            "exposure" | "contrast" | "temperature" | "tint" | "highlights" | "shadows" => {
                k.as_str()
            }
            // Figma's panel says saturation; the file field is `vibrance`.
            "saturation" => "vibrance",
            other => {
                return Err(FigError::malformed(format!(
                    "unknown image filter {other:?}"
                )));
            }
        };
        o.insert(key, Val::Float(finite(*v, k)?.clamp(-1.0, 1.0)));
    }
    Ok(Val::Obj(o))
}

fn image_hash(hash: &str) -> Result<Val, FigError> {
    let raw = (hash.len() == 40 && hash.is_ascii())
        .then(|| hex::decode(hash).ok())
        .flatten()
        .ok_or_else(|| FigError::malformed(format!("invalid 40-hex image SHA-1 {hash:?}")))?;
    Ok(Val::Obj(Obj::from([
        ("hash", Val::Bytes(raw)),
        ("name", Val::Str(hash.to_ascii_lowercase())),
    ])))
}

impl PaintSpec {
    /// The `Paint` value.
    pub fn to_val(&self) -> Result<Val, FigError> {
        let mut o = Obj::new();
        match self {
            PaintSpec::Solid {
                hex,
                opacity,
                visible,
                blend_mode,
            } => {
                let mut color = color_val(hex)?;
                // The hex alpha is the paint's opacity; the colour itself stays opaque.
                let alpha = color.get("a").and_then(Val::as_f32).map_or(1.0, f64::from);
                color.set("a", Val::Float(1.0));
                o.insert("type", Val::Enum(intern("SOLID")));
                o.insert("color", color);
                common(
                    &mut o,
                    Some(opacity.unwrap_or(1.0) * alpha),
                    *visible,
                    blend_mode.as_deref(),
                )?;
            }
            PaintSpec::Gradient {
                kind,
                stops,
                from,
                to,
                opacity,
                visible,
                blend_mode,
            } => {
                o.insert("type", Val::Enum(intern(kind.paint_type())));
                o.insert("stops", stops_val(stops)?);
                o.insert(
                    "transform",
                    gradient_transform(from.unwrap_or([0.0, 0.5]), to.unwrap_or([1.0, 0.5]))?,
                );
                common(
                    &mut o,
                    Some(opacity.unwrap_or(1.0)),
                    *visible,
                    blend_mode.as_deref(),
                )?;
            }
            PaintSpec::Image {
                hash,
                scale,
                rotation,
                tile_scale,
                filters,
                opacity,
                visible,
                blend_mode,
            } => {
                o.insert("type", Val::Enum(intern("IMAGE")));
                o.insert("image", image_hash(hash)?);
                o.insert("imageScaleMode", Val::Enum(intern(scale.stored())));
                if *scale == ImageScale::Crop {
                    o.insert("transform", Xform::IDENTITY.to_val());
                }
                if let Some(r) = rotation {
                    o.insert("rotation", Val::Float(finite(*r, "rotation")?));
                }
                if let Some(s) = tile_scale {
                    o.insert("scale", Val::Float(finite(*s, "tile scale")?.max(0.01)));
                }
                if let Some(f) = filters {
                    o.insert("paintFilter", filters_val(f)?);
                }
                common(
                    &mut o,
                    Some(opacity.unwrap_or(1.0)),
                    *visible,
                    blend_mode.as_deref(),
                )?;
            }
        }
        Ok(Val::Obj(o))
    }
}

/// Changes to one existing paint; unset fields stay as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PaintPatch {
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub opacity: Option<f64>,
    #[serde(default)]
    pub blend_mode: Option<String>,
    /// Solid paints: the colour (`#RRGGBBAA` alpha becomes the opacity).
    #[serde(default)]
    pub hex: Option<String>,
    /// Gradients: the stops.
    #[serde(default)]
    pub stops: Option<Vec<StopSpec>>,
    /// Gradients: both handles (each defaults to the paint's current handle when omitted is
    /// not supported; give both).
    #[serde(default)]
    pub handles: Option<[[f64; 2]; 2]>,
    /// Images: the scale mode.
    #[serde(default)]
    pub scale: Option<ImageScale>,
    #[serde(default)]
    pub rotation: Option<f64>,
    #[serde(default)]
    pub filters: Option<BTreeMap<String, f64>>,
}

impl PaintPatch {
    /// Applies the patch to `paint` in place.
    pub fn apply(&self, paint: &mut Val) -> Result<(), FigError> {
        let kind = paint.text("type").unwrap_or("SOLID").to_string();
        let gradient = kind.starts_with("GRADIENT_");
        let wrong = |what: &str| {
            Err(FigError::malformed(format!(
                "{what} does not apply to a {kind} paint"
            )))
        };
        if let Some(v) = self.visible {
            paint.set("visible", Val::Bool(v));
        }
        if let Some(v) = self.opacity {
            paint.set("opacity", Val::Float(unit(v, "opacity")?));
        }
        if let Some(b) = &self.blend_mode {
            paint.set("blendMode", Val::Enum(intern(b)));
        }
        if let Some(hex) = &self.hex {
            if kind != "SOLID" {
                return wrong("hex");
            }
            let mut c = color_val(hex)?;
            let alpha = c.get("a").and_then(Val::as_f32).map_or(1.0, f64::from);
            c.set("a", Val::Float(1.0));
            paint.set("color", c);
            if hex.trim().trim_start_matches('#').len() == 8 {
                paint.set("opacity", Val::Float(alpha));
            }
        }
        if let Some(stops) = &self.stops {
            if !gradient {
                return wrong("stops");
            }
            paint.set("stops", stops_val(stops)?);
        }
        if let Some([from, to]) = self.handles {
            if !gradient {
                return wrong("handles");
            }
            paint.set("transform", gradient_transform(from, to)?);
        }
        let image = kind == "IMAGE";
        if let Some(s) = self.scale {
            if !image {
                return wrong("scale");
            }
            paint.set("imageScaleMode", Val::Enum(intern(s.stored())));
        }
        if let Some(r) = self.rotation {
            if !image {
                return wrong("rotation");
            }
            paint.set("rotation", Val::Float(finite(r, "rotation")?));
        }
        if let Some(f) = &self.filters {
            if !image {
                return wrong("filters");
            }
            paint.set("paintFilter", filters_val(f)?);
        }
        Ok(())
    }
}

/// An effect to put in a layer's effect list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EffectSpec {
    DropShadow {
        /// `#RRGGBBAA` (default `#00000040`).
        #[serde(default)]
        hex: Option<String>,
        #[serde(default)]
        offset: Option<[f64; 2]>,
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        spread: Option<f64>,
        #[serde(default)]
        show_behind: Option<bool>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        blend_mode: Option<String>,
    },
    InnerShadow {
        #[serde(default)]
        hex: Option<String>,
        #[serde(default)]
        offset: Option<[f64; 2]>,
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        spread: Option<f64>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        blend_mode: Option<String>,
    },
    LayerBlur {
        radius: f64,
        #[serde(default)]
        visible: Option<bool>,
    },
    BackgroundBlur {
        radius: f64,
        #[serde(default)]
        visible: Option<bool>,
    },
}

impl EffectSpec {
    /// The `Effect` value.
    pub fn to_val(&self) -> Result<Val, FigError> {
        let mut o = Obj::new();
        let shadow = |o: &mut Obj,
                      kind: &str,
                      hex: &Option<String>,
                      offset: &Option<[f64; 2]>,
                      radius: &Option<f64>,
                      spread: &Option<f64>,
                      visible: &Option<bool>,
                      blend: &Option<String>|
         -> Result<(), FigError> {
            o.insert("type", Val::Enum(intern(kind)));
            o.insert("color", color_val(hex.as_deref().unwrap_or("#00000040"))?);
            let [x, y] = offset.unwrap_or([0.0, 4.0]);
            o.insert(
                "offset",
                Val::Obj(Obj::from([
                    ("x", Val::Float(finite(x, "offset")?)),
                    ("y", Val::Float(finite(y, "offset")?)),
                ])),
            );
            o.insert(
                "radius",
                Val::Float(finite(radius.unwrap_or(4.0), "radius")?.max(0.0)),
            );
            o.insert(
                "spread",
                Val::Float(finite(spread.unwrap_or(0.0), "spread")?),
            );
            o.insert("visible", Val::Bool(visible.unwrap_or(true)));
            o.insert(
                "blendMode",
                Val::Enum(intern(blend.as_deref().unwrap_or("NORMAL"))),
            );
            Ok(())
        };
        match self {
            EffectSpec::DropShadow {
                hex,
                offset,
                radius,
                spread,
                show_behind,
                visible,
                blend_mode,
            } => {
                shadow(
                    &mut o,
                    "DROP_SHADOW",
                    hex,
                    offset,
                    radius,
                    spread,
                    visible,
                    blend_mode,
                )?;
                o.insert(
                    "showShadowBehindNode",
                    Val::Bool(show_behind.unwrap_or(false)),
                );
            }
            EffectSpec::InnerShadow {
                hex,
                offset,
                radius,
                spread,
                visible,
                blend_mode,
            } => shadow(
                &mut o,
                "INNER_SHADOW",
                hex,
                offset,
                radius,
                spread,
                visible,
                blend_mode,
            )?,
            EffectSpec::LayerBlur { radius, visible }
            | EffectSpec::BackgroundBlur { radius, visible } => {
                let kind = if matches!(self, EffectSpec::LayerBlur { .. }) {
                    "FOREGROUND_BLUR"
                } else {
                    "BACKGROUND_BLUR"
                };
                o.insert("type", Val::Enum(intern(kind)));
                o.insert("radius", Val::Float(finite(*radius, "radius")?.max(0.0)));
                o.insert("visible", Val::Bool(visible.unwrap_or(true)));
            }
        }
        Ok(Val::Obj(o))
    }
}

/// Changes to one existing effect; unset fields stay as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectPatch {
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub hex: Option<String>,
    #[serde(default)]
    pub offset: Option<[f64; 2]>,
    #[serde(default)]
    pub radius: Option<f64>,
    #[serde(default)]
    pub spread: Option<f64>,
    #[serde(default)]
    pub show_behind: Option<bool>,
    #[serde(default)]
    pub blend_mode: Option<String>,
}

impl EffectPatch {
    /// Applies the patch to `effect` in place.
    pub fn apply(&self, effect: &mut Val) -> Result<(), FigError> {
        let kind = effect.text("type").unwrap_or("DROP_SHADOW").to_string();
        let shadow = kind.ends_with("_SHADOW");
        let wrong = |what: &str| {
            Err(FigError::malformed(format!(
                "{what} does not apply to a {kind} effect"
            )))
        };
        if let Some(v) = self.visible {
            effect.set("visible", Val::Bool(v));
        }
        if let Some(r) = self.radius {
            effect.set("radius", Val::Float(finite(r, "radius")?.max(0.0)));
        }
        for (set, what) in [
            (self.hex.is_some(), "hex"),
            (self.offset.is_some(), "offset"),
            (self.spread.is_some(), "spread"),
            (self.blend_mode.is_some(), "blend_mode"),
        ] {
            if set && !shadow {
                return wrong(what);
            }
        }
        if self.show_behind.is_some() && kind != "DROP_SHADOW" {
            return wrong("show_behind");
        }
        if let Some(hex) = &self.hex {
            effect.set("color", color_val(hex)?);
        }
        if let Some([x, y]) = self.offset {
            effect.set(
                "offset",
                Val::Obj(Obj::from([
                    ("x", Val::Float(finite(x, "offset")?)),
                    ("y", Val::Float(finite(y, "offset")?)),
                ])),
            );
        }
        if let Some(s) = self.spread {
            effect.set("spread", Val::Float(finite(s, "spread")?));
        }
        if let Some(b) = self.show_behind {
            effect.set("showShadowBehindNode", Val::Bool(b));
        }
        if let Some(b) = &self.blend_mode {
            effect.set("blendMode", Val::Enum(intern(b)));
        }
        Ok(())
    }
}

/// A text length: a line height or letter spacing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextLength {
    /// Figma's "Auto" line height (stored as 100%).
    Auto,
    Px(f64),
    Percent(f64),
}

impl TextLength {
    /// The `Number` value (`{ value, units }`).
    pub fn to_val(self) -> Result<Val, FigError> {
        let (value, units) = match self {
            TextLength::Auto => (100.0, "PERCENT"),
            TextLength::Px(v) => (finite(v, "length")?, "PIXELS"),
            TextLength::Percent(v) => (finite(v, "length")?, "PERCENT"),
        };
        Ok(Val::Obj(Obj::from([
            ("value", Val::Float(value)),
            ("units", Val::Enum(intern(units))),
        ])))
    }
}

/// Whole-node text settings beyond font and size (`set_typography`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TypographyExtra {
    #[serde(default)]
    pub line_height: Option<TextLength>,
    #[serde(default)]
    pub letter_spacing: Option<TextLength>,
    #[serde(default)]
    pub paragraph_spacing: Option<f64>,
    #[serde(default)]
    pub paragraph_indent: Option<f64>,
    /// `TOP`, `CENTER` or `BOTTOM`.
    #[serde(default)]
    pub align_vertical: Option<String>,
    /// `NONE`, `UNDERLINE` or `STRIKETHROUGH`.
    #[serde(default)]
    pub decoration: Option<String>,
    /// `ORIGINAL`, `UPPER`, `LOWER`, `TITLE`, `SMALL_CAPS`, `SMALL_CAPS_FORCED`.
    #[serde(default)]
    pub case: Option<String>,
    /// End overflowing text with an ellipsis.
    #[serde(default)]
    pub truncate: Option<bool>,
    /// With `truncate`: the most lines shown.
    #[serde(default)]
    pub max_lines: Option<u32>,
}

impl TypographyExtra {
    /// The node fields to set, as `(field, value)`.
    pub fn fields(&self) -> Result<Vec<(&'static str, Val)>, FigError> {
        let mut out = Vec::new();
        let upper = |s: &str| Val::Enum(intern(&s.to_ascii_uppercase()));
        if let Some(l) = self.line_height {
            out.push(("lineHeight", l.to_val()?));
        }
        if let Some(l) = self.letter_spacing {
            out.push(("letterSpacing", l.to_val()?));
        }
        if let Some(v) = self.paragraph_spacing {
            out.push((
                "paragraphSpacing",
                Val::Float(finite(v, "paragraph spacing")?.max(0.0)),
            ));
        }
        if let Some(v) = self.paragraph_indent {
            out.push((
                "paragraphIndent",
                Val::Float(finite(v, "paragraph indent")?.max(0.0)),
            ));
        }
        if let Some(v) = &self.align_vertical {
            out.push(("textAlignVertical", upper(v)));
        }
        if let Some(v) = &self.decoration {
            out.push(("textDecoration", upper(v)));
        }
        if let Some(v) = &self.case {
            out.push(("textCase", upper(v)));
        }
        if let Some(t) = self.truncate {
            out.push((
                "textTruncation",
                Val::Enum(intern(if t { "ENDING" } else { "DISABLED" })),
            ));
        }
        if let Some(n) = self.max_lines {
            out.push(("maxLines", Val::Int(i64::from(n.max(1)))));
        }
        Ok(out)
    }
}
