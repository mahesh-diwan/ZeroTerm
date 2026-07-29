//! Lua scripting support (placeholder)

pub struct LuaEngine;

impl LuaEngine {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self)
    }

    pub fn load_config(&mut self, _path: &str) -> Result<(), anyhow::Error> {
        // TODO: Implement Lua config loading
        Ok(())
    }
}
