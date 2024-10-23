use crate::submodules::add::{Add, AddArgs};
use crate::submodules::build::{Build, BuildArgs};
use crate::submodules::init::{Init, InitArgs};
use crate::submodules::plugin::{Plugin, PluginArgs};
use crate::submodules::resolve::{Resolve, ResolveArgs};
use crate::submodules::sdk::{Sdk, SdkArgs};
use crate::submodules::Submodule;
use crate::LABT_VERSION;
use clap::{CommandFactory, Parser, Subcommand};
use console::style;
use log::error;

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

pub fn parse_args() {
    let args = Cli::parse();

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
