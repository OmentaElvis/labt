use std::fs::create_dir;
use std::fs::create_dir_all;
use std::path::PathBuf;

use labt_proc_macro::labt_lua;
use mlua::Lua;

/// creates the directory specified
/// Returns en error if obtaining the project root directory fails or
/// creating the directory fails
#[labt_lua]
fn mkdir(_lua: &Lua, path: String) {
    let path = PathBuf::from(path);
    let path = if path.is_relative() {
        // if path is relative, then build from project root
        let mut root = crate::get_project_root()
            .map_err(mlua::Error::external)?
            .clone();
        root.push(path);
        root
    } else {
        path
    };

    create_dir(path).map_err(mlua::Error::external)?;

    Ok(())
}
/// creates the directory specified and all the parent directories if missing
/// Returns en error if obtaining the project root directory fails or
/// creating the directory fails
#[labt_lua]
fn mkdir_all(_lua: &Lua, path: String) {
    let path = PathBuf::from(path);
    let path = if path.is_relative() {
        // if path is relative, then build from project root
        let mut root = crate::get_project_root()
            .map_err(mlua::Error::external)?
            .clone();
        root.push(path);
        root
    } else {
        path
    };

    create_dir_all(path).map_err(mlua::Error::external)?;

    Ok(())
}
/// Generates labt table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to labt function fails
/// or the underlying lua operations return errors.
pub fn load_fs_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;

    mkdir(lua, &table)?;
    mkdir_all(lua, &table)?;

    lua.globals().set("fs", table)?;

    Ok(())
}
