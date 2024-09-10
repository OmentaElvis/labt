use anyhow::Context;
use labt_proc_macro::labt_lua;
use mlua::IntoLua;
use mlua::Lua;
use mlua::LuaSerdeExt;

use crate::caching::Cache;
use crate::config::get_config;
use crate::config::get_resolvers_from_config;
use crate::config::lock::load_labt_lock;
use crate::config::lock::strings::ARTIFACT_ID;
use crate::config::lock::strings::DEPENDENCIES;
use crate::config::lock::strings::GROUP_ID;
use crate::config::lock::strings::PACKAGING;
use crate::config::lock::strings::VERSION;
use crate::plugin::api::MluaAnyhowWrapper;
use crate::submodules::build::Step;
use crate::submodules::build::BUILD_STEP;
use crate::submodules::resolve::ProjectDep;

/// Returns the current build step the plugin was executed
#[labt_lua]
fn get_build_step(_: &Lua) {
    let build_step = BUILD_STEP.with(|step| *step.borrow());
    Ok(build_step)
}

#[labt_lua]
fn get_project_config(lua: &Lua) {
    let config = get_config().map_err(MluaAnyhowWrapper::external)?;
    lua.to_value(&config)
}

/// Returns the project root directory
#[labt_lua]
fn get_project_root(lua: &Lua) {
    let path = crate::get_project_root()
        .context("Failed to get project root directory")
        .map_err(MluaAnyhowWrapper::external)?
        .clone();
    Ok(lua.to_value(&path))
}

#[labt_lua]
fn get_lock_dependencies(lua: &Lua) {
    // TODO cache this to reduce uneccessary reading of Labt.lock

    let lock = load_labt_lock().map_err(MluaAnyhowWrapper::external)?;
    let deps = lock.resolved;
    let array = lua.create_table_with_capacity(deps.len(), 0)?;

    for dep in deps {
        let dep_table = lua.create_table()?;
        dep_table.set(ARTIFACT_ID, dep.artifact_id)?;
        dep_table.set(GROUP_ID, dep.group_id)?;
        dep_table.set(VERSION, dep.version)?;
        dep_table.set(DEPENDENCIES, dep.dependencies)?;
        dep_table.set(PACKAGING, dep.packaging)?;
        array.push(dep_table)?;
    }

    Ok(array)
}
/// Returns the cache location for this dependency. This does not check if the path
/// exists. It constructs a valid cache path according to the labt cache resolver.
/// Returns an error if:
///  - Labt home was not initialized
///  - Failed to convert path to its unicode string representation
#[labt_lua]
fn get_cache_path(
    _: &Lua,
    (group_id, artifact_id, version, packaging): (String, String, String, String),
) {
    let dep = ProjectDep {
        group_id: group_id.clone(),
        artifact_id: artifact_id.clone(),
        version: version.clone(),
        packaging: packaging.clone(),
        ..Default::default()
    };
    let mut cache = Cache::from(dep);
    cache
        .use_labt_home()
        .context("Failed to initialize cache path with labt home")
        .map_err(MluaAnyhowWrapper::external)?;

    let path = cache
        .get_path()
        .context(format!(
            "Failed to get cache path for {}:{}:{}",
            group_id, artifact_id, version
        ))
        .map_err(MluaAnyhowWrapper::external)?;

    let path_str = path
        .to_str()
        .context("Failed to convert path to string")
        .map_err(MluaAnyhowWrapper::external)?
        .to_string();

    Ok(path_str)
}

/// Calls dependency resolution algorithm on dependencies found in
/// Labt.toml
/// Returns an error if:
/// - resolving the dependencies fail
/// - failed to read project config [`Labt.toml`]
/// - failed to read and configure resolvers from config
#[labt_lua]
fn resolve(_lua: &Lua) {
    use crate::pom::Project;

    let config = get_config()
        .context("Failed to get project configuration")
        .map_err(MluaAnyhowWrapper::external)?;

    if let Some(deps) = &config.dependencies {
        let dependencies: Vec<Project> = deps
            .iter()
            .map(|(artifact_id, table)| Project::new(&table.group_id, artifact_id, &table.version))
            .collect();
        let resolvers = get_resolvers_from_config(&config)
            .context("Failed to get resolvers")
            .map_err(MluaAnyhowWrapper::external)?;

        crate::submodules::resolve::resolve(dependencies, resolvers)
            .context("Failed to resolve projects dependencies")
            .map_err(MluaAnyhowWrapper::external)?;
    }
    Ok(())
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
/// Generates labt table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to labt function fails
/// or the underlying lua operations return errors.
pub fn load_labt_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;

    // add get_stage, returns the current stage of the build
    get_build_step(lua, &table)?;

    // add get_project_config
    get_project_config(lua, &table)?;
    // add get_project_root
    get_project_root(lua, &table)?;

    // add get_dependencies
    get_lock_dependencies(lua, &table)?;

    get_cache_path(lua, &table)?;

    resolve(lua, &table)?;

    lua.globals().set("labt", table)?;

    Ok(())
}
