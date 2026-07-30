use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

pub struct SshSession {
    session: Option<ssh2::Session>,
    channel: Option<ssh2::Channel>,
}

impl SshSession {
    pub fn new() -> Self {
        Self {
            session: None,
            channel: None,
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
        let tcp = TcpStream::connect(format!("{}:{}", host, port))
            .with_context(|| format!("failed to connect to {}:{}", host, port))?;

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
            session
                .userauth_agent(user)
                .context("agent authentication failed")?;
        }

        if !session.authenticated() {
            anyhow::bail!("SSH authentication failed for {}@{}", user, host);
        }

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
}
