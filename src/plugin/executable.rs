use std::fs::read_to_string;
use std::path::PathBuf;

use anyhow::{Context, Result};
use mlua::{Chunk, Lua};

use crate::submodules::build::Step;

use super::functions::load_labt_table;

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
}

impl<'lua, 'a> ExecutableLua {
    pub fn new(path: PathBuf) -> Self {
        ExecutableLua {
            lua: Lua::new(),
            path,
            build_step: Step::PRE,
        }
    }
    pub fn load(&'lua self) -> Result<Chunk<'lua, 'a>> {
        let lua_string =
            read_to_string(&self.path).context(format!("Failed to read {:?}", self.path))?;

        let chunk = self
            .lua
            .load(lua_string)
            .set_name(self.path.to_str().unwrap_or("[unknown]"));
        Ok(chunk)
    }
    pub fn get_build_step(&self) -> Step {
        self.build_step
    }
    pub fn set_build_step(&mut self, stage: Step) {
        self.build_step = stage;
    }
    pub fn add_function(&mut self) {}
    pub fn load_labt_table(&mut self) -> Result<()> {
        load_labt_table(&mut self.lua)?;
        Ok(())
    }
}
