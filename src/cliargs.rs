use crate::submodules::add::{Add, AddArgs};
use crate::submodules::build::{Build, BuildArgs};
use crate::submodules::init::{Init, InitArgs};
use crate::submodules::resolve::{ResolveArgs, Resolver};
use crate::submodules::Submodule;
use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

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
                eprintln!("Error: {:?}", e);
            }
        }
        Some(Commands::Init(args)) => {
            if let Err(e) = Init::new(args).run() {
                eprintln!("Error: {:?}", e);
            }
        }
        Some(Commands::Resolve(args)) => {
            if let Err(e) = Resolver::new(args).run() {
                eprintln!("Error: {:?}", e);
            }
        }
        Some(Commands::Build(args)) => {
            if let Err(e) = Build::new(args).run() {
                eprintln!("Error: {:?}", e);
            }
        }
        None => {}
    }
}
