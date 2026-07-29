//! ZeroTerm Config - TOML parser, Lua runtime, GUI settings

pub mod config;
pub mod gui;
pub mod lua;

pub use config::Config;

#[cfg(test)]
mod tests;
