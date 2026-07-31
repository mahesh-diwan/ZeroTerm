//! Keyboard-accessible settings overlay menu.
//!
//! The menu is drawn into the active pane's parser screen buffer via synthetic
//! CSI sequences (CUP + SGR + text). Because that is destructive to the cells it
//! overwrites, the covered region is snapshotted on open and restored on close.

use zeroterm_core::cell::{Cell, Cursor};
use zeroterm_core::screen::Screen;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub foreground: &'static str,
    pub background: &'static str,
}

pub const THEMES: [Theme; 3] = [
    Theme {
        name: "tokyo-night",
        foreground: "#c0caf5",
        background: "#1a1b26",
    },
    Theme {
        name: "gruvbox",
        foreground: "#ebdbb2",
        background: "#282828",
    },
    Theme {
        name: "solarized-light",
        foreground: "#586e75",
        background: "#fdf6e3",
    },
];

#[derive(Debug, Clone)]
pub enum SettingsAction {
    FontSizeDelta(i32),
    OpacityDelta(f32),
    #[allow(dead_code)]
    ToggleTheme,
    CycleTheme,
    ReloadConfig,
    Close,
}

#[derive(Debug, Clone)]
pub struct SettingsItem {
    pub label: String,
    pub value: String,
    pub action: SettingsAction,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsContext {
    pub font_size: f32,
    pub opacity: f64,
    pub theme: String,
}

pub struct SettingsMenu {
    pub open: bool,
    pub items: Vec<SettingsItem>,
    pub cursor: usize,
    saved_cells: Option<Vec<Vec<Cell>>>,
    saved_top: Option<usize>,
    saved_cursor: Option<Cursor>,
}

impl Default for SettingsMenu {
    fn default() -> Self {
        Self {
            open: false,
            items: Vec::new(),
            cursor: 0,
            saved_cells: None,
            saved_top: None,
            saved_cursor: None,
        }
    }
}

impl SettingsMenu {
    pub fn new(ctx: &SettingsContext) -> Self {
        let mut menu = Self::default();
        menu.refresh(ctx);
        menu
    }

    pub fn refresh(&mut self, ctx: &SettingsContext) {
        self.items = vec![
            SettingsItem {
                label: "Font Size +".into(),
                value: format!("{:.1}", ctx.font_size),
                action: SettingsAction::FontSizeDelta(1),
            },
            SettingsItem {
                label: "Font Size -".into(),
                value: format!("{:.1}", ctx.font_size),
                action: SettingsAction::FontSizeDelta(-1),
            },
            SettingsItem {
                label: "Opacity +".into(),
                value: format!("{:.2}", ctx.opacity),
                action: SettingsAction::OpacityDelta(0.05),
            },
            SettingsItem {
                label: "Opacity -".into(),
                value: format!("{:.2}", ctx.opacity),
                action: SettingsAction::OpacityDelta(-0.05),
            },
            SettingsItem {
                label: "Theme".into(),
                value: ctx.theme.clone(),
                action: SettingsAction::CycleTheme,
            },
            SettingsItem {
                label: "Reload Config".into(),
                value: String::new(),
                action: SettingsAction::ReloadConfig,
            },
            SettingsItem {
                label: "Close".into(),
                value: String::new(),
                action: SettingsAction::Close,
            },
        ];
        if self.cursor >= self.items.len() {
            self.cursor = 0;
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.cursor = 0;
        }
    }

    pub fn next(&mut self) {
        if !self.items.is_empty() {
            self.cursor = (self.cursor + 1) % self.items.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.items.is_empty() {
            self.cursor = (self.cursor + self.items.len() - 1) % self.items.len();
        }
    }

    pub fn activate(&mut self, ctx: &SettingsContext) -> SettingsAction {
        self.refresh(ctx);
        self.items
            .get(self.cursor)
            .map(|i| i.action.clone())
            .unwrap_or(SettingsAction::Close)
    }

    pub fn theme_name(background: &str) -> String {
        THEMES
            .get(Self::theme_index(background))
            .map(|t| t.name)
            .unwrap_or("custom")
            .to_string()
    }

    pub fn next_theme(&self, current_background: &str) -> (&'static str, &'static str) {
        let idx = Self::theme_index(current_background);
        let next = (idx + 1) % THEMES.len();
        (THEMES[next].foreground, THEMES[next].background)
    }

    fn theme_index(background: &str) -> usize {
        THEMES
            .iter()
            .position(|t| t.background == background)
            .unwrap_or(0)
    }

    fn panel_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.items.len() + 2);
        lines.push(" ZeroTerm Settings ".to_string());
        for (i, item) in self.items.iter().enumerate() {
            let marker = if i == self.cursor { '>' } else { ' ' };
            let value = if item.value.is_empty() {
                String::new()
            } else {
                format!("  {}", item.value)
            };
            lines.push(format!(" {} {}{}", marker, item.label, value));
        }
        lines.push(" arrows: navigate  enter: activate  esc: close ".to_string());
        lines
    }

    pub fn overlay_rect(&self, cols: usize, rows: usize) -> (usize, usize, usize, usize) {
        let lines = self.panel_lines();
        let width = lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(10)
            .min(cols.saturating_sub(2))
            .max(2);
        let height = lines.len().min(rows).max(2);
        let top = (rows - height) / 2;
        let left = (cols - width) / 2;
        (top, left, height, width)
    }

    pub fn overlay_bytes(&self, cols: usize, rows: usize) -> Vec<u8> {
        let lines = self.panel_lines();
        let (top, left, height, width) = self.overlay_rect(cols, rows);
        let panel_bg = (40, 44, 52);
        let panel_fg = (197, 200, 198);
        let sel_bg = (61, 89, 171);
        let sel_fg = (255, 255, 255);

        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[?25l");
        for (i, line) in lines.iter().take(height).enumerate() {
            let (bg, fg) = if i >= 1 && i - 1 == self.cursor {
                (sel_bg, sel_fg)
            } else {
                (panel_bg, panel_fg)
            };
            let text: String = line.chars().take(width).collect();
            let pad = width.saturating_sub(text.chars().count());
            out.extend_from_slice(format!("\x1b[{};{}H", top + i + 1, left + 1).as_bytes());
            out.extend_from_slice(
                format!(
                    "\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m",
                    bg.0, bg.1, bg.2, fg.0, fg.1, fg.2
                )
                .as_bytes(),
            );
            out.extend_from_slice(text.as_bytes());
            for _ in 0..pad {
                out.push(b' ');
            }
            out.extend_from_slice(b"\x1b[0m");
        }
        out
    }

    pub fn save_screen(&mut self, screen: &Screen) {
        let (top, _, height, _) = self.overlay_rect(screen.size().cols, screen.size().rows);
        let buf = screen.buffer();
        self.saved_cells = Some(
            (0..height)
                .map(|i| buf.get(top + i).cloned().unwrap_or_default())
                .collect(),
        );
        self.saved_top = Some(top);
        self.saved_cursor = Some(screen.cursor());
    }

    pub fn restore_screen(&mut self, screen: &mut Screen) {
        if let (Some(cells), Some(top), Some(cursor)) =
            (&self.saved_cells, self.saved_top, &self.saved_cursor)
        {
            for (i, row_cells) in cells.iter().enumerate() {
                screen.set_cells(top + i, row_cells);
            }
            screen.cursor_pos(cursor.row + 1, cursor.col + 1);
            screen.set_cursor_visible(cursor.visible);
        }
        self.saved_cells = None;
        self.saved_top = None;
        self.saved_cursor = None;
    }
}
