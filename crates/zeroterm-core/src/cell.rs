//! Cell model - a single terminal cell with character and attributes

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const DEFAULT_FG: Color = Color {
        r: 0xe0,
        g: 0xe0,
        b: 0xe0,
    };
    pub const DEFAULT_BG: Color = Color {
        r: 0x1e,
        g: 0x1e,
        b: 0x1e,
    };
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const RED: Color = Color {
        r: 0xff,
        g: 0x00,
        b: 0x00,
    };
    pub const GREEN: Color = Color {
        r: 0x00,
        g: 0xff,
        b: 0x00,
    };
    pub const YELLOW: Color = Color {
        r: 0xff,
        g: 0xff,
        b: 0x00,
    };
    pub const BLUE: Color = Color {
        r: 0x00,
        g: 0x00,
        b: 0xff,
    };
    pub const MAGENTA: Color = Color {
        r: 0xff,
        g: 0x00,
        b: 0xff,
    };
    pub const CYAN: Color = Color {
        r: 0x00,
        g: 0xff,
        b: 0xff,
    };
    pub const WHITE: Color = Color {
        r: 0xff,
        g: 0xff,
        b: 0xff,
    };

    pub fn from_ansi_256(idx: u8) -> Color {
        match idx {
            0..=15 => Self::from_ansi_16(idx),
            16..=231 => Self::from_ansi_216(idx),
            232..=255 => Self::from_ansi_gray(idx),
        }
    }

    pub fn from_ansi_16(idx: u8) -> Color {
        const ANSI_16: [Color; 16] = [
            Color::BLACK,
            Color {
                r: 0x80,
                g: 0x00,
                b: 0x00,
            },
            Color {
                r: 0x00,
                g: 0x80,
                b: 0x00,
            },
            Color {
                r: 0x80,
                g: 0x80,
                b: 0x00,
            },
            Color {
                r: 0x00,
                g: 0x00,
                b: 0x80,
            },
            Color {
                r: 0x80,
                g: 0x00,
                b: 0x80,
            },
            Color {
                r: 0x00,
                g: 0x80,
                b: 0x80,
            },
            Color {
                r: 0xc0,
                g: 0xc0,
                b: 0xc0,
            },
            Color {
                r: 0x80,
                g: 0x80,
                b: 0x80,
            },
            Color::RED,
            Color::GREEN,
            Color::YELLOW,
            Color::BLUE,
            Color::MAGENTA,
            Color::CYAN,
            Color::WHITE,
        ];
        ANSI_16[idx as usize]
    }

    pub fn from_ansi_216(idx: u8) -> Color {
        let idx = idx - 16;
        let r = idx / 36;
        let g = (idx % 36) / 6;
        let b = idx % 6;
        let map = |v: u8| if v == 0 { 0 } else { v * 40 + 55 };
        Color {
            r: map(r),
            g: map(g),
            b: map(b),
        }
    }

    pub fn from_ansi_gray(idx: u8) -> Color {
        let v = (idx - 232) * 10 + 8;
        Color { r: v, g: v, b: v }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnderlineStyle {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attributes {
    pub bold: bool,
    pub italic: bool,
    pub underline: UnderlineStyle,
    pub strikethrough: bool,
    pub dim: bool,
    pub blink: bool,
    pub reverse: bool,
    pub invisible: bool,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: UnderlineStyle::None,
            strikethrough: false,
            dim: false,
            blink: false,
            reverse: false,
            invisible: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attributes,
    /// Highlight class index (0 = none/auto, see `highlight` module).
    #[serde(default)]
    pub syntax_color: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::DEFAULT_FG,
            bg: Color::DEFAULT_BG,
            attrs: Attributes::default(),
            syntax_color: 0,
        }
    }
}

impl Cell {
    pub fn new(ch: char) -> Self {
        Self {
            ch,
            ..Default::default()
        }
    }

    pub fn width(&self) -> usize {
        self.ch.width().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.ch == ' '
            && self.fg == Color::DEFAULT_FG
            && self.bg == Color::DEFAULT_BG
            && self.attrs == Attributes::default()
            && self.syntax_color == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
    pub shape: CursorShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            visible: true,
            shape: CursorShape::Block,
        }
    }
}
