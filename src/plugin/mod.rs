use std::{collections::HashMap, path::PathBuf};

use anyhow::Context;

use tokio::fs::read_to_string;

use crate::{get_home, submodules::build::Step};

use self::{config::PluginToml, executable::ExecutableLua};

pub mod config;
pub mod executable;
pub mod functions;

#[derive(Debug)]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub step: Step,
    pub priority: i32,
    /// The files to check for changes during a build step
    pub dependents: Option<(Vec<PathBuf>, Vec<PathBuf>)>,
    /// package paths
    pub package_paths: Vec<PathBuf>,
}

impl Plugin {
    pub fn new(name: String, version: String, path: PathBuf, step: Step) -> Self {
        Plugin {
            name,
            version,
            path,
            step,
            priority: 0,
            dependents: None,
            package_paths: vec![],
        }
    }
    pub fn load(&self) -> anyhow::Result<ExecutableLua> {
        let mut exe = ExecutableLua::new(self.path.clone(), &self.package_paths);
        exe.set_build_step(self.step);
        exe.load_labt_table()
            .context("Error injecting labt table into lua context")?;
        Ok(exe)
    }
}

/// Loads plugin config into the `Plugin` struct by TOML Deserialization
///
/// # Errors
///
/// This function will return an error if IO Error occurs or parsing error of the plugin toml
async fn load(root: PathBuf) -> anyhow::Result<PluginToml> {
    let mut path = root.clone();
    path.push("plugin.toml");
    let file_string = read_to_string(&path).await?;
    let mut plugin: PluginToml = toml_edit::de::from_str(file_string.as_str())?;
    plugin.path = root;

    Ok(plugin)
}

/// Loads plugins from the plugin folder. It internally loads the plugin
/// configs asynchronously/parallel and returns the list of plugins
///
/// # Errors
///
/// This function will return an error if IO error occurs on underlying
/// functions or a parsing error occurs on the plugins config
pub fn load_plugins_config() -> anyhow::Result<Vec<PluginToml>> {
    use anyhow::Ok;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Error creating a tokio runtime")?;

    let plugins = runtime
        .block_on(async {
            let mut plugins: Vec<PluginToml> = vec![];
            let mut path = get_home().context("Error loading labt home directory")?;
            path.push("plugins");

            let paths = (path
                .read_dir()
                .context("Error listing plugins directory contents on labt home")?)
            .flatten();
            let mut handlers = vec![];

            for dir in paths {
                if dir.file_type()?.is_dir() {
                    handlers.push((dir.path(), tokio::spawn(load(dir.path()))));
                }
            }

            for (dir, handler) in handlers {
                let plugin_result = handler.await?;
                let plugin =
                    plugin_result.context(format!("Error parsing plugin config at {:?}", dir))?;
                plugins.push(plugin);
            }

            Ok(plugins)
        })
        .context("Plugin config loader worker threads failed")?;
    Ok(plugins)
}
pub fn load_plugins_from_paths(paths: Vec<PathBuf>) -> anyhow::Result<Vec<PluginToml>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Error creating a tokio runtime")?;

    let plugins = runtime
        .block_on(async {
            let mut plugins: Vec<PluginToml> = vec![];
            let mut handlers = vec![];

            for path in &paths {
                handlers.push((path, tokio::spawn(load(path.clone()))));
            }

            for (dir, handler) in handlers {
                let plugin_result = handler.await?;
                let plugin =
                    plugin_result.context(format!("Error parsing plugin config at {:?}", dir))?;
                plugins.push(plugin);
            }

            Ok::<Vec<PluginToml>, anyhow::Error>(plugins)
        })
        .context("Plugin config loader worker threads failed")?;

    Ok(plugins)
}

/// Loads the plugins from plugins list provided, then proceeds to group them into
/// their respective execution steps
///
/// # Errors
///
/// This function will return an error if underlying `load_plugins_config()` errors
pub fn load_plugins(configs: Vec<PluginToml>) -> anyhow::Result<HashMap<Step, Vec<Plugin>>> {
    let mut plugins: HashMap<Step, Vec<Plugin>> = HashMap::new();

    for config in configs {
        let plugin_steps = config
            .get_steps()
            .context("Unable to parse build stages from plugins")?;
        for plugin in plugin_steps {
            if let Some(step_vec) = plugins.get_mut(&plugin.step) {
                // update the plugin vector
                step_vec.push(plugin);
            } else {
                // the key was not found, so create a new record
                plugins.insert(plugin.step, vec![plugin]);
            }
        }
    }

    Ok(plugins)
}
