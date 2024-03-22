use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::Context;
use glob::glob;
use serde::{Deserialize, Serialize};

use crate::{get_project_root, submodules::build::Step};

use super::Plugin;

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct PluginToml {
    /// plugin name
    pub name: String,
    /// plugin version
    pub version: String,
    /// plugin states
    pub stage: Stage,

    #[serde(skip)]
    pub path: PathBuf,
    /// Paths to serch for required lua modules
    pub package_paths: Option<Vec<PathBuf>>,
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct Stage {
    /// Pre build state, used in generating code or building external
    /// dependency used in next steps
    pub pre: Option<PluginStage>,
    /// Complie application res folder and generate required R.java files
    pub aapt: Option<PluginStage>,
    /// Compile java/kotlin files to produce java jar files of the project
    pub compile: Option<PluginStage>,
    /// Dex jar files to produce android classes.dex files,
    pub dex: Option<PluginStage>,
    /// Bundles all the compiled app files into a zip with .apk extension,
    /// should also sign the bundle
    pub bundle: Option<PluginStage>,
    /// Do anything with the resulting built app file, deploy a release, install, run etc.
    pub post: Option<PluginStage>,
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct PluginStage {
    /// file containing the entry point
    pub file: PathBuf,
    /// plugin priority
    pub priority: i32,
    /// The input files that we should watch for changes
    pub inputs: Option<Vec<String>>,
    /// The output files that we should ensure that it is uptodate
    pub outputs: Option<Vec<String>>,
}

impl PluginToml {
    /// Maps PluginToml stages into their [`Plugin`] representation.
    pub fn get_steps(&self) -> anyhow::Result<Vec<Plugin>> {
        let mut steps = vec![];

        /// because i cant accurately copy & paste these mappings
        /// from PluginToml stage to Plugin Step without creating a bug,
        /// let the macro repeat it, maybe im lazy
        macro_rules! map_plugin {
            [$($i:ident = $j:expr),*] => {
                $(
                // check if $i is set, if set then create a sub plugin
                if let Some(s) = &self.stage.$i {
                    // get this plugin root directory
                    let mut path = self.path.clone();
                    // push the plugin source path to path
                    path.push(s.file.clone());
                    // create a plugin and set its step as $j
                    let mut plugin = Plugin::new(self.name.clone(), self.version.clone(), path, $j);
                    plugin.priority = s.priority;
                    plugin.package_paths = if let Some(package_paths) = &self.package_paths{
                            load_package_paths(package_paths, &self.path)
                        }else{
                            load_package_paths(&[], &self.path)
                        };

                    if s.inputs.is_some() && s.outputs.is_some() {
                        // both have items, so add them to the output
                        plugin.dependents = Some((expand_globs(s.inputs.clone().unwrap()).context("Unable to expand global patterns specified by the inputs dependents")?,
                                expand_globs(s.outputs.clone().unwrap()).context("Unable to expand global patterns specified by the outputs dependents")?));
                    }
                    // add the plugin to the list of plugins
                    steps.push(plugin);
                }
               )*
            };
        }

        map_plugin![
            pre = Step::PRE,
            aapt = Step::AAPT,
            compile = Step::COMPILE,
            dex = Step::DEX,
            bundle = Step::BUNDLE,
            post = Step::POST
        ];

        Ok(steps)
    }
}

fn expand_globs(patterns: Vec<String>) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths: HashSet<PathBuf> = HashSet::new();
    for pattern in patterns {
        let path = PathBuf::from(pattern);
        let path = if path.is_relative() {
            // if is a relative path, append project root instead
            let mut root = get_project_root()
                .context("Failed to get project root directory")?
                .clone();
            root.push(path);
            root
        } else {
            path
        };
        // get the globs expansions and filter unreadable paths
        glob(path.to_str().unwrap_or_default())
            .context("Failed to match glob pattern")?
            .filter_map(Result::ok)
            .for_each(|p| {
                paths.insert(p);
            });
    }

    Ok(paths.iter().map(|p| p.to_owned()).collect())
}
/// tries to check is the provided package paths are relative, and adds
/// the plugin root dir to them to make a valid path
fn load_package_paths(paths: &[PathBuf], plugin_root: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            if p.is_relative() {
                let mut new = PathBuf::from(plugin_root);
                new.push(p);
                new
            } else {
                p.to_owned()
            }
        })
        .collect();

    let mut lua_match = PathBuf::from(plugin_root);
    lua_match.push("?.lua");
    paths.push(lua_match);

    let mut lua_init_match = PathBuf::from(plugin_root);
    lua_init_match.push("?/init.lua");
    paths.push(lua_init_match);

    paths
}
