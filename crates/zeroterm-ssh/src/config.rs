//! Parsing for `~/.ssh/config`.
//!
//! Hand-rolled line parser supporting the keys the terminal uses: `Host`,
//! `HostName`, `User`, `Port`, `IdentityFile`, `ForwardAgent`. Lookup follows
//! ssh(1) "first matching `Host` block wins" semantics with `*` and `?` glob
//! patterns.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One `Host <pattern[,pattern...]>` block from an ssh config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshConfigEntry {
    pub host: String,
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
    pub forward_agent: bool,
}

/// The parsed ssh config: an ordered list of `Host` blocks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshConfig {
    entries: Vec<SshConfigEntry>,
}

impl SshConfig {
    /// Load `~/.ssh/config`. A missing or unreadable file yields an empty
    /// config rather than an error, matching ssh(1)'s tolerance.
    pub fn load() -> SshConfig {
        SshConfig::load_from(&default_config_path())
    }

    /// Parse the config file at `path`. Never errors: unreadable files yield
    /// an empty config.
    pub fn load_from(path: &Path) -> SshConfig {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return SshConfig::default();
        };
        SshConfig::parse_str(&contents)
    }

    fn parse_str(contents: &str) -> SshConfig {
        let mut entries: Vec<SshConfigEntry> = Vec::new();
        for raw in contents.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(keyword), Some(first)) = (parts.next(), parts.next()) else {
                continue;
            };
            let value = read_value(&mut parts, first);
            match keyword.to_ascii_lowercase().as_str() {
                "host" => entries.push(SshConfigEntry {
                    host: value,
                    ..Default::default()
                }),
                "hostname" => {
                    if let Some(entry) = entries.last_mut() {
                        entry.host_name = Some(expand_tilde(&value));
                    }
                }
                "user" => {
                    if let Some(entry) = entries.last_mut() {
                        entry.user = Some(value);
                    }
                }
                "port" => {
                    if let Some(entry) = entries.last_mut() {
                        entry.port = value.parse().ok();
                    }
                }
                "identityfile" => {
                    if let Some(entry) = entries.last_mut() {
                        entry.identity_file = Some(PathBuf::from(expand_tilde(&value)));
                    }
                }
                "forwardagent" => {
                    if let Some(entry) = entries.last_mut() {
                        match value.to_ascii_lowercase().as_str() {
                            "yes" | "on" | "true" | "1" => entry.forward_agent = true,
                            "no" | "off" | "false" | "0" => entry.forward_agent = false,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        SshConfig { entries }
    }

    /// Return the first entry whose `Host` pattern matches `host`
    /// (first-match-wins, mirroring ssh(1)). Hostnames match case-insensitively.
    pub fn lookup(&self, host: &str) -> Option<&SshConfigEntry> {
        let host = host.to_ascii_lowercase();
        self.entries.iter().find(|e| patterns_match(&e.host, &host))
    }
}

/// Path of the user's ssh config (`$HOME/.ssh/config`).
pub fn default_config_path() -> PathBuf {
    std::env::var("HOME")
        .map(|h| Path::new(&h).join(".ssh").join("config"))
        .unwrap_or_else(|_| PathBuf::from(".ssh/config"))
}

/// Re-join tokens that `split_whitespace` split inside a quoted value, e.g.
/// `IdentityFile "~/keys/my id.rsa"`. Unquoted values pass through unchanged.
fn read_value(parts: &mut std::str::SplitWhitespace<'_>, first: &str) -> String {
    if !first.starts_with('"') || first.trim_end().ends_with('"') {
        return first.to_string();
    }
    let mut joined = first.to_string();
    while !joined.trim_end().ends_with('"') {
        match parts.next() {
            Some(tok) => {
                joined.push(' ');
                joined.push_str(tok);
            }
            None => break,
        }
    }
    joined
}

/// True when `host` matches any comma-separated pattern in `patterns`.
fn patterns_match(patterns: &str, host: &str) -> bool {
    patterns
        .split(',')
        .map(str::trim)
        .any(|p| glob_lite(p, host))
}

/// Glob-lite `*`/`?` matcher on already-lowercased inputs.
///
/// ponytail: `*` and `?` only. ssh(1)'s `[!a-z]` character classes and
/// `%d`/`%h`/`%u` hostname token expansion are not implemented; add a real
/// glob parser if any host pattern uses them.
fn glob_lite(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0, 0);
    let mut star = None;
    let mut mark = 0;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// Expand a leading `~`/`~/` to the user's home directory and strip quotes.
///
/// ponytail: bare `~`/`~/` and double-quote stripping only; `~otheruser` and
/// `%d`/`%h`/`%u` token expansion are not implemented.
fn expand_tilde(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed == "~" {
        return std::env::var("HOME").unwrap_or(trimmed.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "\
# work host
Host            work,work.example
    HostName        work.corp.example
    User            alice
    Port            2222
    ForwardAgent    yes

# quoted identity
Host quotes
    IdentityFile    \"~/keys/my id.rsa\"

# bare name host
host   LOCAL
    HostName  localhost

# default identity
Host *
    User            me
    IdentityFile    ~/.ssh/id_ed25519
";

    fn write_tmp(name: &str, contents: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("zeroterm-ssh-config-{name}"));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn parses_full_example_with_all_keys() {
        let config = SshConfig::load_from(&write_tmp("full", FULL));
        assert_eq!(config.entries.len(), 4);
        let work = config.lookup("work").unwrap();
        assert_eq!(work.host, "work,work.example");
        assert_eq!(work.host_name.as_deref(), Some("work.corp.example"));
        assert_eq!(work.user.as_deref(), Some("alice"));
        assert_eq!(work.port, Some(2222));
        assert!(work.forward_agent);
        let quotes = config.lookup("quotes").unwrap();
        assert_eq!(
            quotes.identity_file.as_deref(),
            Some(Path::new(&format!("{}/keys/my id.rsa", home_dir())))
        );
        std::fs::remove_file(std::env::temp_dir().join("zeroterm-ssh-config-full")).ok();
    }

    #[test]
    fn missing_file_is_empty() {
        let config = SshConfig::load_from(Path::new("/nonexistent/zeroterm/config"));
        assert_eq!(config.entries.len(), 0);
        assert!(config.lookup("anything").is_none());
    }

    #[test]
    fn comment_and_blank_lines_skipped() {
        let config = SshConfig::load_from(&write_tmp("blank", "# only a comment\n\n  \nHost a\n"));
        assert_eq!(config.entries.len(), 1);
        assert_eq!(config.lookup("a").map(|e| e.host.as_str()), Some("a"));
    }

    #[test]
    fn keys_are_case_insensitive() {
        let config = SshConfig::load_from(&write_tmp(
            "case",
            "hOsT mixed\nhOsTnAmE host.invalid\nPoRt 2200\n",
        ));
        let entry = config.lookup("mixed").unwrap();
        assert_eq!(entry.host_name.as_deref(), Some("host.invalid"));
        assert_eq!(entry.port, Some(2200));
    }

    #[test]
    fn wildcard_matches_fallback() {
        let config = SshConfig::load_from(&write_tmp(
            "wildcard",
            "Host github.com\n    User git\nHost *\n    User fallback\n",
        ));
        assert_eq!(
            config.lookup("github.com").unwrap().user.as_deref(),
            Some("git")
        );
        assert_eq!(
            config.lookup("other.example").unwrap().user.as_deref(),
            Some("fallback")
        );
    }

    #[test]
    fn question_mark_glob_matches_single_char() {
        let config = SshConfig::load_from(&write_tmp("qmark", "Host srv?\n    User bob\n"));
        assert!(config.lookup("srv1").is_some());
        assert!(config.lookup("srv12").is_none());
    }

    #[test]
    fn first_match_wins() {
        let config = SshConfig::load_from(&write_tmp(
            "first",
            "Host a\n    User first\nHost a\n    User second\n",
        ));
        assert_eq!(config.lookup("a").unwrap().user.as_deref(), Some("first"));
    }

    #[test]
    fn tilde_expansion_in_identity_and_hostname() {
        let home = home_dir();
        let config = SshConfig::load_from(&write_tmp(
            "tilde",
            "Host x\n    IdentityFile ~/.ssh/id_x\n    HostName ~/unix.sock\n",
        ));
        let entry = config.lookup("x").unwrap();
        assert_eq!(
            entry.identity_file.as_deref(),
            Some(Path::new(&format!("{home}/.ssh/id_x")))
        );
        assert_eq!(
            entry.host_name.as_deref(),
            Some(format!("{home}/unix.sock").as_str())
        );
    }

    #[test]
    fn port_defaults_to_none() {
        let config = SshConfig::load_from(&write_tmp("port", "Host p\n    User u\n"));
        assert_eq!(config.lookup("p").unwrap().port, None);
    }

    #[test]
    fn config_is_serializable() {
        let config = SshConfig::load_from(&write_tmp("serde", "Host s\n    User u\n    Port 23\n"));
        let json = serde_json::to_string(&config).unwrap();
        let back: SshConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.lookup("s").unwrap().port, Some(23));
    }

    fn home_dir() -> String {
        std::env::var("HOME").expect("HOME must be set in tests")
    }
}
