use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use crate::session::{detached_start_cmd, remote_attach_cmd, unix_now, SessionRegistry};
#[cfg(unix)]
use anyhow::Context;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::net::TcpStream;

/// One `Host <alias>` block from an ssh config file.
#[derive(Debug, Clone, Default)]
pub struct SshHostEntry {
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
}

pub fn default_ssh_config_path() -> PathBuf {
    std::env::var("HOME")
        .map(|h| Path::new(&h).join(".ssh").join("config"))
        .unwrap_or_default()
}

/// Sorted aliases from ~/.ssh/config, for host-picker UIs.
pub fn ssh_aliases() -> Vec<String> {
    let mut aliases: Vec<String> = parse_ssh_config(&default_ssh_config_path())
        .into_keys()
        .collect();
    aliases.sort();
    aliases
}

/// Parse ~/.ssh/config into alias -> entry. Missing file yields an empty map.
///
/// ponytail: naive line-by-line parser — only exact alias matches are
/// recognized; `Host *` patterns, globs, and `Match` blocks are ignored.
/// Upgrade to a pattern-matching parser if wildcard hosts matter.
pub fn parse_ssh_config(path: &Path) -> HashMap<String, SshHostEntry> {
    let mut map: HashMap<String, SshHostEntry> = HashMap::new();
    let Ok(contents) = std::fs::read_to_string(path) else {
        return map;
    };
    let mut current: Option<String> = None;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(keyword), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => current = Some(value.to_string()),
            "hostname" => {
                if let Some(alias) = &current {
                    map.entry(alias.clone()).or_default().hostname = Some(value.to_string());
                }
            }
            "user" => {
                if let Some(alias) = &current {
                    map.entry(alias.clone()).or_default().user = Some(value.to_string());
                }
            }
            "port" => {
                if let Some(alias) = &current {
                    map.entry(alias.clone()).or_default().port = value.parse().ok();
                }
            }
            "identityfile" => {
                if let Some(alias) = &current {
                    map.entry(alias.clone()).or_default().identity_file = Some(expand_home(value));
                }
            }
            _ => {}
        }
    }
    map
}

fn expand_home(path: &str) -> String {
    let p = path.trim_matches('"');
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    p.to_string()
}

#[cfg(unix)]
pub struct SshSession {
    session: Option<ssh2::Session>,
    channel: Option<ssh2::Channel>,
    registry: SessionRegistry,
    host: Option<String>,
}

/// Non-unix stub so the crate compiles on Windows/macOS-other targets with an
/// identical API surface. connect() always fails; the rest are no-ops.
#[cfg(not(unix))]
pub struct SshSession;

impl Default for SshSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(unix)]
impl SshSession {
    pub fn new() -> Self {
        Self {
            session: None,
            channel: None,
            registry: SessionRegistry::new(),
            host: None,
        }
    }

    pub fn connect(
        &mut self,
        host: &str,
        port: u16,
        user: &str,
        password: Option<&str>,
        key_path: Option<&Path>,
    ) -> Result<()> {
        // Resolve ~/.ssh/config alias overrides. Config values win over call
        // args, mirroring ssh(1).
        let entry = parse_ssh_config(&default_ssh_config_path())
            .get(host)
            .cloned();
        let hostname = entry
            .as_ref()
            .and_then(|e| e.hostname.as_deref())
            .unwrap_or(host);
        let user = entry
            .as_ref()
            .and_then(|e| e.user.as_deref())
            .unwrap_or(user);
        let port = entry.as_ref().and_then(|e| e.port).unwrap_or(port);
        let key_path = entry
            .as_ref()
            .and_then(|e| e.identity_file.as_deref())
            .map(Path::new)
            .or(key_path);

        let tcp = TcpStream::connect(format!("{}:{}", hostname, port))
            .with_context(|| format!("failed to connect to {}:{}", hostname, port))?;

        let mut session = ssh2::Session::new().context("failed to create SSH session")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("SSH handshake failed")?;

        if let Some(pw) = password {
            session
                .userauth_password(user, pw)
                .context("password authentication failed")?;
        } else if let Some(kp) = key_path {
            session
                .userauth_pubkey_file(user, None, kp, None)
                .context("public key authentication failed")?;
        } else {
            // Local ssh-agent auth. ponytail: libssh2 has no agent-forwarding
            // channel, so the agent is reachable from this host only; forwarding
            // it into the remote session is unsupported.
            session
                .userauth_agent(user)
                .context("agent authentication failed")?;
        }

        if !session.authenticated() {
            anyhow::bail!("SSH authentication failed for {}@{}", user, host);
        }

        self.host = Some(host.to_string());

        let mut channel = session
            .channel_session()
            .context("failed to open SSH channel")?;
        channel
            .exec(&std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()))
            .context("failed to exec shell on SSH channel")?;

        self.session = Some(session);
        self.channel = Some(channel);
        Ok(())
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        if let Some(ref mut ch) = self.channel {
            ch.write_all(data)?;
            ch.flush()?;
        }
        Ok(())
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if let Some(ref mut ch) = self.channel {
            let n = ch.read(buf)?;
            Ok(n)
        } else {
            Ok(0)
        }
    }

    pub fn resize(&mut self, cols: u32, rows: u32) -> Result<()> {
        if let Some(ref mut ch) = self.channel {
            ch.request_pty_size(cols, rows, None, None)?;
        }
        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<()> {
        if let Some(mut ch) = self.channel.take() {
            let _ = ch.send_eof();
            let _ = ch.wait_close();
        }
        drop(self.session.take());
        Ok(())
    }

    /// Bound the blocking read/write calls so an idle SSH channel returns
    /// WouldBlock/TimedOut instead of hanging the caller forever.
    pub fn set_timeout(&mut self, timeout_ms: u32) {
        if let Some(ref session) = self.session {
            session.set_timeout(timeout_ms);
        }
    }

    /// Start `command` in a detached remote session and return its id. The work
    /// is delegated to a remote `tmux` server (see [`crate::session`]), so the
    /// session survives this client disconnecting. Runs on a throwaway channel
    /// so the interactive shell channel is left untouched.
    pub fn daemon_start(&mut self, command: &str) -> Result<String> {
        let session = self
            .session
            .as_ref()
            .context("SSH session is not connected")?;
        let id = format!("zt-{}", unix_now());
        let host = self.host.clone().unwrap_or_default();
        self.registry.create(id.clone(), host, command.to_string());
        let mut channel = session
            .channel_session()
            .context("failed to open SSH channel")?;
        channel
            .exec(&detached_start_cmd(&id, command))
            .context("failed to exec detached-start command")?;
        let _ = channel.send_eof();
        let _ = channel.wait_close();
        Ok(id)
    }

    /// Re-attach to a detached remote session by id. libssh2 cannot exec a
    /// second command on an already-exec'd channel, so a new channel is opened
    /// running the remote attach command and replaces the current one.
    pub fn daemon_attach(&mut self, id: &str) -> Result<()> {
        if !self.registry.has(id) {
            anyhow::bail!("unknown detached session '{id}'");
        }
        let session = self
            .session
            .as_ref()
            .context("SSH session is not connected")?;
        let mut channel = session
            .channel_session()
            .context("failed to open SSH channel")?;
        channel
            .exec(&remote_attach_cmd(id))
            .context("failed to exec re-attach command")?;
        self.channel = Some(channel);
        Ok(())
    }

    pub fn registry(&self) -> &SessionRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut SessionRegistry {
        &mut self.registry
    }
}

#[cfg(not(unix))]
impl SshSession {
    pub fn new() -> Self {
        Self
    }

    pub fn connect(
        &mut self,
        _host: &str,
        _port: u16,
        _user: &str,
        _password: Option<&str>,
        _key_path: Option<&Path>,
    ) -> Result<()> {
        anyhow::bail!("SSH is not supported on this platform")
    }

    pub fn write(&mut self, _data: &[u8]) -> Result<()> {
        Ok(())
    }

    pub fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    pub fn resize(&mut self, _cols: u32, _rows: u32) -> Result<()> {
        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    pub fn set_timeout(&mut self, _timeout_ms: u32) {}

    pub fn daemon_start(&mut self, _command: &str) -> Result<String> {
        anyhow::bail!("SSH is not supported on this platform")
    }

    pub fn daemon_attach(&mut self, _id: &str) -> Result<()> {
        anyhow::bail!("SSH is not supported on this platform")
    }
}
