use std::{
    collections::HashMap,
    env::current_dir,
    fs::{create_dir_all, read_to_string, File},
    io::Write,
    path::PathBuf,
    time::Duration,
};

use anyhow::{bail, Context};
use clap::{Args, Subcommand};
use git2::{DescribeFormatOptions, DescribeOptions, Repository, WorktreeAddOptions};
use indicatif::{ProgressBar, ProgressStyle};
use log::{info, warn};
use reqwest::Url;

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
    command: Option<PluginSubcommands>,

    /// Specify the plugin url in the format URL[@version]
    plugin_id: Option<String>,
}

#[derive(Clone, Subcommand)]
#[clap(
    group = clap::ArgGroup::new("plugin_subcommands"),
)]
pub enum PluginSubcommands {
    /// Create a plugin
    Create(CreateArgs),
    /// remove a plugin
    Remove(RemoveArgs),
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
        if let Some(command) = &self.args.command {
            match command {
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
                PluginSubcommands::Fetch => {
                    fetch_plugins_from_config().context("Failed to fetch plugins")?;
                }
            }
        }

        if let Some(id) = &self.args.plugin_id {
            let mut split = id.split('@');
            let url = split.next().unwrap();
            let version = split.next();

            fetch_plugin(url, version, true).context("Failed to configure plugin.")?;
        }
        Ok(())
    }
}

fn fetch_version<'a>(
    repo: &'a Repository,
    version: &str,
) -> anyhow::Result<(String, git2::Reference<'a>)> {
    // in the latest version, pick the latest tag
    let version = if version.eq("latest") {
        let mut describe_options = DescribeOptions::new();
        describe_options.describe_tags();
        describe_options.pattern("v*");
        let describe = repo
            .describe(&describe_options)
            .context("Unable to obtain the latest tag.")?;
        describe
            .format(Some(DescribeFormatOptions::new().abbreviated_size(0)))
            .context("Failed fo format git describe")?
    } else {
        version.to_string()
    };
    let reference_string = format!("refs/tags/{}", version);

    Ok((
        version,
        repo.find_reference(&reference_string)
            .context(format!("Failed to lookup {}", reference_string))?,
    ))
}

/// Do a clone if the location is a http url
/// else if the path exists on os file system, add it to the config
/// Returns an error if the underlying io/parsing operations fail.
pub fn fetch_plugin(
    location: &str,
    version: Option<&str>,
    update_config: bool,
) -> anyhow::Result<()> {
    const LATEST: &str = "latest";
    let version = version.unwrap_or(LATEST);

    let path = if let Ok(url) = Url::parse(location) {
        // let name = if let Some(name) = name {
        //     name.replace('/', "_").to_string()
        // } else {
        //     let url_path = url.path();
        //     let re = regex::Regex::new(r"^\/(?<user>[\w\-\/]+)\/(?<repo>[\w-]+)(\.git)?").unwrap();
        //     let captures = re
        //         .captures(url_path)
        //         .context("Failed to match the url path with expected format.")?;
        //     let user_org = captures.name("user").map(|m| m.as_str());
        //     let repo_name = captures.name("repo").map(|m| m.as_str());

        //     let mut segs = vec![];
        //     if let Some(user) = user_org {
        //         segs.push(user);
        //     }

        //     if let Some(repo) = repo_name {
        //         segs.push(repo);
        //     }

        //     let name = segs.join("_").replace('/', "_");
        //     if name.is_empty() {
        //         bail!("Unable to resolve plugin name from the provided url.");
        //     }
        //     name.to_string()
        // };

        let mut path = get_home().context("Failed to get Labt Home")?;
        path.push("plugins");
        if let Some(domain) = url.domain() {
            path.push(domain);
        } else {
            path.push("example.com"); // keep this
        }

        let url_path = url.path();
        let url_path = if let Some(p) = url_path.strip_suffix(".git") {
            p
        } else {
            url_path
        };
        path.extend(url_path.split('/'));

        let mut git_path = path.clone();
        git_path.push("git");

        // TODO use git2 callbacks to display meaningfull progress
        // TODO do a checkout for a specific tag
        // start a new spinner progress bar and add it to the global multi progress bar
        let spinner = MULTI_PROGRESS_BAR.add(ProgressBar::new_spinner());
        spinner.enable_steady_tick(Duration::from_millis(100));
        spinner.set_style(
            ProgressStyle::with_template("{spinner} {prefix:.blue} {wide_msg}").unwrap(),
        );
        spinner.set_prefix("Plugin");
        let repo = if !git_path.exists() {
            // must be a fresh plugin. Clone it
            create_dir_all(&git_path).context(format!(
                "Unable to create plugin directory at {}",
                git_path.to_string_lossy()
            ))?;
            // TODO Do a basic cleanup if cloning fails
            spinner.set_message(format!("Clonning {}", location));
            Repository::clone(location, git_path)
                .context("Failed to clone plugin to local directory")?
        } else {
            // The git path exists, so the repository definately exists
            // unless someone decides to tamper wit this directory.
            spinner.set_message(format!("Opening repo at {}", location));
            let repo = Repository::open(&git_path).context(format!(
                "Failed to open plugin repository at {}",
                git_path.to_string_lossy()
            ))?;

            // the repository could have changed alot since labt interacted with it.
            // So try to fetch the latest updates from origin
            spinner.set_message(format!("Fetching updates from {}", location));
            let mut remote = repo
                .find_remote("origin")
                .context("Unable to get the repository \"origin\"")?;

            // fetch all these lads
            remote.fetch(
                &[
                    "refs/heads/*:refs/remotes/origin/*",
                    "refs/tags/*:refs/tags/*",
                ],
                None,
                None,
            )?;
            // drop it so that the borrow checker does not try to crusify us.
            drop(remote);

            repo
        };

        let mut worktrees_path = path.clone();
        worktrees_path.push("versions");

        // create the worktree directory
        if !worktrees_path.exists() {
            create_dir_all(&worktrees_path).context(format!(
                "Unable to create plugin worktree directory at {}",
                path.to_string_lossy()
            ))?;
        }

        let (version, reference) =
            fetch_version(&repo, version).context("Failed to resolve version from plugin repo")?;

        // obtain the tag name
        worktrees_path.push(&version);
        if !worktrees_path.exists() && reference.is_tag() {
            let id = reference
                .target()
                .context("Unable to obtain reference oid")?;
            let commit = repo.find_commit(id)?;

            let branch = match repo.branch(&version, &commit, false) {
                Err(err) => {
                    if let git2::ErrorCode::Exists = err.code() {
                        repo.find_branch(&version, git2::BranchType::Local)?
                    } else {
                        return Err(err).context(format!(
                            "Failed to branch out from selected tag: {}",
                            version
                        ));
                    }
                }
                Ok(branch) => branch,
            };

            let mut worktree_options = WorktreeAddOptions::new();
            worktree_options.reference(Some(branch.get()));

            repo.worktree(&version, &worktrees_path, Some(&worktree_options))?;
        }

        spinner.finish_and_clear();
        worktrees_path
    } else {
        let p = PathBuf::from(&location);
        if !p.exists() {
            bail!("The argument provided is neither a valid url nor a valid plugin directory. If you are providing a url, please include the protocol scheme e.g. https:// ");
        }
        p
    };
    let mut plugin_toml_path = path.clone();
    plugin_toml_path.push("plugin.toml");

    let toml_string = read_to_string(&plugin_toml_path).context(format!(
        "Failed to read plugin toml string from {}",
        plugin_toml_path.to_string_lossy()
    ))?;

    let plugin_toml = toml_string.parse::<PluginToml>().context(format!(
        "Failed to parse plugin.toml from {}",
        plugin_toml_path.to_string_lossy()
    ))?;

    // check if its a fs path
    if update_config {
        // TODO check which is best to use. plugin_toml.version or version passed by user.
        add_plugin_to_config(
            plugin_toml.name.clone(),
            plugin_toml.version.clone(),
            location.to_string(),
        )
        .context("Failed to add plugin to project config")?;
    }

    info!(target: "plugin", "Installed plugin: {}@{}", plugin_toml.name, plugin_toml.version);

    Ok(())
}

/// Fetches all plugin listed on the project config
/// Returns an error if the underlying io/parsing operations fail.
pub fn fetch_plugins_from_config() -> anyhow::Result<()> {
    let config = get_config().context("Failed reading project configuration")?;
    if let Some(plugins) = config.plugins {
        for (name, plugin) in plugins {
            fetch_plugin(
                &plugin.location.unwrap_or(
                    get_home() // try using labt home if not specified
                        .context("Failed to get Labt home")?
                        .to_str()
                        .unwrap_or("")
                        .to_string(),
                ),
                Some(plugin.version.as_str()),
                false,
            )
            .context(format!(
                "Failed to fetch plugin: {}@{}",
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
        stages: HashMap::default(),
        path: PathBuf::new(),
        package_paths: None,
        enable_unsafe: false,
        sdk: Vec::new(),
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

    let doc = plugin.to_string();
    path.push("plugin.toml");
    let mut file =
        File::create(&path).context(format!("Failed to create plugin file at {:?}", path))?;
    file.write_all(doc.to_string().as_bytes())
        .context(format!("Failed to write plugin file {:?}", path))?;

    info!(target: "plugin", "Created a plugin at {:?}", path);

    Ok(())
}
