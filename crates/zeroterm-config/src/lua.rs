use mlua::Lua;
use std::collections::HashMap;

/// Evaluate override `set("font_size", 16)` calls from the Lua config file at
/// `path` and return them as a map. A missing file yields an empty map (no
/// overrides). The Lua sandbox strips dangerous globals (`io`, `load`,
/// `require`, `package`, `debug`) and exposes a minimal `os` table
/// (`clock`, `time`) plus scalar globals consumed by apply_overrides.
pub fn evaluate(path: &str) -> Result<HashMap<String, String>, anyhow::Error> {
    let code = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(HashMap::new()),
    };

    let lua = Lua::new();

    // Sandbox: remove dangerous globals
    let globals = lua.globals();
    let _ = globals.raw_remove("io");
    let _ = globals.raw_remove("load");
    let _ = globals.raw_remove("loadfile");
    let _ = globals.raw_remove("dofile");
    let _ = globals.raw_remove("require");
    let _ = globals.raw_remove("package");
    let _ = globals.raw_remove("debug");
    // Safe os: only clock and time
    let safe_os = lua.create_table()?;
    safe_os.set(
        "clock",
        lua.create_function(|_, ()| {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64())
        })?,
    )?;
    safe_os.set(
        "time",
        lua.create_function(|_, ()| {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs())
        })?,
    )?;
    globals.set("os", safe_os)?;

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
