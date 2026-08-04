//! ZeroTerm Core - VT parser, screen buffer, cell model

pub mod cell;
pub mod highlight;
pub mod image_decode;
pub mod parser;
pub mod pty;
pub mod screen;

pub use parser::Parser;
pub use screen::Screen;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod unicode;
