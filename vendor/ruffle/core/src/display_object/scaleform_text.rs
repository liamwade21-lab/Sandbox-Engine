//! Sandbox Engine patch b (vendor/ruffle/UPSTREAM.md): Scaleform GFx 4's text field extensions
//! (`scaleform.gfx.TextFieldEx`), written from Scaleform's documented behaviour.
//!
//! - no-translate: the field's text is shown as set, never passed to the UI backend's translator;
//! - vertical alignment: the laid-out text sits at the top, middle or bottom of the field's box
//!   (`none` is the top, as Flash);
//! - text auto-size: `shrink` lowers every span's font size until the text fits the box (never
//!   above the authored size), `fit` scales it up or down until it just fills the box. Only for
//!   fields whose Flash `autoSize` is none: an auto-sized field's box follows its text instead.

use crate::html::{FormatSpans, Layout, TextFormat};
use swf::Twips;

/// A field's vertical alignment (`TextFieldEx.setVerticalAlign`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerticalAlign {
    #[default]
    None,
    Top,
    Center,
    Bottom,
}

impl VerticalAlign {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "top" => Some(Self::Top),
            "center" => Some(Self::Center),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Top => "top",
            Self::Center => "center",
            Self::Bottom => "bottom",
        }
    }

    /// How far down the text is moved in a box `free` taller than the text (no move when the
    /// text is the taller one).
    pub fn offset(self, free: Twips) -> Twips {
        let free = free.max(Twips::ZERO);
        match self {
            Self::None | Self::Top => Twips::ZERO,
            Self::Center => free / 2,
            Self::Bottom => free,
        }
    }
}

/// A field's text auto-size mode (`TextFieldEx.setTextAutoSize`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAutoSize {
    #[default]
    None,
    Shrink,
    Fit,
}

impl TextAutoSize {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "shrink" => Some(Self::Shrink),
            "fit" => Some(Self::Fit),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Shrink => "shrink",
            Self::Fit => "fit",
        }
    }
}

/// The Scaleform settings of one text field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScaleformText {
    pub no_translate: bool,
    pub vertical_align: VerticalAlign,
    pub auto_size: TextAutoSize,
}

/// Steps of the search for the font scale: the scale is found to 1/2^STEPS of its range.
const STEPS: u32 = 12;

/// The largest scale `fit` tries (a field filled by text 64 times its authored size).
const MOST: f64 = 64.0;

/// Lays out `spans` at the font scale `mode` asks for in a box of `width` x `height` (layout
/// units, the gutter already taken off). `lay` lays out a set of spans.
pub fn auto_size<'gc>(
    mode: TextAutoSize,
    spans: &FormatSpans,
    width: Twips,
    height: Twips,
    mut lay: impl FnMut(&FormatSpans) -> Layout<'gc>,
) -> Layout<'gc> {
    let fits = |layout: &Layout<'gc>| {
        let size = layout.text_size();
        size.width() <= width && size.height() <= height
    };
    let authored = lay(spans);
    let (mut low, mut high) = match mode {
        TextAutoSize::None => return authored,
        TextAutoSize::Shrink if fits(&authored) => return authored,
        TextAutoSize::Shrink => (0.0, 1.0),
        TextAutoSize::Fit if fits(&authored) => (1.0, MOST),
        TextAutoSize::Fit => (0.0, 1.0),
    };
    let mut best = (low == 1.0).then_some(authored);
    for _ in 0..STEPS {
        let scale = (low + high) / 2.0;
        let layout = lay(&scaled(spans, scale));
        if fits(&layout) {
            low = scale;
            best = Some(layout);
        } else {
            high = scale;
        }
    }
    // Nothing fitted: the smallest scale tried.
    best.unwrap_or_else(|| lay(&scaled(spans, high)))
}

/// `spans` with every font size multiplied by `scale`.
fn scaled(spans: &FormatSpans, scale: f64) -> FormatSpans {
    let mut out = spans.clone();
    for (start, end, _, span) in spans.iter_spans() {
        let format = TextFormat {
            size: Some(span.font.size * scale),
            ..Default::default()
        };
        out.set_text_format(start, end, &format);
    }
    out
}
