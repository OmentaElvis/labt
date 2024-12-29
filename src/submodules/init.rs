use crate::{
    config::LabToml,
    plugin::{api::MluaAnyhowWrapper, config::load_package_paths, executable::ExecutableLua},
    PROJECT_ROOT,
};
use anyhow::{bail, Context};
use clap::Args;
use labt_proc_macro::labt_lua;
use mlua::{Lua, LuaSerdeExt, Table};
use std::{env::current_dir, fs::File, io::Write, path::PathBuf, rc::Rc, sync::OnceLock};
use tera::Tera;

use super::{plugin::fetch_plugin, Submodule};

#[derive(Args, Clone)]
pub struct InitArgs {
    /// Template repository url
    name: String,
    /// Directory to create project in
    path: Option<PathBuf>,
    #[arg(long, action)]
    /// Trust the installation of the plugin(s), as they have the ability to execute arbitrary code.
    trust: bool,
}

pub struct Init {
    pub args: InitArgs,
}

pub struct ProjectPaths {
    pub root: PathBuf,
    pub app: PathBuf,
    pub package: PathBuf,
    pub res: PathBuf,
}

impl Init {
    pub fn new(args: &InitArgs) -> Init {
        Init { args: args.clone() }
    }
}

static TERA: OnceLock<Tera> = OnceLock::new();

#[labt_lua]
fn render(_lua: &Lua, (name, context): (String, Table)) {
    let t = TERA
        .get()
        .context("Tera template not initialized yet.")
        .map_err(MluaAnyhowWrapper::external)?;
    let render = t
        .render(
            &name,
            &tera::Context::from_serialize(context)
                .context("Failed to serialize lua table to tera context")
                .map_err(MluaAnyhowWrapper::external)?,
        )
        .context("Failed to render template")
        .map_err(MluaAnyhowWrapper::external)?;
    Ok(render)
}

fn load_template_table(lua: &Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;
    render(lua, &table)?;

    lua.globals().set("template", table)?;
    Ok(())
}

impl Submodule for Init {
    /*
        ============Entry point for this module =================
    */

    /// Executed by this module loader, it receives the commandline
    /// arguments for this subcommand stored in self.args
    /// This is the entry point for Init subcommand
    fn run(&mut self) -> anyhow::Result<()> {
        if self.args.path.is_none() {
            let cwd = std::env::current_dir()?;
            self.args.path = Some(cwd);
        }

        let id = &self.args.name;

        let mut split = id.split('@');
        let url = split.next().unwrap();
        let version = split.next();
        let mut iknow_what_iam_doing = self.args.trust;

        // pull the plugin from its location
        let (config, path) = fetch_plugin(url, version, false, false, &mut iknow_what_iam_doing)
            .context("Unable to fetch requested plugin for project initialization.")?
            .context("Plugin installation cancelled")?;

        if config.init.is_none() {
            bail!("Plugin has no init script available to create a project.");
        }

        let init = config.init.unwrap();
        let init_file = path.join(init.file);

        // override the project path with the current working dir
        let cwd = current_dir()?;
        let _ = PROJECT_ROOT.set(cwd);

        let package_paths = if let Some(package_paths) = &config.package_paths {
            load_package_paths(package_paths, &path)
        } else {
            load_package_paths(&[], &path)
        };

        let mut exec = ExecutableLua::new(init_file, &package_paths, Rc::new(Vec::new()), false);
        exec.load_api_tables()
            .context("Error injecting api tables into lua context")?;
        let lua = exec.get_lua();
        load_template_table(lua)?;

        let chunk = exec.load().context("Failed to load project init script")?;

        let output = self.args.path.as_ref().unwrap();
        chunk
            .exec()
            .context("Failed to execute project init code.")?;

        // now call the templater init function with path
        let template_path = if let Some(template) = init.templates {
            path.join(template)
        } else {
            path.join("templates/*")
        };

        let t = Tera::new(template_path.to_string_lossy().as_ref())?;
        TERA.get_or_init(|| t);

        let init_function: mlua::Function = lua
            .globals()
            .get("init")
            .context("Unable to load project init function")?;

        let (project_table, override_path) = init_function
            .call::<String, (Table, Option<String>)>(output.to_string_lossy().to_string())
            .context("Failed to execute plugin init function")?;

        let project: LabToml = lua.from_value(mlua::Value::Table(project_table))?;
        let toml = toml::to_string(&project).context("Serializing LabtToml to toml string")?;

        let mut output = if let Some(path) = override_path {
            PathBuf::from(path)
        } else {
            self.args.path.as_ref().unwrap().clone()
        };

        output.push("Labt.toml");

        // create file target to write toml file
        let mut file = File::create(&output).context(format!(
            "Error creating Labt.toml file at {}",
            path.to_str().unwrap_or("[unknown]")
        ))?;

        // write the toml to file
        file.write_all(toml.as_bytes()).context(format!(
            "Writing LabtToml string to toml file at {}",
            path.to_str().unwrap_or("[unknown]")
        ))?;
        Ok(())
    }
}
