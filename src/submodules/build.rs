use std::{
    cell::RefCell,
    fmt::Display,
    path::{Path, PathBuf},
};

use anyhow::Context;
use clap::{Args, ValueEnum};
use reqwest::Url;

use crate::{
    config::get_config,
    get_home, get_project_root,
    plugin::{load_plugins, load_plugins_from_paths},
};

use super::Submodule;

// temporary, will remove if a cleaner way of passing the current step
// to plugins is achieved
thread_local! {
    pub static BUILD_STEP: RefCell<Step> = const { RefCell::new(Step::PRE) };
}

#[derive(Clone, Args)]
pub struct BuildArgs {
    pub step: Option<Step>,
}

pub struct Build {
    pub args: BuildArgs,
}

#[derive(Clone, Copy, ValueEnum, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    /// PRE compilation step which indicates that should
    /// run code generators, dependency injection, fetching dynamic
    /// modules etc.
    PRE,
    /// Android asset packaging
    AAPT,
    /// Application source files compilation, e.g. compile java, kotlin etc.
    COMPILE,
    /// Dexing application step to generate app classes.dex from jar files
    DEX,
    /// Bundles compiled resources into apk file and aligning, signing etc.
    BUNDLE,
    /// POST compilation step. Run, create a release file, return results to
    /// CI/CD pipeline etc.
    POST,
}
impl Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::PRE => write!(f, "pre"),
            Step::AAPT => write!(f, "aapt"),
            Step::COMPILE => write!(f, "compile"),
            Step::DEX => write!(f, "dex"),
            Step::BUNDLE => write!(f, "bundle"),
            Step::POST => write!(f, "post"),
        }
    }
}

impl Build {
    pub fn new(args: &BuildArgs) -> Self {
        Self { args: args.clone() }
    }
}

impl Submodule for Build {
    fn run(&mut self) -> anyhow::Result<()> {
        // The order by which to run the plugin build step
        let order: Vec<Step> = if let Some(step) = self.args.step {
            // if the build step was added explicitly, then just run that one
            // particular step
            vec![step]
        } else {
            // TODO add a more intelligent filter to run only the
            // required steps instead of just running everything
            vec![
                Step::PRE,
                Step::AAPT,
                Step::COMPILE,
                Step::DEX,
                Step::BUNDLE,
                Step::POST,
            ]
        };
        let mut home = get_home().context("Failed to load plugin home")?;
        home.push("plugins");
        // try loading plugin from config
        let config = get_config().context("Failed to load plugins list from config")?;
        // array of plugin locations to be loaded
        let mut paths: Vec<PathBuf> = vec![];
        if let Some(plugins) = config.plugins {
            paths.extend(plugins.iter().map(|(name, plugin)| {
                // check if plugin has location string

                if let Some(location) = &plugin.location {
                    // if location is a valid url, load from labt home plugins
                    if Url::parse(location.as_str()).is_ok() {
                        let mut h = home.clone();
                        h.push(format!("{}-{}", name.clone(), plugin.version.clone()));
                        h
                    } else {
                        // else use the defined location
                        PathBuf::from(location)
                    }
                } else {
                    // TODO this branch is really confusing, but maybe in the future it wiil be used to load fro central repo
                    // anyways load from home
                    let mut h = home.clone();
                    h.push(format!("{}-{}", name.clone(), plugin.version.clone()));
                    h
                }
            }));
        }

        {
            // include the paths of plugins in the project folder
            let mut root = get_project_root()
                .context("Failed to read the project root folder")?
                .clone();
            root.push("plugins");
            if root.exists() {
                for path in (root.read_dir()?).flatten().map(|entry| entry.path()) {
                    if path.is_dir() {
                        paths.push(path);
                    }
                }
            }
        }

        let plugin_list = load_plugins_from_paths(paths).context("Failed to load plugins")?;
        let mut map = load_plugins(plugin_list).context("Error loading plugin configurations")?;

        for step in order {
            // update build step if already provided
            BUILD_STEP.with(|s| {
                *s.borrow_mut() = step;
            });

            if let Some(plugins) = map.get_mut(&step) {
                // sort plugins by priority
                plugins.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap());
                '_loop: for plugin in plugins {
                    // filter for only required plugins
                    if let Some((inputs, outputs)) = &plugin.dependents {
                        // iterate on plugin dependents,
                        // if the output list is empty, then it will skip the iteration and assume first run
                        for output in outputs {
                            // for each output, compare to see if to run this plugin stage
                            // only run plugins which have their dependents changed should be run
                            let is_stale = inputs
                                .iter()
                                .any(|input| is_file_newer(input, output).unwrap_or(false));

                            if !is_stale {
                                // the plugin outputs are newer so skip it
                                continue '_loop;
                            }
                        }
                    }
                    // loop through each plugin executing each
                    let exe = plugin.load().context(format!(
                        "Error loading plugin: {}:{} at build step {:?}",
                        plugin.name, plugin.version, plugin.step
                    ))?;

                    let chunk = exe.load().context(format!(
                        "Error loading lua code for {}:{} at build step {:?}",
                        plugin.name, plugin.version, plugin.step
                    ))?;

                    chunk.exec().context(format!(
                        "Failed to execute plugin code {:?} for plugin {}:{} at build step {:?}",
                        plugin.path, plugin.name, plugin.version, plugin.step
                    ))?;
                }
            }
        }

        Ok(())
    }
}
/// Returns true if file a is newer than file b
/// If file b does not exist, returns true
/// if file a does not exist returns false
/// This function may just break in some platforms
/// # Errors
///
/// Returns an error if we fail to get the metadata of the file
pub fn is_file_newer(a: &Path, b: &Path) -> std::io::Result<bool> {
    if !b.exists() {
        return Ok(true);
    }
    if !a.exists() {
        return Ok(false);
    }
    // try to obtain the metadata for comparison
    let metadata_a = match a.metadata() {
        Ok(metadata) => metadata,
        Err(err) => return Err(err),
    };

    let metadata_b = match b.metadata() {
        Ok(metadata) => metadata,
        Err(err) => return Err(err),
    };

    let modification_a = match metadata_a.modified() {
        Ok(modified) => modified,
        Err(err) => return Err(err),
    };
    let modification_b = match metadata_b.modified() {
        Ok(modified) => modified,
        Err(err) => return Err(err),
    };

    Ok(modification_a > modification_b)
}
