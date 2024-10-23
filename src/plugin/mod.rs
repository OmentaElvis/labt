use std::{collections::HashMap, path::PathBuf, rc::Rc, sync::OnceLock};

use anyhow::Context;

use tokio::fs::read_to_string;

use crate::{
    get_home,
    submodules::{
        build::Step,
        sdk::InstalledPackage,
        sdkmanager::{installed_list::InstalledList, ToId},
    },
};

use self::{
    config::{PluginToml, SdkEntry},
    executable::ExecutableLua,
};

pub mod api;
pub mod config;
pub mod executable;

/// A cached value of the InstalledList. It is initialized by get installed list
static INSTALLED_LIST: OnceLock<InstalledList> = OnceLock::new();
/// A cached value of the InstalledList in form of a hash map. It is initialized by get_installed_list_hash
static INSTALLED_LIST_HASH: OnceLock<HashMap<String, &InstalledPackage>> = OnceLock::new();

/// Returns the list of installed packages. Caches the return value for subsequent calls.
/// Returns an error if we fail to parse installed list toml
pub(super) fn get_installed_list() -> anyhow::Result<&'static InstalledList> {
    if let Some(list) = INSTALLED_LIST.get() {
        return Ok(list);
    }
    let list =
        InstalledList::parse_from_sdk().context("Failed to parse Installed sdk packages list.")?;
    Ok(INSTALLED_LIST.get_or_init(|| list))
}

/// Returns a hashmap of installed packages. Good for fast indexing.
/// Returns an error if we fail to get underlying installed list from `get_installed_list`
pub(super) fn get_installed_list_hash(
) -> anyhow::Result<&'static HashMap<String, &'static InstalledPackage>> {
    if let Some(list) = INSTALLED_LIST_HASH.get() {
        return Ok(list);
    }

    let installed_list =
        get_installed_list().context("Failed to get installed sdk packages list.")?;

    let mut list = HashMap::with_capacity(installed_list.packages.len());
    for package in &installed_list.packages {
        list.insert(package.to_id(), package);
    }

    Ok(INSTALLED_LIST_HASH.get_or_init(|| list))
}

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
    /// Unsafe mode enabled for this plugin
    pub unsafe_mode: bool,
    /// List of sdk modules to load
    pub sdk_dependencies: Rc<Vec<SdkEntry>>,
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
            unsafe_mode: false,
            sdk_dependencies: Rc::new(Vec::default()),
        }
    }
    pub fn load(&self) -> anyhow::Result<ExecutableLua> {
        let mut exe = ExecutableLua::new(
            self.path.clone(),
            &self.package_paths,
            Rc::clone(&self.sdk_dependencies),
            self.unsafe_mode,
        );
        exe.set_build_step(self.step);
        exe.load_sdk_loader()
            .context("Failed to inject LABt android sdk loader to lua require module.")?;
        exe.load_api_tables()
            .context("Error injecting api tables into lua context")?;
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
    let mut plugin: PluginToml = file_string
        .parse()
        .context("Failed to parse plugin.toml file.")?;
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
