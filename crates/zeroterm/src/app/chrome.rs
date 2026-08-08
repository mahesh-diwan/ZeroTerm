use zeroterm_core::screen::Screen;

use crate::overlay::ScreenScratch;

/// SSH host picker overlay, drawn into the active pane like the settings menu.
/// Destructive to covered cells, so the region is snapshotted on open and
/// restored on close (same pattern as settings.rs).
pub struct HostPicker {
    pub open: bool,
    aliases: Vec<String>,
    cursor: usize,
    /// Snapshot of the covered screen region, restored on close.
    scratch: ScreenScratch,
}

impl HostPicker {
    pub fn new() -> Self {
        Self {
            open: false,
            aliases: Vec::new(),
            cursor: 0,
            scratch: ScreenScratch::default(),
        }
    }

    #[cfg(all(unix, feature = "ssh"))]
    pub fn open(&mut self, aliases: Vec<String>) {
        self.aliases = aliases;
        self.cursor = 0;
        self.open = true;
    }

    pub fn next(&mut self) {
        if !self.aliases.is_empty() {
            self.cursor = (self.cursor + 1) % self.aliases.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.aliases.is_empty() {
            self.cursor = (self.cursor + self.aliases.len() - 1) % self.aliases.len();
        }
    }

    #[cfg(all(unix, feature = "ssh"))]
    pub fn selected(&self) -> Option<String> {
        self.aliases.get(self.cursor).cloned()
    }

    pub fn panel_lines(&self) -> Vec<String> {
        let mut lines = vec![" SSH Hosts ".to_string()];
        for (i, alias) in self.aliases.iter().enumerate() {
            let marker = if i == self.cursor { '>' } else { ' ' };
            lines.push(format!(" {} {}", marker, alias));
        }
        lines.push(" arrows: navigate  enter: connect  esc: cancel ".to_string());
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
        let top = (rows.saturating_sub(height)) / 2;
        let left = (cols.saturating_sub(width)) / 2;
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

    #[cfg(all(unix, feature = "ssh"))]
    pub fn save_screen(&mut self, screen: &Screen) {
        let (top, _, height, _) = self.overlay_rect(screen.size().cols, screen.size().rows);
        self.scratch.save_region(screen, top, height);
    }

    pub fn restore_screen(&mut self, screen: &mut Screen) {
        self.scratch.restore(screen);
    }
}
