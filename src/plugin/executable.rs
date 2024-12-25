use std::fs::read_to_string;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use anyhow::{anyhow, Context, Result};
use mlua::{Chunk, IntoLuaMulti, Lua, MultiValue, Table, Value};

use crate::get_project_root;
use crate::submodules::build::Step;
use crate::submodules::sdk::toml_strings::REPOSITORY_NAME;
use crate::submodules::sdk::{get_sdk_path, InstalledPackage};
use crate::submodules::sdkmanager::ToId;

use super::api::fs::load_fs_table;
use super::api::labt::load_labt_table;
use super::api::log::load_log_table;
use super::api::sys::load_sys_table;
use super::api::zip::load_zip_table;
use super::api::MluaAnyhowWrapper;
use super::config::{SdkEntry, CHANNEL, PATH, VERSION};
use super::get_installed_list_hash;

const PREFIX: &str = "sdk:";

/// Represents an executable plugin
pub struct Executable {}
impl Executable {
    pub fn main(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct ExecutableLua {
    build_step: Step,
    lua: Lua,
    path: PathBuf,
    package_paths: String,
    sdk: Rc<Vec<SdkEntry>>,
}

impl<'lua, 'a> ExecutableLua {
    pub fn new(
        path: PathBuf,
        package_paths: &[PathBuf],
        sdk: Rc<Vec<SdkEntry>>,
        unsafe_mode: bool,
    ) -> Self {
        let lua = if unsafe_mode {
            unsafe { Lua::unsafe_new() }
        } else {
            Lua::new()
        };
        let paths: String = package_paths
            .iter()
            .filter_map(|p| p.to_str())
            .collect::<Vec<&str>>()
            .join(";");

        ExecutableLua {
            lua,
            path,
            build_step: Step::PRE,
            package_paths: paths,
            sdk,
        }
    }
    pub fn load(&'lua self) -> Result<Chunk<'lua, 'a>> {
        let lua_string =
            read_to_string(&self.path).context(format!("Failed to read {:?}", self.path))?;

        // add default paths and those defined by the plugin

        let chunk = self
            .lua
            .load(lua_string)
            .set_name(self.path.to_str().unwrap_or("[unknown]"));

        let globs = &self.lua.globals();
        let package: Table = globs
            .get("package")
            .context("Failed to get package table from lua global context")?;
        let package_path: String = package
            .get("path")
            .context("Failed to get package.path from lua global context")?;

        let package_path = package_path.trim_end_matches(';').to_string();

        let package_path = [package_path, self.package_paths.clone()].join(";");
        package
            .set("path", package_path)
            .context("Failed to set package.path in lua global context")?;

        Ok(chunk)
    }
    pub fn get_build_step(&self) -> Step {
        self.build_step
    }
    pub fn set_build_step(&mut self, stage: Step) {
        self.build_step = stage;
    }

    /// Builds the package directory if not already installed
    fn get_package_directory(package: &InstalledPackage) -> anyhow::Result<PathBuf> {
        if let Some(dir) = &package.directory {
            return Ok(dir.clone());
        }

        let mut sdk = get_sdk_path()?;
        sdk.push(&package.repository_name);
        let sdk = sdk.join(package.path.split(';').collect::<PathBuf>());

        Ok(sdk)
    }
    /// Fires the command and lets the output go where outputs go, stdout or stderr
    /// Only returns if success and exit code
    fn exec_sdk_command(
        lua: &'lua Lua,
        args: MultiValue,
        _package: &InstalledPackage,
        dir: PathBuf,
    ) -> mlua::Result<MultiValue<'lua>> {
        let mut cmd = Command::new(dir);
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
    /// Fires the given command and returns output to lua
    fn exec_sdk_command_with_output(
        lua: &'lua Lua,
        args: MultiValue,
        _package: &InstalledPackage,
        dir: PathBuf,
    ) -> mlua::Result<MultiValue<'lua>> {
        let mut cmd = Command::new(dir);
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
    /// Constructs the module returned to lua with its injected metatables
    fn build_sdk_module(
        lua: &'lua Lua,
        module: String,
        id: String,
        sdk: Table<'lua>,
    ) -> mlua::Result<Table<'lua>> {
        let meta = lua.create_table()?;
        let installed_list = get_installed_list_hash().map_err(MluaAnyhowWrapper::external)?;
        let module = if module.starts_with(PREFIX) {
            module.strip_prefix(PREFIX).unwrap()
        } else {
            module.as_str()
        };

        let segments: PathBuf = module
            .split('/')
            .skip(1)
            .map(|s| s.trim_matches('.'))
            .collect();

        let package = if let Some(package) = installed_list.get(&id) {
            package
        } else {
            return Err(mlua::Error::external(format!(
                "Trying to index a package \"{}\" that is not installed.",
                id
            )));
        };
        // obtain directory of this package
        let dir = Self::get_package_directory(package)
            .context(format!(
                "Failed to obtain sdk install directory for package: {}",
                package.to_id()
            ))
            .map_err(MluaAnyhowWrapper::external)?
            .join(segments);

        // files function. Returns a full path to a given file name
        let fdir = dir.clone();
        let file_function = lua.create_function(move |_lua, name: String| {
            let mut fdir = fdir.clone(); // wat im willing to do to please the borrow checker :(
            if name.contains('/') || name.contains('\\') {
                return Err(mlua::Error::external(format!(
                    "Invalid symbols contained in file name {}.",
                    name
                )));
            }

            let name = name.trim_matches('.');
            fdir.push(name);

            if !fdir.exists() {
                return Err(mlua::Error::external("File not found."));
            }

            Ok(fdir.to_string_lossy().to_string())
        })?;

        // Metatables
        let exec = lua.create_function(move |lua, (_table, key): (Table, String)| {
            // A very crude safety checking for command
            if key.contains('/') || key.contains('\\') {
                return Err(mlua::Error::external(format!(
                    "Invalid symbols contained in key {}.",
                    key
                )));
            }
            let dir = dir.clone();
            lua.create_function(move |lua, args: MultiValue| {
                let key = key.clone();
                let mut dir = dir.clone();
                // The caller of this function requires stdout
                let (key, needs_output) = if key.starts_with("get_") {
                    (key.strip_prefix("get_").unwrap(), true)
                } else {
                    (key.as_str(), false)
                };

                dir.push(key);
                // no need to build a command if it is not present
                if !dir.exists() {
                    return Err(mlua::Error::external("Command not found."));
                }
                if needs_output {
                    Self::exec_sdk_command_with_output(lua, args, package, dir)
                } else {
                    Self::exec_sdk_command(lua, args, package, dir)
                }
            })
        })?;
        sdk.set("file", file_function)?;
        meta.set("__index", exec)?;
        sdk.set_metatable(Some(meta));
        Ok(sdk)
    }
    pub fn load_sdk_loader(&mut self) -> Result<()> {
        let installed_list = get_installed_list_hash()?;

        let sdk = Rc::clone(&self.sdk);
        let sdk_resolver = self
            .lua
            .create_function(move |lua, module: String| -> mlua::Result<Value>{
                // only resolve packages that begin with sdk.
                if module.starts_with(PREFIX) {
                    let module = module.strip_prefix(PREFIX).unwrap();
                    // extract the main seg after sdk: and before first /
                    let module = if let Some((main, _)) = module.split_once("/") {
                        main
                    } else {
                        module
                    };
                    // only allow explicitly declared sdk modules in plugin.toml
                    if let Some(sdk) = sdk.iter().find(|s| s.name.eq(module)).cloned() {
                        if let Some(package) = installed_list.get(&sdk.to_id()) {
                            Ok(
                                    Value::Function(lua.create_function(move |lua, module:String | {
                                        let table = lua.create_table()?;
                                        // only load the bare minimal required to index the hashmap
                                        table.set("name", sdk.name.clone())?;
                                        table.set(REPOSITORY_NAME, package.repository_name.to_string())?;
                                        table.set(VERSION, package.version.to_string())?;
                                        table.set(PATH, package.path.clone())?;
                                        table.set(CHANNEL, package.channel.to_string())?;
                                        Self::build_sdk_module(lua, module, package.to_id(), table)
                                    })?),
                            )
                        } else {
                            Err(MluaAnyhowWrapper::external(anyhow!(
                                "Sdk package {} is not installed.",
                                sdk.to_id()
                            )))
                        }
                    } else {
                        Err(MluaAnyhowWrapper::external(anyhow!(format!("Trying to require undeclared sdk ({}) in plugin.toml",
                            module))))
                    }
                } else {
                    Ok(
                        Value::String(lua.create_string("\n\tNot a sdk module name. Prefix with 'sdk:' if you intended to load a LABt android sdk module.")?)
                        )
                }
            })
            .context("Failed to create sdk loader function for lua package.searchers")?;

        let package: Table = self
            .lua
            .globals()
            .raw_get("package")
            .context("Failed to get lua package table.")?;
        let loaders: Table = package
            .raw_get("loaders")
            .context("Failed to get lua searchers table.")?;

        loaders
            .push(sdk_resolver)
            .context("Failed to push sdk loader to \"package.searchers\".")?;

        Ok(())
    }
    pub fn add_function(&mut self) {}
    pub fn load_api_tables(&mut self) -> Result<()> {
        load_labt_table(&mut self.lua).context("Failed to add labt table into lua context")?;
        load_fs_table(&mut self.lua).context("Failed to add fs table into lua context")?;
        load_log_table(&mut self.lua).context("Failed to add log table into lua context")?;
        load_zip_table(&mut self.lua).context("Failed to add zip table into lua context")?;
        load_sys_table(&mut self.lua).context("Failed to add sys table into lua context")?;
        Ok(())
    }
}
