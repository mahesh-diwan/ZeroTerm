//! Curated terminal color themes.

use zeroterm_core::cell::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub fg: Color,
    pub surface: Color,
    pub surface_highlight: Color,
    pub border: Color,
    pub accent: Color,
    pub selection_bg: Color,
    pub ansi: [Color; 16],
}

impl Theme {
    pub const fn tokyo_night() -> Self {
        Self {
            name: "tokyo-night",
            bg: Color::rgb(0x1a, 0x1b, 0x26),
            fg: Color::rgb(0xc0, 0xca, 0xf5),
            surface: Color::rgb(0x16, 0x16, 0x1e),
            surface_highlight: Color::rgb(0x1f, 0x23, 0x35),
            border: Color::rgb(0x24, 0x28, 0x3b),
            accent: Color::rgb(0x7a, 0xa2, 0xf7),
            selection_bg: Color::rgb(0x28, 0x34, 0x57),
            ansi: [
                Color::rgb(0x15, 0x16, 0x1e),
                Color::rgb(0xf7, 0x76, 0x8e),
                Color::rgb(0x9e, 0xce, 0x6a),
                Color::rgb(0xe0, 0xaf, 0x68),
                Color::rgb(0x7a, 0xa2, 0xf7),
                Color::rgb(0xbb, 0x9a, 0xf7),
                Color::rgb(0x7d, 0xcf, 0xff),
                Color::rgb(0xa9, 0xb1, 0xd6),
                Color::rgb(0x41, 0x48, 0x68),
                Color::rgb(0xf7, 0x76, 0x8e),
                Color::rgb(0x9e, 0xce, 0x6a),
                Color::rgb(0xe0, 0xaf, 0x68),
                Color::rgb(0x7a, 0xa2, 0xf7),
                Color::rgb(0xbb, 0x9a, 0xf7),
                Color::rgb(0x7d, 0xcf, 0xff),
                Color::rgb(0xc0, 0xca, 0xf5),
            ],
        }
    }

    pub const fn catppuccin_mocha() -> Self {
        Self {
            name: "catppuccin-mocha",
            bg: Color::rgb(0x1e, 0x1e, 0x2e),
            fg: Color::rgb(0xcd, 0xd6, 0xf4),
            surface: Color::rgb(0x18, 0x18, 0x25),
            surface_highlight: Color::rgb(0x31, 0x32, 0x44),
            border: Color::rgb(0x45, 0x47, 0x5a),
            accent: Color::rgb(0x89, 0xb4, 0xfa),
            selection_bg: Color::rgb(0x45, 0x47, 0x5a),
            ansi: [
                Color::rgb(0x45, 0x47, 0x5a),
                Color::rgb(0xf3, 0x8b, 0xa8),
                Color::rgb(0xa6, 0xe3, 0xa1),
                Color::rgb(0xf9, 0xe2, 0xaf),
                Color::rgb(0x89, 0xb4, 0xfa),
                Color::rgb(0xc6, 0xa0, 0xf6),
                Color::rgb(0x94, 0xe2, 0xd5),
                Color::rgb(0xba, 0xc2, 0xde),
                Color::rgb(0x58, 0x5b, 0x70),
                Color::rgb(0xf3, 0x8b, 0xa8),
                Color::rgb(0xa6, 0xe3, 0xa1),
                Color::rgb(0xf9, 0xe2, 0xaf),
                Color::rgb(0x89, 0xb4, 0xfa),
                Color::rgb(0xc6, 0xa0, 0xf6),
                Color::rgb(0x94, 0xe2, 0xd5),
                Color::rgb(0xa6, 0xad, 0xc8),
            ],
        }
    }

    pub const fn dracula() -> Self {
        Self {
            name: "dracula",
            bg: Color::rgb(0x28, 0x2a, 0x36),
            fg: Color::rgb(0xf8, 0xf8, 0xf2),
            surface: Color::rgb(0x21, 0x22, 0x2c),
            surface_highlight: Color::rgb(0x34, 0x35, 0x40),
            border: Color::rgb(0x44, 0x44, 0x4a),
            accent: Color::rgb(0xbd, 0x93, 0xf9),
            selection_bg: Color::rgb(0x44, 0x44, 0x4a),
            ansi: [
                Color::rgb(0x00, 0x00, 0x00),
                Color::rgb(0xff, 0x55, 0x55),
                Color::rgb(0x50, 0xfa, 0x7b),
                Color::rgb(0xf1, 0xfa, 0x8c),
                Color::rgb(0xbd, 0x93, 0xf9),
                Color::rgb(0xff, 0x79, 0xc6),
                Color::rgb(0x8b, 0xe9, 0xfd),
                Color::rgb(0xbb, 0xbb, 0xbb),
                Color::rgb(0x55, 0x55, 0x55),
                Color::rgb(0xff, 0x55, 0x55),
                Color::rgb(0x50, 0xfa, 0x7b),
                Color::rgb(0xf1, 0xfa, 0x8c),
                Color::rgb(0xbd, 0x93, 0xf9),
                Color::rgb(0xff, 0x79, 0xc6),
                Color::rgb(0x8b, 0xe9, 0xfd),
                Color::rgb(0xff, 0xff, 0xff),
            ],
        }
    }

    pub const fn gruvbox_dark() -> Self {
        Self {
            name: "gruvbox-dark",
            bg: Color::rgb(0x28, 0x28, 0x28),
            fg: Color::rgb(0xeb, 0xdb, 0xb2),
            surface: Color::rgb(0x1d, 0x20, 0x21),
            surface_highlight: Color::rgb(0x32, 0x30, 0x2f),
            border: Color::rgb(0x50, 0x49, 0x45),
            accent: Color::rgb(0xfa, 0xbd, 0x2f),
            selection_bg: Color::rgb(0x50, 0x49, 0x45),
            ansi: [
                Color::rgb(0x1d, 0x20, 0x21),
                Color::rgb(0xfb, 0x49, 0x34),
                Color::rgb(0xb8, 0xbb, 0x26),
                Color::rgb(0xfa, 0xbd, 0x2f),
                Color::rgb(0x83, 0xa5, 0x98),
                Color::rgb(0xd3, 0x86, 0x9b),
                Color::rgb(0x8e, 0xc0, 0x7c),
                Color::rgb(0xd5, 0xc4, 0xa1),
                Color::rgb(0x92, 0x83, 0x74),
                Color::rgb(0xfb, 0x49, 0x34),
                Color::rgb(0xb8, 0xbb, 0x26),
                Color::rgb(0xfa, 0xbd, 0x2f),
                Color::rgb(0x83, 0xa5, 0x98),
                Color::rgb(0xd3, 0x86, 0x9b),
                Color::rgb(0x8e, 0xc0, 0x7c),
                Color::rgb(0xfb, 0xf1, 0xc7),
            ],
        }
    }

    pub const fn nord() -> Self {
        Self {
            name: "nord",
            bg: Color::rgb(0x2e, 0x34, 0x40),
            fg: Color::rgb(0xd8, 0xde, 0xe9),
            surface: Color::rgb(0x27, 0x2c, 0x36),
            surface_highlight: Color::rgb(0x3b, 0x42, 0x52),
            border: Color::rgb(0x43, 0x48, 0x54),
            accent: Color::rgb(0x88, 0xc0, 0xd0),
            selection_bg: Color::rgb(0x43, 0x48, 0x54),
            ansi: [
                Color::rgb(0x3b, 0x42, 0x52),
                Color::rgb(0xbf, 0x61, 0x6a),
                Color::rgb(0xa3, 0xbe, 0x8c),
                Color::rgb(0xeb, 0xcb, 0x8b),
                Color::rgb(0x81, 0xa1, 0xc1),
                Color::rgb(0xb4, 0x8e, 0xad),
                Color::rgb(0x88, 0xc0, 0xd0),
                Color::rgb(0xe5, 0xe9, 0xf0),
                Color::rgb(0x4c, 0x56, 0x6a),
                Color::rgb(0xbf, 0x61, 0x6a),
                Color::rgb(0xa3, 0xbe, 0x8c),
                Color::rgb(0xeb, 0xcb, 0x8b),
                Color::rgb(0x81, 0xa1, 0xc1),
                Color::rgb(0xb4, 0x8e, 0xad),
                Color::rgb(0x8f, 0xbc, 0xbb),
                Color::rgb(0xec, 0xef, 0xf4),
            ],
        }
    }

    pub const fn rose_pine() -> Self {
        Self {
            name: "rose-pine",
            bg: Color::rgb(0x19, 0x17, 0x24),
            fg: Color::rgb(0xe0, 0xde, 0xf4),
            surface: Color::rgb(0x1f, 0x1d, 0x2e),
            surface_highlight: Color::rgb(0x2a, 0x27, 0x3e),
            border: Color::rgb(0x55, 0x50, 0x80),
            accent: Color::rgb(0xc4, 0xa7, 0xe7),
            selection_bg: Color::rgb(0x55, 0x50, 0x80),
            ansi: [
                Color::rgb(0x26, 0x23, 0x3a),
                Color::rgb(0xeb, 0x6f, 0x92),
                Color::rgb(0x31, 0x74, 0x8f),
                Color::rgb(0xf6, 0xc1, 0x77),
                Color::rgb(0x9c, 0xce, 0xfd),
                Color::rgb(0xc4, 0xa7, 0xe7),
                Color::rgb(0x9c, 0xce, 0xfd),
                Color::rgb(0xe0, 0xde, 0xf4),
                Color::rgb(0x6e, 0x6a, 0x86),
                Color::rgb(0xeb, 0x6f, 0x92),
                Color::rgb(0x31, 0x74, 0x8f),
                Color::rgb(0xf6, 0xc1, 0x77),
                Color::rgb(0x9c, 0xce, 0xfd),
                Color::rgb(0xc4, 0xa7, 0xe7),
                Color::rgb(0x9c, 0xce, 0xfd),
                Color::rgb(0xff, 0xfb, 0xf4),
            ],
        }
    }

    pub const fn one_dark() -> Self {
        Self {
            name: "one-dark",
            bg: Color::rgb(0x28, 0x2c, 0x34),
            fg: Color::rgb(0xab, 0xb2, 0xbf),
            surface: Color::rgb(0x21, 0x25, 0x2b),
            surface_highlight: Color::rgb(0x2c, 0x31, 0x3c),
            border: Color::rgb(0x42, 0x48, 0x53),
            accent: Color::rgb(0x61, 0xaf, 0xef),
            selection_bg: Color::rgb(0x42, 0x48, 0x53),
            ansi: [
                Color::rgb(0x28, 0x2c, 0x34),
                Color::rgb(0xe0, 0x6c, 0x75),
                Color::rgb(0x98, 0xc3, 0x79),
                Color::rgb(0xe5, 0xc0, 0x7b),
                Color::rgb(0x61, 0xaf, 0xef),
                Color::rgb(0xc6, 0x78, 0xdd),
                Color::rgb(0x56, 0xb6, 0xc2),
                Color::rgb(0xab, 0xb2, 0xbf),
                Color::rgb(0x5c, 0x63, 0x70),
                Color::rgb(0xe0, 0x6c, 0x75),
                Color::rgb(0x98, 0xc3, 0x79),
                Color::rgb(0xe5, 0xc0, 0x7b),
                Color::rgb(0x61, 0xaf, 0xef),
                Color::rgb(0xc6, 0x78, 0xdd),
                Color::rgb(0x56, 0xb6, 0xc2),
                Color::rgb(0xff, 0xff, 0xff),
            ],
        }
    }

    pub const fn solarized_dark() -> Self {
        Self {
            name: "solarized-dark",
            bg: Color::rgb(0x00, 0x2b, 0x36),
            fg: Color::rgb(0x83, 0x94, 0x96),
            surface: Color::rgb(0x07, 0x36, 0x42),
            surface_highlight: Color::rgb(0x00, 0x40, 0x4f),
            border: Color::rgb(0x58, 0x6e, 0x75),
            accent: Color::rgb(0x26, 0x8b, 0xd2),
            selection_bg: Color::rgb(0x58, 0x6e, 0x75),
            ansi: [
                Color::rgb(0x07, 0x36, 0x42),
                Color::rgb(0xdc, 0x32, 0x2f),
                Color::rgb(0x85, 0x99, 0x00),
                Color::rgb(0xb5, 0x89, 0x00),
                Color::rgb(0x26, 0x8b, 0xd2),
                Color::rgb(0xd3, 0x36, 0x82),
                Color::rgb(0x2a, 0xa1, 0x98),
                Color::rgb(0xee, 0xe8, 0xd5),
                Color::rgb(0x00, 0x2b, 0x36),
                Color::rgb(0xcb, 0x4b, 0x16),
                Color::rgb(0x58, 0x6e, 0x75),
                Color::rgb(0x65, 0x7b, 0x83),
                Color::rgb(0x83, 0x94, 0x96),
                Color::rgb(0x6c, 0x71, 0xc4),
                Color::rgb(0x93, 0xa1, 0xa1),
                Color::rgb(0xfd, 0xf6, 0xe3),
            ],
        }
    }

    pub fn by_name(name: &str) -> Self {
        match name {
            "catppuccin" | "catppuccin-mocha" => Self::catppuccin_mocha(),
            "dracula" => Self::dracula(),
            "gruvbox" | "gruvbox-dark" => Self::gruvbox_dark(),
            "nord" => Self::nord(),
            "rose-pine" | "rosepine" => Self::rose_pine(),
            "one-dark" | "onedark" => Self::one_dark(),
            "solarized" | "solarized-dark" => Self::solarized_dark(),
            _ => Self::tokyo_night(),
        }
    }

    /// Remap a screen cell color to the theme: default fg/bg and the 16 ANSI
    /// palette entries are substituted. Everything else (256-color, truecolor,
    /// syntax highlight) passes through untouched.
    pub fn map_cell_color(&self, c: Color) -> Color {
        const DEFAULT_FG: Color = Color::rgb(0xe0, 0xe0, 0xe0);
        const DEFAULT_BG: Color = Color::rgb(0x1e, 0x1e, 0x1e);
        const FALLBACK_ANSI: [Color; 16] = [
            Color::rgb(0x00, 0x00, 0x00),
            Color::rgb(0x80, 0x00, 0x00),
            Color::rgb(0x00, 0x80, 0x00),
            Color::rgb(0x80, 0x80, 0x00),
            Color::rgb(0x00, 0x00, 0x80),
            Color::rgb(0x80, 0x00, 0x80),
            Color::rgb(0x00, 0x80, 0x80),
            Color::rgb(0xc0, 0xc0, 0xc0),
            Color::rgb(0x80, 0x80, 0x80),
            Color::rgb(0xff, 0x00, 0x00),
            Color::rgb(0x00, 0xff, 0x00),
            Color::rgb(0xff, 0xff, 0x00),
            Color::rgb(0x00, 0x00, 0xff),
            Color::rgb(0xff, 0x00, 0xff),
            Color::rgb(0x00, 0xff, 0xff),
            Color::rgb(0xff, 0xff, 0xff),
        ];
        if c == DEFAULT_FG {
            return self.fg;
        }
        if c == DEFAULT_BG {
            return self.bg;
        }
        for (i, base) in FALLBACK_ANSI.iter().enumerate() {
            if c == *base {
                return self.ansi[i];
            }
        }
        c
    }
}
