use std::path::PathBuf;

use anyhow::Result;
use mlua::IntoLua;
use mlua::Lua;
use mlua::LuaSerdeExt;

use crate::config::lock::load_lock_dependencies;
use crate::config::lock::strings::ARTIFACT_ID;
use crate::config::lock::strings::DEPENDENCIES;
use crate::config::lock::strings::GROUP_ID;
use crate::config::lock::strings::VERSION;
use crate::config::{get_config, LabToml};
use crate::submodules::build::Step;
use crate::submodules::build::BUILD_STEP;
use crate::submodules::resolve::ProjectDep;

pub fn load_labt_table(lua: &mut Lua) -> Result<()> {
    let table = lua.create_table()?;

    // add get_stage, returns the current stage of the build
    let get_build_step = lua.create_function(move |_, ()| Ok(get_build_step()))?;
    table.set("get_build_step", get_build_step)?;

    // add get_project_config
    let get_project_config = lua.create_function(move |lua, ()| {
        let config = get_project_config().map_err(mlua::Error::external)?;
        Ok(lua.to_value(&config))
    })?;
    table.set("get_project_config", get_project_config)?;

    let get_project_root = lua.create_function(move |lua, ()| {
        let path = get_project_root().map_err(mlua::Error::external)?;
        Ok(lua.to_value(&path))
    })?;
    table.set("get_project_root", get_project_root)?;

    // add get_dependencies
    // TODO cache this to reduce uneccessary reading of Labt.lock
    let get_lock_dependencies = lua.create_function(move |lua, ()| {
        let deps = get_lock_dependencies().map_err(mlua::Error::external)?;
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
    })?;

    table.set("get_lock_dependencies", get_lock_dependencies)?;

    lua.globals().set("labt", table)?;

    Ok(())
}

fn get_build_step() -> Step {
    BUILD_STEP.with(|step| *step.borrow())
}

fn get_project_config() -> anyhow::Result<LabToml> {
    get_config()
}
/// Returns the project root directory
fn get_project_root() -> anyhow::Result<PathBuf> {
    Ok(crate::get_project_root()?.clone())
}

fn get_lock_dependencies() -> anyhow::Result<Vec<ProjectDep>> {
    load_lock_dependencies()
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
