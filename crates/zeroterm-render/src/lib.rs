//! ZeroTerm Renderer - wgpu-based GPU renderer

pub mod renderer;

pub use renderer::{Renderer, Selection};

pub struct RenderConfig {
    pub font_size: f32,
    pub cell_width: f32,
    pub line_height: f32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            cell_width: 1.0,
            line_height: 1.2,
        }
    }
}