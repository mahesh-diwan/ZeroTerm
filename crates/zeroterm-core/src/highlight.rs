//! Minimal keyword-based syntax highlighting for terminal lines.
//!
//! Pure function over a line of chars; the renderer maps the returned class
//! indexes to a color palette. Only a tiny shell word list is matched —
//! enough to make a command line readable, nothing more.

/// Highlight class indexes stored in `Cell::syntax_color`.
pub const HL_KEYWORD: u8 = 1;
pub const HL_STRING: u8 = 2;
pub const HL_NUMBER: u8 = 3;
pub const HL_COMMENT: u8 = 4;
pub const HL_URL: u8 = 5;

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "fi", "for", "while", "do", "done", "case", "esac", "function", "echo",
    "export", "cd", "ls", "grep", "sudo", "git", "cargo", "make",
];

/// Class index for one line of chars, or `None` for default coloring.
/// Rules, in order: `#` comment to EOL, `'`/`"` string span, decimal number,
/// whole-word keyword match (so `ifconfig` never matches `if`), then URL spans
/// (`http(s)://…`, `ftp://…`, `www.…`) which override everything else.
pub fn highlight_line(chars: &[char]) -> Vec<Option<u8>> {
    let mut out = vec![None; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '#' => {
                for slot in out.iter_mut().skip(i) {
                    *slot = Some(HL_COMMENT);
                }
                break;
            }
            '\'' | '"' => {
                let start = i;
                let quote = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                for slot in out.iter_mut().take(i.saturating_add(1)).skip(start) {
                    *slot = Some(HL_STRING);
                }
                i += 1;
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                for slot in out.iter_mut().take(i).skip(start) {
                    *slot = Some(HL_NUMBER);
                }
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                if i > start {
                    let word: String = chars[start..i].iter().collect();
                    if KEYWORDS.contains(&word.as_str()) {
                        for slot in out.iter_mut().take(i).skip(start) {
                            *slot = Some(HL_KEYWORD);
                        }
                    }
                }
            }
            _ => i += 1,
        }
    }

    // URL pass: scan every position, tag `http(s)://`, `ftp://`, `www.` runs.
    // ponytail: hand-rolled scanner instead of a regex dep — terminal lines are
    // short, a regex crate for this would be pure weight. Overrides earlier
    // classes (a keyword inside a URL is still a URL).
    let mut i = 0;
    while i < chars.len() {
        if let Some(len) = url_len_at(chars, i) {
            for slot in out.iter_mut().take(i + len).skip(i) {
                *slot = Some(HL_URL);
            }
            i += len;
        } else {
            i += 1;
        }
    }
    out
}

/// Length of a URL token starting at `i`, or `None` if `i` isn't a URL start.
/// URL chars run until whitespace/quote/pipe/brackets; trailing punctuation
/// (`.,:;!?`) is stripped so `https://x.com,` keeps the comma outside.
fn url_len_at(chars: &[char], i: usize) -> Option<usize> {
    const SCHEMES: [&str; 3] = ["http://", "https://", "ftp://"];
    let s: String = chars
        .get(i..i.saturating_add(8).min(chars.len()))?
        .iter()
        .collect();
    if !SCHEMES.iter().any(|p| s.starts_with(p)) && !s.starts_with("www.") {
        return None;
    }
    let mut end = i;
    while end < chars.len() {
        let c = chars[end];
        if c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '`' | '|' | '<' | '>' | '[' | ']' | '{' | '}'
            )
        {
            break;
        }
        end += 1;
    }
    while end > i && matches!(chars[end - 1], '.' | ',' | ':' | ';' | '!' | '?') {
        end -= 1;
    }
    (end > i).then_some(end - i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Vec<Option<u8>> {
        highlight_line(&s.chars().collect::<Vec<char>>())
    }

    #[test]
    fn keyword_matches_whole_word() {
        let hl = line("echo hello");
        assert_eq!(hl[0], Some(HL_KEYWORD));
        assert_eq!(hl[1], Some(HL_KEYWORD));
        assert_eq!(hl[2], Some(HL_KEYWORD));
        assert_eq!(hl[3], Some(HL_KEYWORD));
        assert_eq!(hl[5], None);
    }

    #[test]
    fn keyword_respects_word_boundary() {
        let hl = line("ifconfig if so");
        assert_eq!(hl[0], None); // "ifconfig" is one word, not "if"
        assert_eq!(hl[9], Some(HL_KEYWORD)); // "if" at index 9
        assert_eq!(hl[10], Some(HL_KEYWORD));
    }

    #[test]
    fn string_spans_quotes_and_content() {
        let hl = line("echo \"hi there\"");
        assert_eq!(hl[6], Some(HL_STRING));
        assert_eq!(hl[7], Some(HL_STRING));
        assert_eq!(hl[14], Some(HL_STRING)); // closing quote
        assert_eq!(hl[0], Some(HL_KEYWORD)); // "echo" still a keyword
    }

    #[test]
    fn comment_runs_to_eol() {
        let hl = line("ls -la # hidden stuff");
        assert_eq!(hl[6], None); // space before '#'
        assert_eq!(hl[7], Some(HL_COMMENT)); // '#'
        assert_eq!(hl[8], Some(HL_COMMENT));
        assert_eq!(hl[hl.len() - 1], Some(HL_COMMENT));
    }

    #[test]
    fn number_literals_are_marked() {
        let hl = line("echo 42 7");
        assert_eq!(hl[5], Some(HL_NUMBER));
        assert_eq!(hl[6], Some(HL_NUMBER));
        assert_eq!(hl[8], Some(HL_NUMBER));
    }

    #[test]
    fn cell_field_resets_to_none() {
        let c = crate::cell::Cell::default();
        assert_eq!(c.syntax_color, 0);
        assert!(c.is_empty());
        assert_eq!(crate::cell::Cell::new('a').syntax_color, 0);
    }

    #[test]
    fn non_ascii_alphabetic_does_not_loop() {
        // CJK/accented chars are alphabetic (is_alphabetic) but not ASCII
        // words; the scanner must skip them, not spin forever.
        let hl = line("echo \u{7897} hi");
        assert_eq!(hl[0], Some(HL_KEYWORD)); // echo
        assert_eq!(hl[5], None); // 碗 is not a keyword
        assert_eq!(hl[hl.len() - 1], None); // "hi" is not in the keyword list
    }

    #[test]
    fn urls_are_detected() {
        let hl = line("open https://example.com now");
        let s = "https://example.com".chars().count();
        assert_eq!(hl[5], Some(HL_URL));
        assert_eq!(hl[5 + s - 1], Some(HL_URL));
        assert_eq!(hl[5 + s], None); // trailing space
        assert_eq!(hl[5 + s + 1], None); // "now" not a URL
    }

    #[test]
    fn www_and_trailing_punctuation() {
        let hl = line("www.example.com, done");
        assert_eq!(hl[0], Some(HL_URL));
        assert_eq!(hl[14], Some(HL_URL)); // "www.example.com" (15 chars)
        assert_eq!(hl[15], None); // comma stripped, not URL
        assert_eq!(hl[16], None); // space
    }

    #[test]
    fn url_overrides_keyword_and_comment() {
        let hl = line("echo https://example.com/#sec");
        assert_eq!(hl[0], Some(HL_KEYWORD)); // echo stays a keyword
        let url_start = 5;
        assert_eq!(hl[url_start], Some(HL_URL)); // "https..." is URL even though "http" starts alphabetic
        assert_eq!(hl[hl.len() - 1], Some(HL_URL)); // fragment isn't a comment
    }

    #[test]
    fn no_url_without_scheme() {
        let hl = line("this is not a url httpx://y");
        assert_eq!(hl.iter().filter(|&&c| c == Some(HL_URL)).count(), 0);
    }
}
