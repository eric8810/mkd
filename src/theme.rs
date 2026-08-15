use gpui::{Hsla, rgb};

/// `rgb()` yields `Rgba`; style fields want `Hsla`.
pub fn hsl(c: u32) -> Hsla {
    rgb(c).into()
}

/// Light, GitHub-docs-inspired palette.
pub struct Theme {
    pub bg: Hsla,
    pub fg: Hsla,
    pub heading: Hsla,
    pub muted: Hsla,
    pub code_bg: Hsla,
    pub code_fg: Hsla,
    pub quote_border: Hsla,
    pub rule: Hsla,

    // Containers
    pub tip_accent: Hsla,
    pub tip_bg: Hsla,
    pub warn_accent: Hsla,
    pub warn_bg: Hsla,
    pub danger_accent: Hsla,
    pub danger_bg: Hsla,
    pub info_accent: Hsla,
    pub info_bg: Hsla,
    pub neutral_accent: Hsla,
    pub neutral_bg: Hsla,

    // Code
    pub line_highlight: Hsla,
}

impl Theme {
    pub fn light() -> Self {
        Theme {
            bg: hsl(0xffffff),
            fg: hsl(0x24292f),
            heading: hsl(0x1f2328),
            muted: hsl(0x59636e),
            code_bg: hsl(0xf6f8fa),
            code_fg: hsl(0x1f2328),
            quote_border: hsl(0xd1d9e0),
            rule: hsl(0xd1d9e0),

            tip_accent: hsl(0x1a7f37),
            tip_bg: hsl(0xf0fff4),
            warn_accent: hsl(0x9a6700),
            warn_bg: hsl(0xfffbeb),
            danger_accent: hsl(0xd1242f),
            danger_bg: hsl(0xffeff0),
            info_accent: hsl(0x0969da),
            info_bg: hsl(0xf3f7ff),
            neutral_accent: hsl(0x57606a),
            neutral_bg: hsl(0xf6f8fa),

            line_highlight: hsl(0xfff7cc),
        }
    }
}
