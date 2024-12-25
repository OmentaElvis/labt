use std::process::Command;

use anyhow::Context;
use mlua::{IntoLuaMulti, Lua, MultiValue, Table};

use crate::get_project_root;

use super::MluaAnyhowWrapper;

fn exec_command<'lua>(
    lua: &'lua Lua,
    cmd: &str,
    args: MultiValue,
) -> mlua::Result<MultiValue<'lua>> {
    let mut cmd = Command::new(cmd);
    cmd.current_dir(
        get_project_root()
            .context("Failed to get project root.")
            .map_err(MluaAnyhowWrapper::external)?,
    );
    for arg in args {
        cmd.arg(arg.to_string()?);
    }
    let status = cmd.status()?;

    (status.success(), status.code()).into_lua_multi(lua)
}

fn exec_command_with_output<'lua>(
    lua: &'lua Lua,
    cmd: &str,
    args: MultiValue,
) -> mlua::Result<MultiValue<'lua>> {
    let mut cmd = Command::new(cmd);
    cmd.current_dir(
        get_project_root()
            .context("Failed to get project root.")
            .map_err(MluaAnyhowWrapper::external)?,
    );
    for arg in args {
        cmd.arg(arg.to_string()?);
    }
    let out = cmd.output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    (out.status.success(), stdout, stderr).into_lua_multi(lua)
}

/// Generates sys table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to sys table fails
/// or the underlying lua operations return errors.
pub fn load_sys_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;

    // Metatables
    let exec = lua.create_function(move |lua, (_table, key): (Table, String)| {
        // A very crude safety checking for command
        if key.contains('/') || key.contains('\\') {
            return Err(mlua::Error::external(format!(
                "Invalid symbols contained in key {}.",
                key
            )));
        }
        lua.create_function(move |lua, args: MultiValue| {
            let key = key.clone();
            // The caller of this function requires stdout
            let (key, needs_output) = if key.starts_with("get_") {
                (key.strip_prefix("get_").unwrap(), true)
            } else {
                (key.as_str(), false)
            };

            if needs_output {
                exec_command_with_output(lua, key, args)
            } else {
                exec_command(lua, key, args)
            }
        })
    })?;
    let meta = lua.create_table()?;
    meta.set("__index", exec)?;
    table.set_metatable(Some(meta));
    lua.globals().set("sys", table)?;

    Ok(())
}
