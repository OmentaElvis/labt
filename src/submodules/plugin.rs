use std::{
    env::current_dir,
    fs::{create_dir_all, File},
    io::Write,
    path::PathBuf,
    time::Duration,
};

use anyhow::Context;
use clap::{Args, Subcommand};
use git2::Repository;
use indicatif::{ProgressBar, ProgressStyle};
use log::{info, warn};

use crate::{
    config::{add_plugin_to_config, get_config, remove_plugin_from_config},
    get_home,
    plugin::config::PluginToml,
    MULTI_PROGRESS_BAR,
};

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
            PluginSubcommands::Create(arg) => {
                create_new_plugin(
                    arg.name.clone(),
                    arg.version.clone(),
                    arg.path.clone(),
                    arg.local,
                )
                .context("Failed to create the new plugin")?;
            }
            PluginSubcommands::Remove(arg) => {
                // removes the plugin from config only and remains globally
                remove_plugin_from_config(arg.name.clone())
                    .context("Failed to remove plugin from config")?;
            }
            PluginSubcommands::Use(arg) => {
                fetch_plugin(
                    arg.name.clone(),
                    arg.version.clone(),
                    arg.location.clone(),
                    true,
                )
                .context("Failed to configure plugin.")?;
            }
            PluginSubcommands::Fetch => {
                fetch_plugins_from_config().context("Failed to fetch plugins")?;
            }
        }
        Ok(())
    }
}

/// Do a clone if the location is a http url
/// else if the path exists on os file system, add it to the config
/// Returns an error if the underlying io/parsing operations fail.
pub fn fetch_plugin(
    name: String,
    version: String,
    location: String,
    update_config: bool,
) -> anyhow::Result<()> {
    // check if its a fs path
    let path = PathBuf::from(&location);
    if path.exists() {
        if update_config {
            add_plugin_to_config(name, version, location)
                .context("Failed to add plugin to project config")?;
        }
        return Ok(());
    }

    let mut path = get_home().context("Failed to get Labt Home")?;
    path.push("plugins");
    path.push(format!("{}-{}", name, version));

    // check if plugin already exists on that path
    let mut conf = path.clone();
    conf.push("plugin.toml");
    if conf.exists() {
        if update_config {
            add_plugin_to_config(name, version, location)
                .context("Failed to add plugin to project config")?;
        }
        return Ok(());
    }

    // The plugin does not exist
    create_dir_all(&path).context(format!(
        "Failed to create plugin directory for {}-{}",
        name, version
    ))?;

    // start a new spinner progress bar and add it to the global multi progress bar
    let spinner = MULTI_PROGRESS_BAR.with(|multi| multi.borrow().add(ProgressBar::new_spinner()));
    spinner.enable_steady_tick(Duration::from_millis(100));
    spinner.set_style(ProgressStyle::with_template("{spinner} {prefix:.blue} {wide_msg}").unwrap());
    spinner.set_prefix("Plugin");
    spinner.set_message(format!("Clonning {}", location));

    // TODO use git2 callbacks to display meaningfull progress
    // TODO do a checkout for a specific tag

    // must be a url
    let _repo = Repository::clone(location.as_str(), path)
        .context("Failed to clone plugin to local directory")?;

    if update_config {
        // add plugin to config
        add_plugin_to_config(name.clone(), version.clone(), location)
            .context("Failed to add plugin to project config")?;
    }
    // finish and clear progressbar
    spinner.finish_and_clear();
    info!(target: "plugin", "Installed plugin: {}-{}", name, version);

    Ok(())
}

/// Fetches all plugin listed on the project config
/// Returns an error if the underlying io/parsing operations fail.
pub fn fetch_plugins_from_config() -> anyhow::Result<()> {
    let config = get_config().context("Failed reading project configuration")?;
    if let Some(plugins) = config.plugins {
        for (name, plugin) in plugins {
            fetch_plugin(
                name.clone(),
                plugin.version.clone(),
                plugin.location.unwrap_or(
                    get_home() // try using labt home if not specified
                        .context("Failed to get Labt home")?
                        .to_str()
                        .unwrap_or("")
                        .to_string(),
                ),
                false,
            )
            .context(format!(
                "Failed to fetch plugin: {}-{}",
                name, plugin.version
            ))?;
        }
    }
    Ok(())
}

/// Creates a new plugin on the provided path, if local_plugin is true, the
/// plugin is created on current directory
/// UNSTABLE
/// Returns an error if underlying IO/serialization operations fail
pub fn create_new_plugin(
    name: String,
    version: String,
    path: Option<PathBuf>,
    local_plugin: bool,
) -> anyhow::Result<()> {
    warn!("This is an unstable feature, Things may not work correctly");
    let plugin = PluginToml {
        name: name.clone(),
        version: version.clone(),
        stage: crate::plugin::config::Stage {
            pre: None,
            aapt: None,
            compile: None,
            dex: None,
            bundle: None,
            post: None,
        },
        path: PathBuf::new(),
        package_paths: None,
    };

    let mut path = if local_plugin {
        let mut cwd = current_dir().context("Failed to get current working directory.")?;
        cwd.push("plugins");
        cwd.push(format!("{}-{}", name, version));
        create_dir_all(&cwd).context("Failed creating plugin directory on project folder")?;
        cwd
    } else {
        // if is not a local plugin, then check for path
        if let Some(path) = path {
            // path was specified so the user knows what they are doing
            // so return path without any extras
            path
        } else {
            let mut path = get_home().context("Failed to get Labt Home")?;
            path.push("plugins");
            path.push(format!("{}-{}", name, version));
            path
        }
    };

    let doc = toml_edit::ser::to_document(&plugin).context("Failed to serialize plugin config")?;
    path.push("plugin.toml");
    let mut file =
        File::create(&path).context(format!("Failed to create plugin file at {:?}", path))?;
    file.write_all(doc.to_string().as_bytes())
        .context(format!("Failed to write plugin file {:?}", path))?;

    info!(target: "plugin", "Created a plugin at {:?}", path);

    Ok(())
}
