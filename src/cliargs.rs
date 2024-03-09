use crate::submodules::add::{Add, AddArgs};
use crate::submodules::build::{Build, BuildArgs};
use crate::submodules::init::{Init, InitArgs};
use crate::submodules::resolve::{ResolveArgs, Resolver};
use crate::submodules::Submodule;
use clap::{CommandFactory, Parser, Subcommand};
use console::style;
use log::error;

#[derive(Parser)]
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
}

pub fn parse_args() {
    let args = Cli::parse();

    match &args.command {
        Some(Commands::Add(args)) => {
            if let Err(e) = Add::new(args).run() {
                error!(target: "build","{:?}", e);
            }
        }
        Some(Commands::Init(args)) => {
            if let Err(e) = Init::new(args).run() {
                error!(target: "build","{:?}", e);
            }
        }
        Some(Commands::Resolve(args)) => {
            if let Err(e) = Resolver::new(args).run() {
                error!(target: "build","{:?}", e);
            }
        }
        Some(Commands::Build(args)) => {
            if let Err(e) = Build::new(args).run() {
                error!(target: "build", "{:?}", e);
            }
        }
        None => {
            let mut c = Cli::command();
            let line = style("----------------------------").bold().dim();
            println!("{line}{}{line}", LOGO);
            c.print_help().unwrap();
        }
    }
}
