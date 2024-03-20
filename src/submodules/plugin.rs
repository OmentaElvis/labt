use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::Submodule;

#[derive(Clone, Args)]
pub struct PluginArgs {
    /// Plugin subcommand
    #[command(subcommand)]
    command: PluginSubcommands,
}

#[derive(Clone, Subcommand)]
pub enum PluginSubcommands {
    /// Create a plugin
    Create(CreateArgs),
    /// remove a plugin
    Remove(RemoveArgs),
    /// Use a plugin, tries to install the plugin if missing
    Use(UseArgs),
    /// Install missing plugins defined in Project config
    Fetch,
}

#[derive(Clone, Args)]
pub struct CreateArgs {
    /// Plugin name to create
    name: String,
    /// Plugin version
    version: String,
    /// The location to create the plugin, default Labt plugins folder
    path: Option<PathBuf>,
    /// If the plugin is local to the project
    #[arg(short, long, action)]
    local: bool,
}

#[derive(Clone, Args)]
pub struct RemoveArgs {
    /// Plugin name to be removed
    name: String,
}

#[derive(Clone, Args)]
pub struct UseArgs {
    /// The name of the plugin
    name: String,
    /// The version of the plugin
    version: String,
    /// The path/url where to fetch the plugin
    location: String,
}

pub struct Plugin<'a> {
    /// commandline args passed to this submodule
    args: &'a PluginArgs,
}

impl<'a> Plugin<'a> {
    pub fn new(args: &'a PluginArgs) -> Self {
        Plugin { args }
    }
}

impl<'a> Submodule for Plugin<'a> {
    /// The module entry point
    fn run(&mut self) -> anyhow::Result<()> {
        // match through the subcommands
        match &self.args.command {
            PluginSubcommands::Create(arg) => {}
            PluginSubcommands::Remove(arg) => {}
            PluginSubcommands::Use(args) => {}
            PluginSubcommands::Fetch => {}
        }
        Ok(())
    }
}
