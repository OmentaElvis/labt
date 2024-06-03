use std::fs::read_to_string;
use std::path::PathBuf;

use anyhow::{Context, Result};
use mlua::{Chunk, Lua, Table};

use crate::submodules::build::Step;

use super::api::fs::load_fs_table;
use super::api::labt::load_labt_table;
use super::api::log::load_log_table;
use super::api::zip::load_zip_table;

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
}

impl<'lua, 'a> ExecutableLua {
    pub fn new(path: PathBuf, package_paths: &[PathBuf]) -> Self {
        let lua = Lua::new();
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
    pub fn add_function(&mut self) {}
    pub fn load_api_tables(&mut self) -> Result<()> {
        load_labt_table(&mut self.lua).context("Failed to add labt table into lua context")?;
        load_fs_table(&mut self.lua).context("Failed to add fs table into lua context")?;
        load_log_table(&mut self.lua).context("Failed to add log table into lua context")?;
        load_zip_table(&mut self.lua).context("Failed to add zip table into lua context")?;
        Ok(())
    }
}
