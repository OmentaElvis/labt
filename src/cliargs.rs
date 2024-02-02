use crate::submodules::add::{Add, AddArgs};
use crate::submodules::init::{Init, InitArgs};
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
}

pub fn parse_args() {
    let args = Cli::parse();

    match &args.command {
        Some(Commands::Add(args)) => {
            Add::new(args).run().unwrap();
        }
        Some(Commands::Init(args)) => {
            if let Err(e) = Init::new(args).run() {
                println!("{e}");
            }
        }
        None => {}
    }
}
