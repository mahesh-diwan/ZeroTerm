use mlua::Lua;
use std::collections::HashMap;

#[allow(dead_code)]
pub struct LuaEngine {
    overrides: HashMap<String, String>,
}

impl LuaEngine {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            overrides: HashMap::new(),
        })
    }

    pub fn evaluate(path: &str) -> Result<HashMap<String, String>, anyhow::Error> {
        let code = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Ok(HashMap::new()),
        };

        let lua = Lua::new();
        let overrides = lua.create_table()?;

        let ov = overrides.clone();
        let set_fn = lua.create_function(move |_, (key, value): (String, String)| {
            ov.set(key.clone(), value)?;
            Ok(())
        })?;
        lua.globals().set("set", set_fn)?;

        lua.globals().set("font_size", 14.0)?;
        lua.globals().set("line_height", 1.2)?;
        lua.globals().set("opacity", 1.0)?;
        lua.globals().set("theme", "tokyo-night")?;

        lua.load(&code).exec()?;

        let mut result = HashMap::new();
        for pair in overrides.pairs::<String, String>() {
            let (k, v) = pair?;
            result.insert(k, v);
        }
        Ok(result)
    }
}
