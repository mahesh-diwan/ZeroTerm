//! Pure key routing: decode winit key events into typed actions.
//!
//! The `KeyboardInput` arm of `App::window_event` used to be a 430-line match
//! that interleaved *decoding a keypress* (which key + modifiers maps to which
//! action) with *executing it* (mutating the session, the editor, the PTY).
//! That made the entire shortcut surface untestable — no winit event, no App
//! instance, no test.
//!
//! This module owns the decode half. Every function here is pure:
//!   - `global_key`  : the keybinding table + modal-overlay key routing
//!   - `console_key` : scroll keys, selection extend, copy/paste, escape seqs
//!   - `key_sequence`: keycode -> terminal escape bytes (pure, tested)
//!   - `search_key`  : keys a SearchState overlay consumes
//!   - `ai_key`      : keys an AiOverlay consumes
//!
//! The App keeps only the stateful glue: a thin `apply` match that calls the
//! same methods the old arm called, plus the editor handle (which is itself a
//! deep module). Decoding is the interface; executing is the implementation.

use winit::keyboard::{KeyCode, ModifiersState};

use zeroterm_mux::split::SplitDir;

#[cfg(feature = "ai")]
use crate::ai_overlay::AiKind;

/// Modifier state condensed to the three bits the keybindings care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Mods {
    pub fn from_state(s: &ModifiersState) -> Self {
        Self {
            ctrl: s.control_key(),
            shift: s.shift_key(),
            alt: s.alt_key(),
        }
    }

    /// Ctrl+Shift, Alt excluded — the "application" chord used by most
    /// ZeroTerm shortcuts (new tab, split, search, ...).
    pub fn ctrl_shift(&self) -> bool {
        self.ctrl && self.shift && !self.alt
    }

    /// Bare chord: no modifier at all.
    pub fn bare(&self) -> bool {
        !self.ctrl && !self.shift && !self.alt
    }
}

/// Context flags the pure router needs to know which modal overlay is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyCtx {
    /// SSH host picker is open: its arrow/enter/escape keys take priority.
    pub picker_open: bool,
    /// Settings menu is open: its arrow/enter/escape keys take priority.
    pub settings_open: bool,
}

/// Keys the search overlay consumes (all other keys go to the query prompt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchKey {
    Close,
    Backspace,
    Step(bool),
    /// Printable text appended to the query, char by char.
    Text(String),
}

/// Keys the AI overlay consumes (Escape or the toggle chord closes it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiKey {
    Close,
}

/// Global keybinding actions. These mutate App state and are applied by App.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalAction {
    NewTab,
    CloseTab,
    Split(SplitDir),
    ToggleSettings,
    ToggleSearch,
    ToggleFloating,
    ToggleQuake,
    NextTab,
    PrevTab,
    SwitchToTab(usize),
    FocusPane(KeyCode),
    CycleOpacity,
    JumpBlock(i32),
    #[cfg(feature = "ai")]
    OpenAi(AiKind),
    #[cfg(all(unix, feature = "ssh"))]
    Ssh,
    #[cfg(feature = "plugins")]
    RunPlugin,
    /// Host picker arrow/enter/escape while it is open.
    Picker(PickerKey),
    /// Settings menu arrow/enter/escape while it is open.
    Settings(SettingsKey),
    /// No global binding matched.
    Pass,
}

/// Host picker navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKey {
    Up,
    Down,
    Select,
    Escape,
}

/// Settings menu navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsKey {
    Up,
    Down,
    Activate,
    Escape,
}

/// Console-layer actions: scroll, selection, copy/paste, and PTY bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleAction {
    ScrollUp(usize),
    ScrollDown(usize),
    ScrollTop,
    ScrollBottom,
    /// Shift+arrow selection extend. App falls back to the raw escape
    /// sequence when the feature is disabled.
    ExtendSelection { code: KeyCode, ctrl: bool },
    CopySelection,
    Paste,
    /// Raw bytes to write to the active pane's PTY.
    Pty(Vec<u8>),
    None,
}

/// Global keybinding table: (code, mods) -> action.
///
/// Mirrors the old `KeyboardInput` arm's ordering: Alt+E and Ctrl+Shift+P are
/// checked before the modal overlays (they may open/close them), then the open
/// overlay consumes its own keys, then the application chords.
pub fn global_key(code: KeyCode, mods: Mods, ctx: KeyCtx) -> GlobalAction {
    if mods.ctrl_shift() && code == KeyCode::KeyP {
        return GlobalAction::ToggleSettings;
    }
    if ctx.picker_open {
        return match code {
            KeyCode::ArrowUp => GlobalAction::Picker(PickerKey::Up),
            KeyCode::ArrowDown => GlobalAction::Picker(PickerKey::Down),
            KeyCode::Enter => GlobalAction::Picker(PickerKey::Select),
            KeyCode::Escape => GlobalAction::Picker(PickerKey::Escape),
            _ => GlobalAction::Pass,
        };
    }
    if ctx.settings_open {
        return match code {
            KeyCode::ArrowUp => GlobalAction::Settings(SettingsKey::Up),
            KeyCode::ArrowDown => GlobalAction::Settings(SettingsKey::Down),
            KeyCode::Enter => GlobalAction::Settings(SettingsKey::Activate),
            KeyCode::Escape => GlobalAction::Settings(SettingsKey::Escape),
            _ => GlobalAction::Pass,
        };
    }
    if mods.ctrl_shift() {
        return match code {
            KeyCode::KeyT => GlobalAction::NewTab,
            KeyCode::KeyW => GlobalAction::CloseTab,
            KeyCode::KeyE => GlobalAction::Split(SplitDir::Vertical),
            KeyCode::KeyD => GlobalAction::Split(SplitDir::Horizontal),
            #[cfg(feature = "ai")]
            KeyCode::KeyI => GlobalAction::OpenAi(AiKind::Explain),
            #[cfg(feature = "ai")]
            KeyCode::KeyA => GlobalAction::OpenAi(AiKind::Suggest),
            KeyCode::KeyO => GlobalAction::CycleOpacity,
            #[cfg(all(unix, feature = "ssh"))]
            KeyCode::KeyS => GlobalAction::Ssh,
            #[cfg(feature = "plugins")]
            KeyCode::KeyB => GlobalAction::RunPlugin,
            KeyCode::KeyK => GlobalAction::JumpBlock(-1),
            KeyCode::KeyJ => GlobalAction::JumpBlock(1),
            KeyCode::KeyF => GlobalAction::ToggleSearch,
            KeyCode::KeyG => GlobalAction::ToggleFloating,
            KeyCode::Tab => GlobalAction::PrevTab,
            _ => GlobalAction::Pass,
        };
    }
    if mods.ctrl && !mods.shift && !mods.alt && code == KeyCode::Tab {
        return GlobalAction::NextTab;
    }
    if mods.bare() && code == KeyCode::F12 {
        return GlobalAction::ToggleQuake;
    }
    if mods.alt && !mods.ctrl && !mods.shift {
        if matches!(
            code,
            KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::ArrowUp
                | KeyCode::ArrowDown
        ) {
            return GlobalAction::FocusPane(code);
        }
        if let Some(idx) = digit_tab_index(code) {
            return GlobalAction::SwitchToTab(idx);
        }
    }
    GlobalAction::Pass
}

/// Alt+1..=9 -> tab index 0..=8.
fn digit_tab_index(code: KeyCode) -> Option<usize> {
    match code {
        KeyCode::Digit1 => Some(0),
        KeyCode::Digit2 => Some(1),
        KeyCode::Digit3 => Some(2),
        KeyCode::Digit4 => Some(3),
        KeyCode::Digit5 => Some(4),
        KeyCode::Digit6 => Some(5),
        KeyCode::Digit7 => Some(6),
        KeyCode::Digit8 => Some(7),
        KeyCode::Digit9 => Some(8),
        _ => None,
    }
}

/// Console-layer decode: shift-scroll, selection extend, copy/paste, and the
/// keycode -> escape-sequence encoding. Returns `None` when nothing applies.
pub fn console_key(code: KeyCode, mods: Mods) -> ConsoleAction {
    if mods.shift && !mods.alt {
        match code {
            KeyCode::PageUp if !mods.ctrl => return ConsoleAction::ScrollUp(20),
            KeyCode::PageDown if !mods.ctrl => return ConsoleAction::ScrollDown(20),
            KeyCode::Home if !mods.ctrl => return ConsoleAction::ScrollTop,
            KeyCode::End if !mods.ctrl => return ConsoleAction::ScrollBottom,
            KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::ArrowUp
            | KeyCode::ArrowDown => {
                return ConsoleAction::ExtendSelection {
                    code,
                    ctrl: mods.ctrl,
                };
            }
            _ => {}
        }
    }
    // Ctrl+Shift+C/V are checked BEFORE the plain ctrl-letter encoding: the
    // old arm's guard order (`_ if ctrl && !alt` before `_ if ctrl && shift`)
    // shadowed them, so copy/paste were dead code and Ctrl+Shift+C sent ^C.
    if mods.ctrl && mods.shift && !mods.alt {
        return match code {
            KeyCode::KeyC => ConsoleAction::CopySelection,
            KeyCode::KeyV => ConsoleAction::Paste,
            _ => {
                if let Some(seq) = key_sequence(code, mods) {
                    ConsoleAction::Pty(seq)
                } else {
                    ConsoleAction::None
                }
            }
        };
    }
    match key_sequence(code, mods) {
        Some(seq) => ConsoleAction::Pty(seq),
        None => ConsoleAction::None,
    }
}

/// Keycode -> terminal escape sequence. Pure and unit-tested; the same bytes
/// the old arm produced, minus the copy/paste side effects (now actions).
pub fn key_sequence(code: KeyCode, mods: Mods) -> Option<Vec<u8>> {
    let seq: Vec<u8> = match code {
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Escape => vec![0x1b],
        KeyCode::ArrowUp => vec![0x1b, b'[', b'A'],
        KeyCode::ArrowDown => vec![0x1b, b'[', b'B'],
        KeyCode::ArrowRight => vec![0x1b, b'[', b'C'],
        KeyCode::ArrowLeft => vec![0x1b, b'[', b'D'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::F1 => vec![0x1b, b'[', b'1', b'1', b'~'],
        KeyCode::F2 => vec![0x1b, b'[', b'1', b'2', b'~'],
        KeyCode::F3 => vec![0x1b, b'[', b'1', b'3', b'~'],
        KeyCode::F4 => vec![0x1b, b'[', b'1', b'4', b'~'],
        KeyCode::F5 => vec![0x1b, b'[', b'1', b'5', b'~'],
        KeyCode::F6 => vec![0x1b, b'[', b'1', b'7', b'~'],
        KeyCode::F7 => vec![0x1b, b'[', b'1', b'8', b'~'],
        KeyCode::F8 => vec![0x1b, b'[', b'1', b'9', b'~'],
        KeyCode::F9 => vec![0x1b, b'[', b'2', b'0', b'~'],
        KeyCode::F10 => vec![0x1b, b'[', b'2', b'1', b'~'],
        KeyCode::F11 => vec![0x1b, b'[', b'2', b'3', b'~'],
        KeyCode::F12 => vec![0x1b, b'[', b'2', b'4', b'~'],
        // Ctrl+letter -> control byte; Ctrl+Shift handled above (copy/paste).
        _ if mods.ctrl && !mods.alt => {
            let b = ctrl_byte(code)?;
            vec![b]
        }
        _ => return None,
    };
    Some(seq)
}

/// Ctrl+A..=Z / Ctrl+Space -> the classic control code.
fn ctrl_byte(code: KeyCode) -> Option<u8> {
    Some(match code {
        KeyCode::KeyA => 0x01,
        KeyCode::KeyB => 0x02,
        KeyCode::KeyC => 0x03,
        KeyCode::KeyD => 0x04,
        KeyCode::KeyE => 0x05,
        KeyCode::KeyF => 0x06,
        KeyCode::KeyG => 0x07,
        KeyCode::KeyH => 0x08,
        KeyCode::KeyI => 0x09,
        KeyCode::KeyJ => 0x0a,
        KeyCode::KeyK => 0x0b,
        KeyCode::KeyL => 0x0c,
        KeyCode::KeyM => 0x0d,
        KeyCode::KeyN => 0x0e,
        KeyCode::KeyO => 0x0f,
        KeyCode::KeyP => 0x10,
        KeyCode::KeyQ => 0x11,
        KeyCode::KeyR => 0x12,
        KeyCode::KeyS => 0x13,
        KeyCode::KeyT => 0x14,
        KeyCode::KeyU => 0x15,
        KeyCode::KeyV => 0x16,
        KeyCode::KeyW => 0x17,
        KeyCode::KeyX => 0x18,
        KeyCode::KeyY => 0x19,
        KeyCode::KeyZ => 0x1a,
        KeyCode::Space => 0x00,
        _ => return None,
    })
}

/// Keys the search overlay consumes while open.
pub fn search_key(code: KeyCode, mods: Mods, text: Option<&str>) -> SearchKey {
    match code {
        KeyCode::Escape => return SearchKey::Close,
        KeyCode::KeyF if mods.ctrl && mods.shift => return SearchKey::Close,
        KeyCode::Backspace => return SearchKey::Backspace,
        KeyCode::Enter | KeyCode::ArrowDown if !mods.shift => {
            return SearchKey::Step(true);
        }
        KeyCode::Enter | KeyCode::ArrowUp if mods.shift => return SearchKey::Step(false),
        KeyCode::ArrowUp => return SearchKey::Step(false),
        KeyCode::ArrowDown => return SearchKey::Step(true),
        _ => {}
    }
    match text {
        Some(t) if !t.is_empty() && !mods.ctrl && !mods.alt => {
            SearchKey::Text(t.to_string())
        }
        _ => SearchKey::Text(String::new()),
    }
}

/// Keys that close the AI overlay while it is open. Any other key is swallowed
/// by the overlay (the caller returns without forwarding it) but does NOT close
/// the panel — only Escape or the toggle chords do.
#[cfg_attr(not(feature = "ai"), allow(unused_variables))]
pub fn ai_key(code: KeyCode, mods: Mods) -> Option<AiKey> {
    match code {
        KeyCode::Escape => Some(AiKey::Close),
        #[cfg(feature = "ai")]
        KeyCode::KeyI if mods.ctrl_shift() => Some(AiKey::Close),
        #[cfg(feature = "ai")]
        KeyCode::KeyA if mods.ctrl_shift() => Some(AiKey::Close),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(ctrl: bool, shift: bool, alt: bool) -> Mods {
        Mods { ctrl, shift, alt }
    }

    #[test]
    fn ctrl_shift_t_creates_tab() {
        assert_eq!(
            global_key(KeyCode::KeyT, m(true, true, false), KeyCtx::default()),
            GlobalAction::NewTab
        );
    }

    #[test]
    fn ctrl_shift_w_closes_tab() {
        assert_eq!(
            global_key(KeyCode::KeyW, m(true, true, false), KeyCtx::default()),
            GlobalAction::CloseTab
        );
    }

    #[test]
    fn ctrl_shift_e_splits_vertical() {
        assert_eq!(
            global_key(KeyCode::KeyE, m(true, true, false), KeyCtx::default()),
            GlobalAction::Split(SplitDir::Vertical)
        );
    }

    #[test]
    fn ctrl_shift_d_splits_horizontal() {
        assert_eq!(
            global_key(KeyCode::KeyD, m(true, true, false), KeyCtx::default()),
            GlobalAction::Split(SplitDir::Horizontal)
        );
    }

    #[test]
    fn ctrl_shift_f_toggles_search() {
        assert_eq!(
            global_key(KeyCode::KeyF, m(true, true, false), KeyCtx::default()),
            GlobalAction::ToggleSearch
        );
    }

    #[test]
    fn ctrl_shift_p_toggles_settings_even_with_picker_open() {
        assert_eq!(
            global_key(
                KeyCode::KeyP,
                m(true, true, false),
                KeyCtx {
                    picker_open: true,
                    settings_open: false
                }
            ),
            GlobalAction::ToggleSettings
        );
    }

    #[test]
    fn picker_owns_arrows_when_open() {
        assert_eq!(
            global_key(
                KeyCode::ArrowUp,
                m(false, false, false),
                KeyCtx {
                    picker_open: true,
                    settings_open: false
                }
            ),
            GlobalAction::Picker(PickerKey::Up)
        );
    }

    #[test]
    fn settings_owns_enter_when_open() {
        assert_eq!(
            global_key(
                KeyCode::Enter,
                m(false, false, false),
                KeyCtx {
                    picker_open: false,
                    settings_open: true
                }
            ),
            GlobalAction::Settings(SettingsKey::Activate)
        );
    }

    #[test]
    fn ctrl_tab_next_tab_ctrl_shift_tab_prev() {
        assert_eq!(
            global_key(KeyCode::Tab, m(true, false, false), KeyCtx::default()),
            GlobalAction::NextTab
        );
        assert_eq!(
            global_key(KeyCode::Tab, m(true, true, false), KeyCtx::default()),
            GlobalAction::PrevTab
        );
    }

    #[test]
    fn alt_digit_switches_tab() {
        assert_eq!(
            global_key(KeyCode::Digit3, m(false, false, true), KeyCtx::default()),
            GlobalAction::SwitchToTab(2)
        );
    }

    #[test]
    fn alt_arrows_focus_adjacent_pane() {
        assert_eq!(
            global_key(KeyCode::ArrowRight, m(false, false, true), KeyCtx::default()),
            GlobalAction::FocusPane(KeyCode::ArrowRight)
        );
    }

    #[test]
    fn f12_toggles_quake() {
        assert_eq!(
            global_key(KeyCode::F12, m(false, false, false), KeyCtx::default()),
            GlobalAction::ToggleQuake
        );
    }

    #[test]
    fn shift_page_up_scrolls_up_20() {
        assert_eq!(
            console_key(KeyCode::PageUp, m(false, true, false)),
            ConsoleAction::ScrollUp(20)
        );
    }

    #[test]
    fn shift_home_scrolls_to_top() {
        assert_eq!(
            console_key(KeyCode::Home, m(false, true, false)),
            ConsoleAction::ScrollTop
        );
    }

    #[test]
    fn shift_arrow_extends_selection() {
        assert_eq!(
            console_key(KeyCode::ArrowLeft, m(false, true, false)),
            ConsoleAction::ExtendSelection {
                code: KeyCode::ArrowLeft,
                ctrl: false
            }
        );
    }

    #[test]
    fn ctrl_shift_c_copies_not_sends_control_c() {
        // Regression: the old guard order shadowed this arm, so Ctrl+Shift+C
        // silently sent ^C to the shell. Copy must win.
        assert_eq!(
            console_key(KeyCode::KeyC, m(true, true, false)),
            ConsoleAction::CopySelection
        );
    }

    #[test]
    fn ctrl_shift_v_pastes() {
        assert_eq!(
            console_key(KeyCode::KeyV, m(true, true, false)),
            ConsoleAction::Paste
        );
    }

    #[test]
    fn plain_enter_encodes_cr() {
        assert_eq!(
            key_sequence(KeyCode::Enter, m(false, false, false)),
            Some(vec![b'\r'])
        );
    }

    #[test]
    fn arrow_keys_encode_csi() {
        assert_eq!(
            key_sequence(KeyCode::ArrowUp, m(false, false, false)),
            Some(vec![0x1b, b'[', b'A'])
        );
        assert_eq!(
            key_sequence(KeyCode::ArrowDown, m(false, false, false)),
            Some(vec![0x1b, b'[', b'B'])
        );
    }

    #[test]
    fn ctrl_c_encodes_control_byte() {
        assert_eq!(
            key_sequence(KeyCode::KeyC, m(true, false, false)),
            Some(vec![0x03])
        );
    }

    #[test]
    fn ctrl_shift_letter_other_than_cv_keeps_control_byte() {
        // Ctrl+Shift+X is not copy/paste; it stays the classic control code.
        assert_eq!(
            console_key(KeyCode::KeyX, m(true, true, false)),
            ConsoleAction::Pty(vec![0x18])
        );
    }

    #[test]
    fn printable_letter_is_not_a_console_key() {
        assert_eq!(
            console_key(KeyCode::KeyA, m(false, false, false)),
            ConsoleAction::None
        );
    }

    #[test]
    fn search_enter_steps_forward() {
        assert_eq!(
            search_key(KeyCode::Enter, m(false, false, false), None),
            SearchKey::Step(true)
        );
    }

    #[test]
    fn search_shift_enter_steps_backward() {
        assert_eq!(
            search_key(KeyCode::Enter, m(false, true, false), None),
            SearchKey::Step(false)
        );
    }

    #[test]
    fn search_backspace_and_escape() {
        assert_eq!(
            search_key(KeyCode::Backspace, m(false, false, false), None),
            SearchKey::Backspace
        );
        assert_eq!(
            search_key(KeyCode::Escape, m(false, false, false), None),
            SearchKey::Close
        );
    }

    #[test]
    fn search_printable_text_appends() {
        assert_eq!(
            search_key(KeyCode::KeyA, m(false, false, false), Some("a")),
            SearchKey::Text("a".to_string())
        );
    }

    #[test]
    fn ai_escape_closes_other_keys_do_not() {
        assert_eq!(
            ai_key(KeyCode::Escape, m(false, false, false)),
            Some(AiKey::Close)
        );
        assert_eq!(ai_key(KeyCode::KeyA, m(false, false, false)), None);
    }
}
