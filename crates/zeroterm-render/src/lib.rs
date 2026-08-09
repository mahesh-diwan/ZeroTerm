//! ZeroTerm Renderer - wgpu-based GPU renderer

mod cell_batch;
mod diag;
mod pass;
pub mod atlas;
pub mod renderer;
pub mod theme;

pub use atlas::estimate_cell_size;
pub use renderer::{tab_span, Renderer, Selection, TabInfo};

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
