use std::{
    collections::HashMap,
    env::current_dir,
    fs::{create_dir_all, read_to_string, File},
    io::Write,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use anyhow::{bail, Context};
use clap::{Args, Subcommand};
use git2::{DescribeFormatOptions, DescribeOptions, Repository, WorktreeAddOptions};
use indicatif::{ProgressBar, ProgressStyle};
use log::{info, trace, warn};
use reqwest::Url;

use crate::{
    config::{
        add_plugin_to_config, get_config, remove_plugin_from_config, repository::RepositoryXml,
    },
    get_home,
    plugin::config::PluginToml,
    pom::VersionRange,
    submodules::{
        resolvers::GOOGLE_REPO_URL,
        sdk::{
            get_sdk_path, parse_repository_toml, toml_strings, Installer, InstallerTarget, Sdk,
            DEFAULT_URL, FAILED_TO_PARSE_SDK_STR, GOOGLE_REPO_NAME_STR,
        },
        sdkmanager::installed_list::InstalledList,
    },
    LABT_VERSION, MULTI_PROGRESS_BAR,
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
            if let Err(err) = remote.fetch(
                &[
                    "refs/heads/*:refs/remotes/origin/*",
                    "refs/tags/*:refs/tags/*",
                ],
                None,
                None,
            ) {
                match (err.code(), err.class()) {
                    (git2::ErrorCode::GenericError, git2::ErrorClass::Net) => {
                        warn!(target: "plugin", "A network request failed. We are unable to update the plugin git repo. We will proceed in offline mode but latest versions will be missing or incorrect.")
                    }
                    _ => {
                        bail!(err);
                    }
                }
            }
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

    // now lets check if this config requires a specific plugin
    if let Some(labt) = &plugin_toml.labt {
        let give_err = |v: &str| {
            format!(
                "Failed to compare LABT ({}) version with plugin requested version ({})",
                LABT_VERSION, v
            )
        };

        let reject = match labt {
            VersionRange::Gt(v) => {
                !version_compare::compare_to(LABT_VERSION, v, version_compare::Cmp::Gt)
                    .map_err(|_| anyhow::anyhow!(give_err(v)))?
            }
            VersionRange::Ge(v) => {
                !version_compare::compare_to(LABT_VERSION, v, version_compare::Cmp::Ge)
                    .map_err(|_| anyhow::anyhow!(give_err(v)))?
            }
            VersionRange::Lt(v) => {
                !version_compare::compare_to(LABT_VERSION, v, version_compare::Cmp::Lt)
                    .map_err(|_| anyhow::anyhow!(give_err(v)))?
            }
            VersionRange::Le(v) => {
                !version_compare::compare_to(LABT_VERSION, v, version_compare::Cmp::Le)
                    .map_err(|_| anyhow::anyhow!(give_err(v)))?
            }
            VersionRange::Eq(v) => {
                !version_compare::compare_to(v, LABT_VERSION, version_compare::Cmp::Eq)
                    .map_err(|_| anyhow::anyhow!(give_err(v)))?
            }
        };
        if reject {
            bail!("{}@{} requested LABt ({}) which is not compatible with the currently available LABt version ({}). Please check for other versions of the plugin or Install the appropriate version of LABt. ", 
                plugin_toml.name,
                plugin_toml.version,
                plugin_toml.labt.unwrap(),
                LABT_VERSION
            );
        }
    }

    if !plugin_toml.sdk.is_empty() {
        let mut installed_list = InstalledList::parse_from_sdk()?;
        const PLUGIN_SDK: &str = "plugin sdk";

        // add all the repositories specified by the plugin.
        for (name, repo) in plugin_toml.sdk_repo.iter() {
            // TODO possiblitity of repository name collisions.
            info!(target: PLUGIN_SDK, "Installing {} sdk repo for plugin {}@{}.", name, plugin_toml.name, plugin_toml.version);
            Sdk::add_repository(name, &repo.url).context(format!(
                "Failed to install {} sdk repo requested by plugin {}@{}.",
                name, plugin_toml.name, plugin_toml.version
            ))?;
        }

        let (host_os, bits) = Sdk::get_host_os_and_bits(None)?;

        let running = Arc::new(AtomicBool::new(true));
        let mut installer = Installer::new(
            Url::parse(DEFAULT_URL)?,
            bits,
            host_os.clone(),
            false,
            running,
        );

        // A very rough caching for the repository lists
        let mut repositories: HashMap<String, RepositoryXml> = HashMap::new();

        // the plugin requested for an sdk, so try to check for their existance an install if necessary
        for sdk in plugin_toml.sdk {
            // check if repo is specified

            // =================== INSTALL PLAN ===================
            // - We assume at this point all the repositories are ready
            // - The installer needs the repository "RemotePackage"
            // - We need to select the correct sdk repository
            //    + open the matching sdk repository config from file or cache memory
            //    + in the repository, find the package path/id. Error if not found, otherwise return the "RemotePackage".
            //    + pass the "RemotePackage" to the installer and its details
            //    + repeat for the next plugin sdk packages

            // Load this repository
            let repo = if let Some(repo) = &repositories.get(&sdk.repo) {
                repo
            } else {
                // not cached so load the repository
                if let Some(repo_entry) = installed_list.repositories.get(&sdk.repo) {
                    let mut path = repo_entry.path.clone();
                    path.push(toml_strings::CONFIG_FILE);
                    let repo = parse_repository_toml(&path).context(FAILED_TO_PARSE_SDK_STR)?;
                    repositories.insert(sdk.repo.to_string(), repo);
                    repositories.get(&sdk.repo).unwrap()
                } else {
                    // you fambled the repository name
                    bail!("The plugin config tried to install an sdk package from a repository name it did not specify in its config! ");
                }
            };

            // since we have obtained the correct repo. Now find the package
            let package = repo
                .get_remote_packages()
                .iter()
                .find(|p| {
                    if !&sdk.path.eq(p.get_path()) {
                        return false;
                    }

                    if !sdk.version.eq(p.get_revision()) {
                        return false;
                    }

                    if &sdk.channel != p.get_channel() {
                        return false;
                    }

                    true
                })
                .context(format!(
                    "Package {} v{}-{} does not exist on \"{}\" sdk repo.",
                    sdk.path, sdk.version, sdk.channel, sdk.channel
                ))?;

            if sdk.repo == GOOGLE_REPO_NAME_STR {
                installer.add_package(GOOGLE_REPO_NAME_STR, package.clone())?;
            } else {
                // the repo devs should specify the base url for this package.
                let base_url = if let Some(url) = package.get_base_url() {
                    Url::parse(url)?
                } else {
                    trace!(target: "sdkmanager", "Repository did not specify its base URL. Setting google repo url as a place holder hoping that they did for whatever archive we are installing. ");
                    Url::parse(GOOGLE_REPO_URL)?
                };

                let path: PathBuf = package.get_path().split(';').collect();
                let mut sdk_path = get_sdk_path()?;
                sdk_path.push(&sdk.repo);
                let target = InstallerTarget {
                    bits,
                    host_os: host_os.clone(),
                    target_path: sdk_path.join(path),
                    package: package.clone(),
                    download_url: Arc::new(base_url),
                    repository_name: sdk.repo.to_string(),
                };

                installer.add_target(target);
            }
        }
        installer.install()?;
        for package in installer.complete_tasks {
            installed_list.add_installed_package(package);
        }
        installed_list
            .save_to_file()
            .context("Failed to update installed package list with installed packages")?;
    }
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
        labt: None,
        sdk_repo: HashMap::new(),
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
