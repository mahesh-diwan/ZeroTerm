//! AI response overlay: mid-screen panel that shows the result of an async
//! explain / suggest request (Ctrl+Shift+I = explain last output,
//! Ctrl+Shift+A = suggest next command). The request fires on a background
//! thread and its result is polled from a channel, so the render loop never
//! blocks. Drawn into the active pane's parser screen via synthetic CSI,
//! snapshot on open / restore on close (same pattern as the settings overlay).

use zeroterm_core::cell::{Cell, Cursor};
use zeroterm_core::screen::Screen;

use crate::app::block_output_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(not(feature = "ai"), allow(dead_code))]
pub enum AiKind {
    #[default]
    Explain,
    Suggest,
}

impl AiKind {
    pub fn title(self) -> &'static str {
        match self {
            AiKind::Explain => "AI \u{2014} explain last output",
            AiKind::Suggest => "AI \u{2014} suggest next command",
        }
    }
}

#[derive(Default)]
pub enum AiState {
    #[default]
    Loading,
    Done(String),
    Error(String),
}

#[derive(Default)]
pub struct AiOverlay {
    pub open: bool,
    pub kind: AiKind,
    pub state: AiState,
    /// Receiver for the in-flight request. `None` once the result is consumed.
    pub pending: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    saved_cells: Option<Vec<Vec<Cell>>>,
    saved_top: Option<usize>,
    saved_cursor: Option<Cursor>,
}

impl AiOverlay {
    #[cfg_attr(not(feature = "ai"), allow(dead_code))]
    pub fn open(&mut self, kind: AiKind) {
        self.kind = kind;
        self.state = AiState::Loading;
        self.pending = None;
        self.open = true;
    }

    /// Collect a finished request result if one has arrived; true when the
    /// state changed so the caller can redraw.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(text)) => {
                self.state = AiState::Done(text);
                self.pending = None;
                true
            }
            Ok(Err(e)) => {
                self.state = AiState::Error(e);
                self.pending = None;
                true
            }
            Err(_) => false,
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.pending = None;
        self.saved_cells = None;
        self.saved_top = None;
        self.saved_cursor = None;
    }

    fn panel_lines(&self, rows: usize) -> Vec<String> {
        let body_cap = rows.saturating_sub(2).max(1);
        let mut lines = vec![format!(" {} ", self.kind.title())];
        match &self.state {
            AiState::Loading => lines.push(" requesting\u{2026} (esc to close) ".to_string()),
            AiState::Error(e) => lines.extend(Self::wrap(&format!(" error: {}", e), body_cap)),
            AiState::Done(text) => lines.extend(Self::wrap(text, body_cap)),
        }
        lines.push(" esc: close ".to_string());
        lines
    }

    fn wrap(text: &str, max_lines: usize) -> Vec<String> {
        let mut out: Vec<String> = text.lines().take(max_lines).map(String::from).collect();
        if text.lines().count() > max_lines {
            out.push(" \u{2026} truncated \u{2014} esc to close ".to_string());
        }
        out
    }

    pub fn overlay_rect(&self, cols: usize, rows: usize) -> (usize, usize, usize, usize) {
        let lines = self.panel_lines(rows);
        let width = lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(10)
            .min(cols.saturating_sub(2))
            .max(2);
        let height = lines.len().min(rows).max(2);
        let top = rows.saturating_sub(height) / 2;
        let left = cols.saturating_sub(width) / 2;
        (top, left, height, width)
    }

    pub fn overlay_bytes(&self, cols: usize, rows: usize) -> Vec<u8> {
        let lines = self.panel_lines(rows);
        let (top, left, height, width) = self.overlay_rect(cols, rows);
        let panel_bg = (40, 44, 52);
        let panel_fg = (197, 200, 198);
        let title_fg = (122, 162, 247);

        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[?25l");
        for (i, line) in lines.iter().take(height).enumerate() {
            let fg = if i == 0 { title_fg } else { panel_fg };
            let text: String = line.chars().take(width).collect();
            let pad = width.saturating_sub(text.chars().count());
            out.extend_from_slice(format!("\x1b[{};{}H", top + i + 1, left + 1).as_bytes());
            out.extend_from_slice(
                format!(
                    "\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m",
                    panel_bg.0, panel_bg.1, panel_bg.2, fg.0, fg.1, fg.2
                )
                .as_bytes(),
            );
            out.extend_from_slice(text.as_bytes());
            out.extend(std::iter::repeat_n(b' ', pad));
            out.extend_from_slice(b"\x1b[0m");
        }
        out
    }

    #[cfg_attr(not(feature = "ai"), allow(dead_code))]
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

/// Build the explain prompt from the last non-empty command block: its
/// command plus the plain text of its output rows.
#[cfg_attr(not(feature = "ai"), allow(dead_code))]
pub fn explain_prompt(screen: &Screen) -> Option<String> {
    let block = screen
        .blocks()
        .iter()
        .rev()
        .find(|b| !b.command.is_empty())?;
    let output = block_output_text(screen, block);
    Some(format!(
        "The last command in this terminal session produced the following output. \
         Explain what the command does and what the output means.\n\nCommand: {}\n\nOutput:\n{}",
        block.command, output
    ))
}

/// Build the suggest context from command history (last 10 commands, oldest
/// first). The client wraps it in its own "suggest the next command" prompt.
#[cfg_attr(not(feature = "ai"), allow(dead_code))]
pub fn suggest_context(screen: &Screen) -> Option<String> {
    let history: Vec<&str> = screen
        .blocks()
        .iter()
        .rev()
        .take(10)
        .map(|b| b.command.as_str())
        .filter(|c| !c.is_empty())
        .rev()
        .collect();
    if history.is_empty() {
        return None;
    }
    Some(history.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroterm_core::cell::Cell;

    /// Screen with two command blocks: "ls -la" at row 0 (output row 1) then
    /// "git status" starting at row 1.
    fn screen_with_history() -> Screen {
        let mut screen = Screen::new(80, 24);
        screen.set_cells(0, &"total 4".chars().map(Cell::new).collect::<Vec<_>>());
        screen.set_block_command("ls -la");
        screen.mark_block_boundary();
        screen.cursor_row(2);
        screen.set_cells(
            1,
            &"file.txt  2048".chars().map(Cell::new).collect::<Vec<_>>(),
        );
        screen.set_block_command("git status");
        screen.mark_block_boundary();
        screen
    }

    #[test]
    fn explain_prompt_embeds_last_command_and_output() {
        let prompt = explain_prompt(&screen_with_history()).unwrap();
        assert!(prompt.contains("git status"), "prompt: {}", prompt);
        assert!(prompt.contains("file.txt  2048"));
    }

    #[test]
    fn explain_prompt_none_without_commands() {
        assert!(explain_prompt(&Screen::new(80, 24)).is_none());
    }

    #[test]
    fn suggest_context_returns_history_in_order() {
        assert_eq!(
            suggest_context(&screen_with_history()).unwrap(),
            "ls -la\ngit status"
        );
    }

    #[test]
    fn suggest_context_none_without_commands() {
        assert!(suggest_context(&Screen::new(80, 24)).is_none());
    }

    #[test]
    fn overlay_survives_tiny_window() {
        let mut ai = AiOverlay::default();
        ai.open(AiKind::Explain);
        let (top, left, height, width) = ai.overlay_rect(1, 1);
        assert_eq!(top, 0);
        assert_eq!(left, 0);
        assert!(height >= 2 && width >= 2);
        let (top, left, height, width) = ai.overlay_rect(80, 24);
        assert!(top + height <= 24);
        assert!(left + width <= 80);
    }

    #[test]
    fn poll_collects_finished_result() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok("git commit -m \"done\"".to_string())).unwrap();
        let mut ai = AiOverlay {
            pending: Some(rx),
            ..Default::default()
        };
        assert!(ai.poll());
        assert!(matches!(ai.state, AiState::Done(_)));
        assert!(ai.pending.is_none());
        // A consumed receiver never polls again.
        let (_, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        ai.pending = Some(rx);
        assert!(!ai.poll());
    }
}
