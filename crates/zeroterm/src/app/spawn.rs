//! Shell bootstrap + PTY/SSH process spawning. Carved out of `session.rs`,
//! which used to hold three unrelated clusters (pane, spawn, session) in one
//! file. Nothing here touches session state: given a command + size + wake
//! handle, these functions return ready pty channels.

#[cfg(all(unix, feature = "ssh"))]
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::Result;
use tracing::warn;
use winit::event_loop::EventLoopProxy;

use zeroterm_core::pty::{PortablePtyBackend, PtyBackend};
use zeroterm_core::screen::Size as PtySize;

use crate::app::pane::PtyCommand;

/// Best-effort write of a shell bootstrap file. On failure we warn loudly
/// and return false so the caller can fall back to a PLAIN shell invocation:
/// passing `--rcfile` (bash) or `ZDOTDIR` (zsh) for a file that was never
/// written would print an error at every prompt AND silently drop the user's
/// real rcfile (zsh) or the starship/OSC-133 integration (both).
fn write_bootstrap_file(path: &std::path::Path, content: &str) -> bool {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!("Failed to create bootstrap dir {}: {}", parent.display(), e);
            return false;
        }
    }
    match std::fs::write(path, content) {
        Ok(()) => true,
        Err(e) if path.exists() => {
            // Read-only dir but a bootstrap from a previous run is already on
            // disk: keep using it — a (possibly stale) bootstrap still provides
            // starship + OSC 133 integration, while dropping to a plain shell
            // would lose both.
            warn!(
                "Failed to refresh bootstrap file {} (using existing): {}",
                path.display(),
                e
            );
            true
        }
        Err(e) => {
            warn!("Failed to write bootstrap file {}: {}", path.display(), e);
            false
        }
    }
}

pub fn starship_setup(
    shell: &str,
    shell_args: &[String],
) -> (String, Vec<String>, Vec<(&'static str, String)>) {
    let has_starship = std::process::Command::new("starship")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !has_starship {
        return (shell.to_string(), shell_args.to_vec(), Vec::new());
    }
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| std::path::Path::new(&h).join(".config"))
                .unwrap_or_default()
        });
    let user_starship = config_dir.join("starship.toml");
    let zt_starship = config_dir.join("zeroterm/starship.toml");
    // The starship.toml copy is cosmetic-only (starship falls back to its own
    // default config), so a write failure is logged but never fatal.
    if !user_starship.exists() && !zt_starship.exists() {
        let _ = write_bootstrap_file(&zt_starship, include_str!("../../assets/starship.toml"));
    }
    let mut env = Vec::new();
    if zt_starship.exists() {
        env.push((
            "STARSHIP_CONFIG",
            zt_starship.to_string_lossy().into_owned(),
        ));
    }
    env.push(("STARSHIP_SHELL", shell.to_string()));

    let init = if shell.ends_with("bash") {
        // bash --rcfile replaces ~/.bashrc for the interactive shell, but a
        // `-l` login shell IGNORES --rcfile and reads profile files instead
        // (verified in a PTY: with -l the rcfile never runs). So we run plain
        // interactive bash with --rcfile pointing at a bootstrap that
        // reproduces the login environment (/etc/profile, ~/.bash_profile),
        // sources the user's real ~/.bashrc, then enables starship. The old
        // `eval "$(starship init bash)"; exec bash -l` evaluated the init and
        // immediately discarded it across exec — the fresh login shell never
        // saw starship, so users got the stock bash prompt no matter what.
        let boot = config_dir.join("zeroterm/bashrc");
        let content = r#"# ZeroTerm bootstrap: login-equivalent env + user rc + starship
if [ -f /etc/profile ]; then . /etc/profile; fi
profile_loaded=0
for f in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
  if [ -f "$f" ]; then . "$f"; profile_loaded=1; break; fi
done
# Source ~/.bashrc directly only if no login profile handled it: profiles
# commonly source ~/.bashrc themselves, and sourcing it twice re-runs
# aliases/exports/banners and can corrupt starship's precmd ordering.
if [ "$profile_loaded" -eq 0 ] && [ -f "$HOME/.bashrc" ]; then . "$HOME/.bashrc"; fi
eval "$(starship init bash)"
# ZeroTerm shell integration: OSC 133 command blocks + exit codes + OSC 7 cwd.
# Runs first in PROMPT_COMMAND so \$? still holds the last command's status
# (starship's own hook is chained after, so it sees the same exit code).
__zeroterm_precmd() {
  local code=$?
  printf '\033]133;D;%s\007' "$code"
  printf '\033]133;A\007'
  printf '\033]7;file://%s%s\007' "${HOSTNAME:-localhost}" "${PWD// /%20}"
}
PROMPT_COMMAND="__zeroterm_precmd;${PROMPT_COMMAND:-}"
"#;
        // Only pass --rcfile when the bootstrap actually landed: a failed
        // write would make bash print an error and run with no integration.
        if write_bootstrap_file(&boot, content) {
            Some(vec![
                r"--rcfile".into(),
                boot.to_string_lossy().into_owned(),
            ])
        } else {
            None
        }
    } else if shell.ends_with("zsh") {
        // zsh has no --rcfile; ZDOTDIR points at a directory whose .zshrc
        // reproduces the login env (.zprofile), sources the user's real
        // .zshrc, then enables starship.
        let zdotdir = config_dir.join("zeroterm/zdotdir");
        let content = r#"# ZeroTerm bootstrap: login-equivalent env + user rc + starship
if [ -f /etc/zprofile ]; then . /etc/zprofile; fi
if [ -f "$HOME/.zprofile" ]; then . "$HOME/.zprofile"; fi
if [ -f "$HOME/.zshrc" ]; then . "$HOME/.zshrc"; fi
eval "$(starship init zsh)"
# ZeroTerm shell integration: OSC 133 blocks + exit codes + OSC 7 cwd.
__zeroterm_precmd() {
  local code=$?
  printf '\033]133;D;%s\007' "$code"
  printf '\033]133;A\007'
  printf '\033]7;file://%s%s\007' "${HOSTNAME:-localhost}" "${PWD// /%20}"
}
# PREPEND (not append): starship's own precmd runs `starship prompt`, which
# executes commands and resets \$? for every later hook. Running FIRST is what
# lets us capture the real exit status of the last command.
precmd_functions=(__zeroterm_precmd $precmd_functions)
"#;
        // Only point ZDOTDIR at the bootstrap dir when its .zshrc actually
        // landed — a failed write would leave zsh reading an empty rcfile and
        // silently skip the user's real ~/.zshrc.
        if write_bootstrap_file(&zdotdir.join(".zshrc"), content) {
            env.push(("ZDOTDIR", zdotdir.to_string_lossy().into_owned()));
            Some(vec![])
        } else {
            None
        }
    } else {
        None
    };
    match init {
        Some(args) => (shell.to_string(), args, env),
        None => (shell.to_string(), shell_args.to_vec(), env),
    }
}

pub fn spawn_pty_process(
    shell: &str,
    shell_args: &[String],
    env: &[(&str, &str)],
    cols: usize,
    rows: usize,
    wake: EventLoopProxy<()>,
) -> Result<(Receiver<Vec<u8>>, Sender<PtyCommand>)> {
    let shell_refs: Vec<&str> = shell_args.iter().map(|s| s.as_str()).collect();
    let mut backend = match PortablePtyBackend::new() {
        Ok(b) => b,
        Err(e) => return Ok(spawn_err_channels(shell, e.to_string())),
    };
    let process = match backend.spawn(shell, &shell_refs, None, env) {
        Ok(p) => p,
        Err(e) => return Ok(spawn_err_channels(shell, e.to_string())),
    };
    let (reader, mut process) = match process.split_reader() {
        Ok(r) => r,
        Err(e) => return Ok(spawn_err_channels(shell, e.to_string())),
    };
    if let Err(e) = process.resize(PtySize { cols, rows }) {
        return Ok(spawn_err_channels(shell, e.to_string()));
    }

    let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
    let (pty_tx, input_rx) = mpsc::channel::<PtyCommand>();

    // Dedicated reader thread: forward PTY output to the parser without ever
    // blocking the command channel (fixes the PTY I/O deadlock where a
    // blocking read starved pending keystrokes).
    std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = reader;
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if output_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                    let _ = wake.send_event(());
                }
                Err(_) => break,
            }
        }
    });

    // Command thread: write/resize/kill only; blocks on recv, never on PTY.
    std::thread::spawn(move || {
        while let Ok(cmd) = input_rx.recv() {
            match cmd {
                PtyCommand::Write(data) => {
                    let _ = process.write(&data);
                }
                PtyCommand::Resize(size) => {
                    let _ = process.resize(size);
                }
                PtyCommand::Kill => {
                    let _ = process.kill();
                    return;
                }
            }
        }
        let _ = process.kill();
    });

    Ok((pty_rx, pty_tx))
}

/// Degraded spawn: return a fake channel that delivers one ANSI error message
/// and swallows all commands, so panes render "failed to spawn" instead of
/// dying silently. All `spawn_pty_process` call sites keep working unchanged.
fn spawn_err_channels(shell: &str, err: String) -> (Receiver<Vec<u8>>, Sender<PtyCommand>) {
    let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
    let msg = format!(
        "\x1b[31m[zeroterm] failed to spawn shell '{}': {}\x1b[0m\r\n",
        shell, err
    );
    let _ = output_tx.send(msg.into_bytes());
    drop(output_tx);
    let (pty_tx, _discard) = mpsc::channel::<PtyCommand>();
    (pty_rx, pty_tx)
}

#[cfg(all(unix, feature = "ssh"))]
#[allow(clippy::too_many_arguments)]
pub fn spawn_ssh_process(
    host: &str,
    port: u16,
    user: &str,
    password: Option<&str>,
    key_path: Option<&Path>,
    cols: usize,
    rows: usize,
    wake: EventLoopProxy<()>,
) -> Result<(Receiver<Vec<u8>>, Sender<PtyCommand>)> {
    let host = host.to_string();
    let user = user.to_string();
    let password = password.map(|s| s.to_string());
    let key_path = key_path.map(|p| p.to_path_buf());

    let (output_tx, pty_rx) = mpsc::channel::<Vec<u8>>();
    let (pty_tx, input_rx) = mpsc::channel::<PtyCommand>();

    std::thread::spawn(move || {
        let mut ssh = zeroterm_ssh::client::SshSession::new();
        if let Err(e) = ssh.connect(&host, port, &user, password.as_deref(), key_path.as_deref()) {
            let _ = output_tx.send(format!("\r\n\x1b[31mSSH: {}\x1b[0m\r\n", e).into_bytes());
            let _ = wake.send_event(());
            return;
        }
        if let Err(e) = ssh.resize(cols as u32, rows as u32) {
            let _ =
                output_tx.send(format!("\r\n\x1b[31mSSH resize: {}\x1b[0m\r\n", e).into_bytes());
            let _ = wake.send_event(());
        }
        // An idle SSH channel must not starve keystrokes, so bound the read
        // (mirrors the PTY fix: no blocking call ever holds up the command
        // channel).
        ssh.set_timeout(50);

        let mut buf = [0u8; 65536];
        loop {
            while let Ok(cmd) = input_rx.try_recv() {
                match cmd {
                    PtyCommand::Write(data) => {
                        if ssh.write(&data).is_err() {
                            return;
                        }
                    }
                    PtyCommand::Resize(size) => {
                        let _ = ssh.resize(size.cols as u32, size.rows as u32);
                    }
                    PtyCommand::Kill => {
                        let _ = ssh.disconnect();
                        return;
                    }
                }
            }
            match ssh.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if output_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                    let _ = wake.send_event(());
                }
                Err(e) => match e.downcast_ref::<std::io::Error>() {
                    Some(ioe)
                        if ioe.kind() == std::io::ErrorKind::WouldBlock
                            || ioe.kind() == std::io::ErrorKind::TimedOut => {}
                    _ => break,
                },
            }
        }
        let _ = ssh.disconnect();
    });

    Ok((pty_rx, pty_tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_err_channels_delivers_ansi_error_and_swallows_commands() {
        let (rx, tx) = spawn_err_channels("definitely-not-a-shell", "no such file".into());
        let chunk = rx.recv().unwrap();
        let text = String::from_utf8(chunk).unwrap();
        assert!(text.contains("failed to spawn shell 'definitely-not-a-shell'"));
        assert!(text.contains("no such file"));
        assert!(text.starts_with("\x1b[31m"));
        assert!(
            rx.recv().is_err(),
            "channel closes after the single error chunk"
        );

        // Commands go nowhere: the fake channel's command receiver is dropped.
        assert!(tx.send(PtyCommand::Write(vec![b'x'])).is_err());
        assert!(tx.send(PtyCommand::Kill).is_err());
    }
}
