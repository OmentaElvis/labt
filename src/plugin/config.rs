use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::submodules::build::Step;

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
}

impl PluginToml {
    pub fn get_steps(&self) -> Vec<Plugin> {
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

        steps
    }
}
