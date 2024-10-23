use labt_proc_macro::labt_lua;
use mlua::{Lua, Value};

/// Logs a message at the info level
#[labt_lua]
fn info(_: &Lua, (target, message): (String, String)) {
    log::info!(target: target.as_str(), "{}", message);
    Ok(())
}
/// Logs a message at the warn level
#[labt_lua]
fn warn(_: &Lua, (target, message): (String, String)) {
    log::warn!(target: target.as_str(), "{}", message);
    Ok(())
}
/// Logs a message at the error level
#[labt_lua]
fn error(_: &Lua, (target, message): (String, String)) {
    log::error!(target: target.as_str(), "{}", message);
    Ok(())
}
/// Dumps a lua table for debugging
#[labt_lua]
fn dump(_lua: &Lua, table: Value) {
    println!("{:#?}", table);
    Ok(())
}

/// Generates log table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to log table fails
/// or the underlying lua operations return errors.
pub fn load_log_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;

    info(lua, &table)?;
    error(lua, &table)?;
    warn(lua, &table)?;
    dump(lua, &table)?;

    lua.globals().set("log", table)?;

    Ok(())
}
