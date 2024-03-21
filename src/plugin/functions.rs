use anyhow::Result;
use labt_proc_macro::labt_lua;
use mlua::IntoLua;
use mlua::Lua;
use mlua::LuaSerdeExt;

use crate::config::get_config;
use crate::config::lock::load_lock_dependencies;
use crate::config::lock::strings::ARTIFACT_ID;
use crate::config::lock::strings::DEPENDENCIES;
use crate::config::lock::strings::GROUP_ID;
use crate::config::lock::strings::VERSION;
use crate::submodules::build::Step;
use crate::submodules::build::BUILD_STEP;

/// Generates labt table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to labt function fails
/// or the underlying lua operations return errors.
pub fn load_labt_table(lua: &mut Lua) -> Result<()> {
    let table = lua.create_table()?;

    // add get_stage, returns the current stage of the build
    get_build_step(lua, &table)?;

    // add get_project_config
    get_project_config(lua, &table)?;
    get_project_root(lua, &table)?;

    // add get_dependencies
    get_lock_dependencies(lua, &table)?;

    lua.globals().set("labt", table)?;

    Ok(())
}

/// Returns the current build step the plugin was executed
#[labt_lua]
fn get_build_step(_: &Lua) {
    Ok(BUILD_STEP.with(|step| *step.borrow()))
}

#[labt_lua]
fn get_project_config(lua: &Lua) {
    let config = get_config().map_err(mlua::Error::external)?;
    lua.to_value(&config)
}

/// Returns the project root directory
#[labt_lua]
fn get_project_root(lua: &Lua) {
    let path = crate::get_project_root()
        .map_err(mlua::Error::external)?
        .clone();
    Ok(lua.to_value(&path))
}

#[labt_lua]
fn get_lock_dependencies(lua: &Lua) {
    // TODO cache this to reduce uneccessary reading of Labt.lock

    let deps = load_lock_dependencies().map_err(mlua::Error::external)?;
    let array = lua.create_table_with_capacity(deps.len(), 0)?;

    for dep in deps {
        let dep_table = lua.create_table()?;
        dep_table.set(ARTIFACT_ID, dep.artifact_id)?;
        dep_table.set(GROUP_ID, dep.group_id)?;
        dep_table.set(VERSION, dep.version)?;
        dep_table.set(DEPENDENCIES, dep.dependencies)?;
        array.push(dep_table)?;
    }

    Ok(array)
}

impl<'lua> IntoLua<'lua> for Step {
    fn into_lua(
        self,
        lua: &'lua mlua::prelude::Lua,
    ) -> mlua::prelude::LuaResult<mlua::prelude::LuaValue<'lua>> {
        match self {
            Self::PRE => Ok(mlua::Value::String(lua.create_string("PRE")?)),
            Self::AAPT => Ok(mlua::Value::String(lua.create_string("AAPT")?)),
            Self::COMPILE => Ok(mlua::Value::String(lua.create_string("COMPILE")?)),
            Self::DEX => Ok(mlua::Value::String(lua.create_string("DEX")?)),
            Self::BUNDLE => Ok(mlua::Value::String(lua.create_string("BUNDLE")?)),
            Self::POST => Ok(mlua::Value::String(lua.create_string("POST")?)),
        }
    }
}
