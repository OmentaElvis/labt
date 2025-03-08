use std::env;
use std::process::Stdio;
use std::rc::Rc;

use crate::plugin::executable::ExecutableLua;
use crate::submodules::add::{Add, AddArgs};
use crate::submodules::build::{Build, BuildArgs};
use crate::submodules::init::{Init, InitArgs};
use crate::submodules::plugin::{Plugin, PluginArgs};
use crate::submodules::resolve::{Resolve, ResolveArgs};
use crate::submodules::sdk::{InstalledPackage, Sdk, SdkArgs};
use crate::submodules::sdkmanager::installed_list::InstalledList;
use crate::submodules::sdkmanager::ToId;
use crate::submodules::Submodule;
use crate::LABT_VERSION;
use anyhow::Context;
use clap::{CommandFactory, Parser, Subcommand};
use console::style;
use log::error;
use mlua::Function;

#[derive(Parser)]
#[clap(version = LABT_VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

const LOGO: &str = r#"
  _        _    ____  _   
 | |      / \  | __ )| |_ 
 | |     / _ \ |  _ \| __|
 | |___ / ___ \| |_) | |_ 
 |_____/_/   \_\____/ \__|
 Lightweight Android Build
          Tool
"#;

#[derive(Subcommand)]
enum Commands {
    /// Adds a new project dependency
    Add(AddArgs),
    /// Initializes a new project
    Init(InitArgs),
    /// Fetches the project dependencies
    Resolve(ResolveArgs),
    /// Builds the project
    Build(BuildArgs),
    /// Manage plugins
    Plugin(PluginArgs),
    /// Sdk manager
    Sdk(SdkArgs),
}

fn run_sdk_command(package: &InstalledPackage, args: Vec<String>) -> anyhow::Result<()> {
    // try to load init.lua and execute run(args)
    let dir = ExecutableLua::get_package_directory(package).context(format!(
        "Failed to obtain sdk install directory for package: {}",
        package.to_id()
    ))?;
    let init = dir.join("init.lua");

    let deps = Rc::new(Vec::new());
    let mut exe = ExecutableLua::new(init, &[], deps.clone(), true);

    exe.load_sdk_loader()?;
    exe.load_api_tables()?;

    let lua = exe.get_lua();
    ExecutableLua::add_path(lua, &dir.join("?.lua").to_string_lossy())?;
    ExecutableLua::add_path(lua, &dir.join("?").join("init.lua").to_string_lossy())?;
    ExecutableLua::add_cpath(lua, &dir.join("?.so").to_string_lossy())?;

    let chunk = exe.load()?;
    chunk.exec()?;
    let globals = lua.globals();
    let function: Function = globals.get("run").context(
        "Failed to find `run` function in global context. Unable to locate entry point.",
    )?;
    function.call::<&[String], mlua::Value>(&args[1..])?;

    Ok(())
}

fn try_run_command(
    command: &str,
    installed_list: &InstalledList,
    args: Vec<String>,
) -> anyhow::Result<bool> {
    let mut executed = false;
    for package in &installed_list.packages {
        let mut dir = ExecutableLua::get_package_directory(package).context(format!(
            "Failed to obtain sdk install directory for package: {}",
            package.to_id()
        ))?;

        if let Some(path) = &package.command {
            // if command option is defined, treat it as a subdirectory to our sdk directory where we can find the subcommand
            dir.push(path);
        }

        let file = dir.join(command);
        if file.exists() {
            let mut cmd = std::process::Command::new(file);
            cmd.args(&args[2..]);
            cmd.stdout(Stdio::inherit());
            cmd.stderr(Stdio::inherit());
            cmd.stdin(Stdio::inherit());
            cmd.output().context("Failed to run sdk command!")?;
            executed = true;
            break;
        }
        log::trace!(target: "sdk" ,"{:?} not found", file);
    }

    Ok(executed)
}

pub fn parse_args() {
    let args = match Cli::try_parse() {
        Ok(args) => args,
        Err(err) => {
            // look through the list of installed packages for matching subcommand
            let installed_list: InstalledList = match InstalledList::parse_from_sdk() {
                Ok(installed) => installed,
                Err(err) => {
                    log::error!(target: "sdk", "Failed to parse installed.toml: {:#?}", err);
                    return;
                }
            };
            let args: Vec<String> = env::args().collect();
            if let Some(key) = args.get(1) {
                for package in &installed_list.packages {
                    match (&package.command, package.module) {
                        (Some(name), Some(true)) => {
                            if name != key.as_str() {
                                continue;
                            }
                            if let Err(err) = run_sdk_command(package, args) {
                                log::error!(target: "sdk", "{}", err);
                            }
                            return;
                        }
                        _ => continue,
                    }
                }
                // no command found in installed list.
                // Now do what $PATH does by scanning through the directory finding an executable file
                match try_run_command(key, &installed_list, args.clone()) {
                    Ok(executed) => {
                        if executed {
                            return;
                        }
                    }
                    Err(err) => {
                        log::error!(target: "sdk", "{}", err);
                        return;
                    }
                }
            }
            err.exit();
        }
    };

    match &args.command {
        Some(Commands::Add(args)) => {
            if let Err(e) = Add::new(args).run() {
                error!(target: "add","{:?}", e);
            }
        }
        Some(Commands::Init(args)) => {
            if let Err(e) = Init::new(args).run() {
                error!(target: "init","{:?}", e);
            }
        }
        Some(Commands::Resolve(args)) => {
            if let Err(e) = Resolve::new(args).run() {
                error!(target: "resolve","{:?}", e);
            }
        }
        Some(Commands::Build(args)) => {
            if let Err(e) = Build::new(args).run() {
                error!(target: "build", "{:?}", e);
            }
        }
        Some(Commands::Plugin(args)) => {
            if let Err(e) = Plugin::new(args).run() {
                error!(target: "plugin", "{:?}", e);
            }
        }
        Some(Commands::Sdk(args)) => {
            if let Err(e) = Sdk::new(args).run() {
                error!(target: "sdk", "{:?}", e);
            }
        }
        None => {
            let mut c = Cli::command();
            let line = style("----------------------------").bold().dim();
            let version = style(LABT_VERSION).bold();
            println!("{line}{}{:^24}\n{line}", LOGO, version);
            c.print_help().unwrap();
        }
    }
}
