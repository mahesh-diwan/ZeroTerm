//! Env-gated diagnostics (`ZTDIAG=1`) for attributing blank-window / broken
//! rendering reports. The renderer used to hand-roll this: scattered
//! `eprintln!` probes plus a ~60-line frame readback (copy → map → poll →
//! PPM encode) inside `end_frame`. `Diag` owns the gate and the readback so
//! the renderer keeps a single call per concern.
//!
//! Nothing here runs unless `ZTDIAG=1` is set; with the env var absent every
//! method is a no-op and no buffers are created.

pub(crate) struct Diag {
    enabled: bool,
    frames: u32,
    dump_every: u32,
}

/// A frame readback queued inside the render encoder (before submit) and
/// finalized after submit, once the buffer is populated.
pub(crate) struct DiagReadback {
    buffer: wgpu::Buffer,
    bytes_per_row: u32,
    width: u32,
    height: u32,
}

impl Diag {
    pub(crate) fn new() -> Self {
        Self {
            enabled: std::env::var("ZTDIAG").is_ok(),
            frames: 0,
            dump_every: 30,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Tagged probe line, e.g. `Diag::probe("render_screen", &format!(...))`.
    /// No-op when the gate is off.
    pub(crate) fn probe(&self, tag: &str, msg: &str) {
        if self.enabled {
            eprintln!("[ZTDIAG] {tag}: {msg}");
        }
    }

    /// Advance the frame counter; true when this frame should be dumped
    /// (gated and `frame % dump_every == 0`).
    pub(crate) fn should_dump_frame(&mut self) -> bool {
        self.frames = self.frames.wrapping_add(1);
        self.enabled && self.frames.is_multiple_of(self.dump_every)
    }

    /// Queue a copy of the presented frame into a CPU-readable buffer. Must be
    /// called inside the frame's command encoder, before submit. Returns None
    /// if the frame geometry is degenerate or the copy can't be queued.
    pub(crate) fn queue_readback(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::SurfaceTexture,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Option<DiagReadback> {
        if width == 0 || height == 0 {
            return None;
        }
        let bytes_per_row = (width as usize * 4 + 255) & !255;
        let rb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zt-diag-readback"),
            size: (bytes_per_row as u64) * (height as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &frame.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &rb,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        // Keep a record of the surface format for the PPM writer's own use.
        let _ = std::fs::write(
            "/tmp/zt-frame-format.txt",
            format!("{format:?}\n{width}x{height}"),
        );
        Some(DiagReadback {
            buffer: rb,
            bytes_per_row: bytes_per_row as u32,
            width,
            height,
        })
    }

    /// After `queue.submit`, map the readback and write `/tmp/zt-frame.ppm`
    /// plus an alpha histogram (`/tmp/zt-frame-alpha.txt`). The alpha
    /// histogram answers the classic "stripes through the window" question:
    /// all-255 means translucency is compositor-side, not our framebuffer.
    pub(crate) fn finalize_readback(device: &wgpu::Device, rb: DiagReadback) {
        let slice = rb.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r.is_ok());
        });
        device.poll(wgpu::Maintain::Wait);
        // Only touch the mapped range if map_async actually succeeded; a
        // failed map would panic on get_mapped_range.
        if !rx.recv().unwrap_or(false) {
            return;
        }
        let data = slice.get_mapped_range();
        let w = rb.width as usize;
        let h = rb.height as usize;
        let bpr = rb.bytes_per_row as usize;
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        let mut alpha_counts: std::collections::HashMap<u8, u32> = std::collections::HashMap::new();
        for row in 0..h {
            let base = row * bpr;
            for col in 0..w {
                let i = base + col * 4;
                ppm.push(data[i]);
                ppm.push(data[i + 1]);
                ppm.push(data[i + 2]);
                *alpha_counts.entry(data[i + 3]).or_insert(0) += 1;
            }
        }
        let _ = std::fs::write("/tmp/zt-frame.ppm", &ppm);
        let mut alphas: Vec<_> = alpha_counts.into_iter().collect();
        alphas.sort_by_key(|a| std::cmp::Reverse(a.1));
        let mut alpha_txt = String::new();
        for (a, n) in alphas.into_iter().take(6) {
            alpha_txt += &format!("alpha={a} count={n}\n");
        }
        let _ = std::fs::write("/tmp/zt-frame-alpha.txt", &alpha_txt);
    }
}
