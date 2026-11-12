// 颜色与日志角色配色：与旧版 theme.rs 保持同一套色板。
pub type Rgb = (u8, u8, u8);

pub struct Palette {
    pub canvas: Rgb,
    pub sidebar: Rgb,
    pub surface: Rgb,
    pub surface2: Rgb,
    pub border: Rgb,
    pub border_strong: Rgb,
    pub text: Rgb,
    pub secondary: Rgb,
    pub muted: Rgb,
    pub faint: Rgb,
    pub ok: Rgb,
    pub warn: Rgb,
    pub danger: Rgb,
    pub console: Rgb,
    pub console_raised: Rgb,
    pub console_border: Rgb,
    pub console_text: Rgb,
    pub console_muted: Rgb,
}

pub const DARK: Palette = Palette {
    canvas: (0x0B, 0x0D, 0x12),
    sidebar: (0x0E, 0x10, 0x16),
    surface: (0x15, 0x18, 0x21),
    surface2: (0x1B, 0x1F, 0x29),
    border: (0x23, 0x28, 0x33),
    border_strong: (0x31, 0x38, 0x48),
    text: (0xE7, 0xEA, 0xF0),
    secondary: (0xB7, 0xBE, 0xCC),
    muted: (0x8A, 0x93, 0xA3),
    faint: (0x5F, 0x68, 0x78),
    ok: (0x34, 0xD3, 0x99),
    warn: (0xFB, 0xBF, 0x24),
    danger: (0xF8, 0x71, 0x71),
    console: (0x0A, 0x0C, 0x10),
    console_raised: (0x12, 0x15, 0x1B),
    console_border: (0x23, 0x28, 0x33),
    console_text: (0xD7, 0xDB, 0xE2),
    console_muted: (0x7C, 0x85, 0x92),
};

pub const LIGHT: Palette = Palette {
    canvas: (0xF4, 0xF5, 0xF7),
    sidebar: (0xFF, 0xFF, 0xFF),
    surface: (0xFF, 0xFF, 0xFF),
    surface2: (0xF1, 0xF3, 0xF6),
    border: (0xE2, 0xE5, 0xEA),
    border_strong: (0xCD, 0xD3, 0xDB),
    text: (0x16, 0x18, 0x1D),
    secondary: (0x4B, 0x52, 0x60),
    muted: (0x74, 0x7C, 0x89),
    faint: (0x9A, 0xA1, 0xAC),
    ok: (0x0F, 0x9D, 0x6E),
    warn: (0xB4, 0x53, 0x09),
    danger: (0xD6, 0x45, 0x45),
    console: (0xFF, 0xFF, 0xFF),
    console_raised: (0xF7, 0xF8, 0xFA),
    console_border: (0xE6, 0xE8, 0xEC),
    console_text: (0x1F, 0x23, 0x28),
    console_muted: (0x6B, 0x72, 0x80),
};

const DARK_ACCENTS: [Rgb; 4] = [
    (0x7C, 0x6C, 0xFF),
    (0x4C, 0x8D, 0xFF),
    (0x2D, 0xD4, 0xBF),
    (0xF5, 0xB5, 0x44),
];

const LIGHT_ACCENTS: [Rgb; 4] = [
    (0x5B, 0x4B, 0xE6),
    (0x25, 0x63, 0xEB),
    (0x0D, 0x94, 0x88),
    (0xB4, 0x53, 0x09),
];

pub struct LogPalette {
    pub text: Rgb,
    pub muted: Rgb,
    pub tx: Rgb,
    pub hex: Rgb,
    pub highlight: Rgb,
}

pub const LOG_DARK: LogPalette = LogPalette {
    text: (0xD5, 0xDA, 0xE2),
    muted: (0x7E, 0x87, 0x96),
    tx: (0x6E, 0xA8, 0xFF),
    hex: (0x5F, 0xD0, 0xC5),
    highlight: (0xE3, 0xB3, 0x41),
};

pub const LOG_LIGHT: LogPalette = LogPalette {
    text: (0x24, 0x29, 0x2F),
    muted: (0x8A, 0x91, 0x9C),
    tx: (0x25, 0x63, 0xEB),
    hex: (0x0F, 0x76, 0x6E),
    highlight: (0xB4, 0x53, 0x09),
};

pub struct TermPalette {
    pub ansi: [Rgb; 16],
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub cursor_dim: Rgb,
}

pub const TERM_DARK: TermPalette = TermPalette {
    ansi: [
        (0x48, 0x4F, 0x58),
        (0xF8, 0x51, 0x49),
        (0x3F, 0xB9, 0x50),
        (0xD2, 0x99, 0x22),
        (0x58, 0xA6, 0xFF),
        (0xBC, 0x8C, 0xFF),
        (0x39, 0xC5, 0xCF),
        (0xC9, 0xD1, 0xD9),
        (0x70, 0x77, 0x80),
        (0xFF, 0x79, 0x71),
        (0x67, 0xE1, 0x78),
        (0xFA, 0xC1, 0x4A),
        (0x80, 0xCE, 0xFF),
        (0xE4, 0xB4, 0xFF),
        (0x61, 0xED, 0xF7),
        (0xF1, 0xF9, 0xFF),
    ],
    fg: (0xC9, 0xD1, 0xD9),
    bg: (0x0A, 0x0C, 0x10),
    cursor: (0x58, 0xA6, 0xFF),
    cursor_dim: (0x48, 0x4F, 0x58),
};

pub const TERM_LIGHT: TermPalette = TermPalette {
    ansi: [
        (0x3B, 0x40, 0x46),
        (0xC9, 0x33, 0x2B),
        (0x2E, 0x7D, 0x32),
        (0xB3, 0x5B, 0x00),
        (0x1A, 0x6B, 0xD1),
        (0x8B, 0x3F, 0xC4),
        (0x0E, 0x7C, 0x86),
        (0x6B, 0x71, 0x7A),
        (0x8A, 0x91, 0x9C),
        (0xE0, 0x4A, 0x43),
        (0x3E, 0x9A, 0x44),
        (0xD1, 0x7A, 0x00),
        (0x2E, 0x7F, 0xEB),
        (0xA5, 0x5B, 0xD6),
        (0x1F, 0x9A, 0xA5),
        (0x24, 0x29, 0x2F),
    ],
    fg: (0x24, 0x29, 0x2F),
    bg: (0xFF, 0xFF, 0xFF),
    cursor: (0x1A, 0x6B, 0xD1),
    cursor_dim: (0x9A, 0xA1, 0xAC),
};

pub fn log_palette(dark: bool) -> &'static LogPalette {
    if dark {
        &LOG_DARK
    } else {
        &LOG_LIGHT
    }
}

pub fn term_palette(dark: bool) -> &'static TermPalette {
    if dark {
        &TERM_DARK
    } else {
        &TERM_LIGHT
    }
}

pub fn hex(rgb: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// 前端初始化用的主题数据。
pub fn theme_json(cfg: &crate::config::Config) -> serde_json::Value {
    let dark = cfg.dark;
    let p = if dark { &DARK } else { &LIGHT };
    let idx = (cfg.accent.max(0) as usize).min(3);
    let base = if dark { DARK_ACCENTS[idx] } else { LIGHT_ACCENTS[idx] };
    let hover = mix(base, if dark { (255, 255, 255) } else { (0, 0, 0) }, 0.16);
    let soft = mix(base, p.canvas, if dark { 0.84 } else { 0.90 });
    let border = mix(base, p.canvas, if dark { 0.55 } else { 0.62 });
    let text = crate::state::parse_hex_rgb(&cfg.text_color).unwrap_or(p.text);

    serde_json::json!({
        "dark": dark,
        "canvas": hex(p.canvas),
        "sidebar": hex(p.sidebar),
        "surface": hex(p.surface),
        "surface2": hex(p.surface2),
        "border": hex(p.border),
        "borderStrong": hex(p.border_strong),
        "text": hex(text),
        "secondary": hex(p.secondary),
        "muted": hex(p.muted),
        "faint": hex(p.faint),
        "accent": hex(base),
        "accentHover": hex(hover),
        "accentSoft": hex(soft),
        "accentBorder": hex(border),
        "ok": hex(p.ok),
        "warn": hex(p.warn),
        "danger": hex(p.danger),
        "console": hex(p.console),
        "consoleRaised": hex(p.console_raised),
        "consoleBorder": hex(p.console_border),
        "consoleText": hex(p.console_text),
        "consoleMuted": hex(p.console_muted),
        "logText": hex(log_palette(dark).text),
        "logMuted": hex(log_palette(dark).muted),
        "logTx": hex(log_palette(dark).tx),
        "logHex": hex(log_palette(dark).hex),
        "logHighlight": hex(log_palette(dark).highlight),
    })
}
