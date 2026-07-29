//! PTY abstraction layer

use crate::screen::Size;
use anyhow::Result;
use portable_pty::{CommandBuilder, PtySize, PtySystem};

pub trait PtyBackend: Send {
    fn spawn(
        &mut self,
        program: &str,
        args: &[&str],
        cwd: Option<&str>,
    ) -> Result<Box<dyn PtyProcess>>;
    fn resize(&mut self, size: Size) -> Result<()>;
}

pub trait PtyProcess: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn resize(&mut self, size: Size) -> Result<()>;
    fn wait(&mut self) -> Result<i32>;
    fn kill(&mut self) -> Result<()>;
}

pub struct PortablePtyBackend {
    system: Box<dyn PtySystem + Send>,
}

impl PortablePtyBackend {
    pub fn new() -> Result<Self> {
        let system = portable_pty::native_pty_system();
        Ok(Self { system })
    }
}

impl PtyBackend for PortablePtyBackend {
    fn spawn(
        &mut self,
        program: &str,
        args: &[&str],
        cwd: Option<&str>,
    ) -> Result<Box<dyn PtyProcess>> {
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }

        let pair = self.system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let child = pair.slave.spawn_command(cmd)?;
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        Ok(Box::new(PortablePtyProcess {
            reader,
            writer,
            child: Some(child),
            master: pair.master,
        }))
    }

    fn resize(&mut self, _size: Size) -> Result<()> {
        // Resize is handled on the process level
        Ok(())
    }
}

pub struct PortablePtyProcess {
    reader: Box<dyn std::io::Read + Send>,
    writer: Box<dyn std::io::Write + Send>,
    child: Option<Box<dyn portable_pty::Child + Send>>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

impl PtyProcess for PortablePtyProcess {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        use std::io::Read;
        self.reader.read(buf).map_err(Into::into)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        use std::io::Write;
        self.writer.write(buf).map_err(Into::into)
    }

    fn resize(&mut self, size: Size) -> Result<()> {
        self.master.resize(PtySize {
            rows: size.rows as u16,
            cols: size.cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    fn wait(&mut self) -> Result<i32> {
        if let Some(mut child) = self.child.take() {
            let status = child.wait()?;
            let code = status.exit_code();
            Ok(code as i32)
        } else {
            Ok(0)
        }
    }

    fn kill(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            child.kill().map_err(Into::into)
        } else {
            Ok(())
        }
    }
}
