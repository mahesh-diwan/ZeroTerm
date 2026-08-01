//! Image decoding: PNG/GIF/WebP → RGBA frames (single or animated).

use std::io::Cursor;

use image::{AnimationDecoder, ImageReader};

/// Max frames kept for one animated image (subsequent frames are dropped).
pub const MAX_ANIM_FRAMES: usize = 100;

/// Matches screen::MAX_IMAGE_BYTES — pre-decode guard against hostile payloads.
const MAX_DECODE_BYTES: u64 = 100 << 20;

#[derive(Debug, Clone)]
pub struct FrameData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub delay_ms: u64,
}

#[derive(Debug, Clone)]
pub struct DecodedFrames {
    pub frames: Vec<FrameData>,
    pub is_animated: bool,
}

/// Decode image bytes into RGBA frames.
///
/// GIF is decoded frame-by-frame (fully composited, canvas-sized). PNG/WebP
/// yield a single frame; animated WebP is not exposed by the image crate's
/// decoder, so it degrades to its first (still) frame.
pub fn decode_frames(data: &[u8]) -> anyhow::Result<DecodedFrames> {
    let (w, h) = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .into_dimensions()?;
    if (w as u64) * (h as u64) * 4 > MAX_DECODE_BYTES {
        anyhow::bail!("image too large: {w}x{h}");
    }
    if data.starts_with(b"GIF8") {
        return decode_gif(data);
    }
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .decode()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(DecodedFrames {
        is_animated: false,
        frames: vec![FrameData {
            width,
            height,
            rgba: rgba.into_raw(),
            delay_ms: 0,
        }],
    })
}

fn decode_gif(data: &[u8]) -> anyhow::Result<DecodedFrames> {
    let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(data))?;
    let mut frames: Vec<FrameData> = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = if denom == 0 {
            0
        } else {
            numer as u64 / denom as u64
        };
        let rgba = frame.into_buffer();
        let (width, height) = rgba.dimensions();
        frames.push(FrameData {
            width,
            height,
            rgba: rgba.into_raw(),
            delay_ms,
        });
        if frames.len() >= MAX_ANIM_FRAMES {
            break;
        }
    }
    if frames.is_empty() {
        anyhow::bail!("no frames decoded from GIF");
    }
    Ok(DecodedFrames {
        is_animated: frames.len() > 1,
        frames,
    })
}
