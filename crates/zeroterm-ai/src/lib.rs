//! ZeroTerm AI - Local AI integration (Ollama/LM Studio)

pub mod client;
pub use client::{AiClient, AiError};

#[cfg(test)]
mod tests;