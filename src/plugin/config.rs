use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    path::{Path, PathBuf},
    rc::Rc,
    str::FromStr,
};

use anyhow::{bail, Context};
use glob::glob;
use serde::{Deserialize, Serialize};
use toml_edit::{value, Document};

use crate::{
    config::repository::{ChannelType, Revision},
    get_project_root,
    pom::VersionRange,
    submodules::{
        build::Step,
        sdk::{toml_strings, GOOGLE_REPO_NAME_STR},
        sdkmanager::{installed_list::RepositoryInfo, ToId, ToIdLong},
    },
};

use super::Plugin;

pub(super) const NAME: &str = "name";
pub(super) const VERSION: &str = "version";
pub(super) const LABT: &str = "labt";
pub(super) const STAGE: &str = "stage";
pub(super) const FILE: &str = "file";
pub(super) const PRIORITY: &str = "priority";
pub(super) const INPUTS: &str = "inputs";
pub(super) const OUTPUTS: &str = "outputs";
pub(super) const PACKAGE_PATHS: &str = "package_paths";
pub(super) const SDK: &str = "sdk";
pub(super) const REPO: &str = "repo";
pub(super) const PATH: &str = "path";
pub(super) const CHANNEL: &str = "channel";
pub(super) const UNSAFE: &str = "unsafe";
pub(super) const INIT: &str = "init";
pub(super) const TEMPLATES: &str = "templates";

const PRE: &str = "pre";
const AAPT: &str = "aapt";
const COMPILE: &str = "compile";
const DEX: &str = "dex";
const BUNDLE: &str = "bundle";
const POST: &str = "post";

/// The sdk entries that this plugin requires
#[derive(PartialEq, Debug, Clone)]
pub struct SdkEntry {
    pub repo: String,
    pub name: String,
    pub path: String,
    pub version: Revision,
    pub channel: ChannelType,
}

impl Default for SdkEntry {
    fn default() -> Self {
        SdkEntry {
            repo: GOOGLE_REPO_NAME_STR.to_string(),
            name: String::default(),
            path: String::default(),
            version: Revision::default(),
            channel: ChannelType::Unset,
        }
    }
}

impl ToId for SdkEntry {
    fn create_id(&self) -> (&String, &Revision, &ChannelType) {
        (&self.path, &self.version, &self.channel)
    }
}
impl ToIdLong for SdkEntry {
    fn create_id(&self) -> (&String, &String, &Revision, &ChannelType) {
        (&self.repo, &self.path, &self.version, &self.channel)
    }
}

#[derive(Default, Debug)]
pub struct PluginToml {
    /// plugin name
    pub name: String,
    /// plugin version
    pub version: String,
    /// plugin states
    pub stages: HashMap<Step, PluginStage>,

    /// A list of plugin specified repositories
    pub sdk_repo: HashMap<String, RepositoryInfo>,

    /// plugin sdk dependencies
    pub sdk: Vec<SdkEntry>,

    /// this plugin templating script
    pub init: Option<PluginInit>,

    pub path: PathBuf,
    /// Paths to search for required lua modules
    pub package_paths: Option<Vec<PathBuf>>,
    /// Enable unsafe lua api for entire plugin
    pub enable_unsafe: bool,
    /// required Labt version
    pub labt: Option<VersionRange>,
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

#[derive(Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginStage {
    /// file containing the entry point
    pub file: PathBuf,
    /// plugin priority
    pub priority: i32,
    /// The input files that we should watch for changes
    pub inputs: Option<Vec<String>>,
    /// The output files that we should ensure that it is uptodate
    pub outputs: Option<Vec<String>>,
    /// Enable unsafe lua api
    #[serde(rename = "unsafe", default)]
    pub enable_unsafe: bool,
}

#[derive(Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginInit {
    /// File for the template based initialization
    pub file: PathBuf,
    /// The templates directory to load from
    pub templates: Option<String>,
}

impl PluginToml {
    /// Maps PluginToml stages into their [`Plugin`] representation.
    pub fn get_steps(self) -> anyhow::Result<Vec<Plugin>> {
        let mut steps = vec![];
        let sdk_rc = Rc::new(self.sdk.clone());

        /// because i cant accurately copy & paste these mappings
        /// from PluginToml stage to Plugin Step without creating a bug,
        /// let the macro repeat it, maybe im lazy
        macro_rules! map_plugin {
            [$($j:expr),*] => {
                $(
                // check if $i is set, if set then create a sub plugin
                if let Some(s) = &self.stages.get(&$j) {
                    // get this plugin root directory
                    let mut path = self.path.clone();
                    // push the plugin source path to path
                    path.push(s.file.clone());
                    // create a plugin and set its step as $j
                    let mut plugin = Plugin::new(self.name.clone(), self.version.clone(), path, $j);
                    plugin.sdk_dependencies = Rc::clone(&sdk_rc);
                    plugin.priority = s.priority;
                    plugin.unsafe_mode = self.enable_unsafe || s.enable_unsafe;
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
            Step::PRE,
            Step::AAPT,
            Step::COMPILE,
            Step::DEX,
            Step::BUNDLE,
            Step::POST
        ];

        Ok(steps)
    }
}
#[derive(Debug)]
enum PluginTomlErrorKind {
    /// name key is missing
    MissingKey(&'static str),
    /// name key is missing
    MissingTableKey(&'static str, String, Option<usize>),
    /// Failed to convert a value to string
    ToStringErr(&'static str, Option<&'static str>, Option<usize>),
    /// Failed to convert a value to bool
    ToBoolErr(&'static str, Option<&'static str>, Option<usize>),
    /// Invalid sdk string entry
    InvalidSdkKey(String, String),
    /// Invalid version string
    InvalidSdkVersionString(String),
    /// Invalid channel name
    InvalidChannel(String),
}
#[derive(Debug)]
struct PluginTomlError {
    kind: PluginTomlErrorKind,
}

impl PluginTomlError {
    pub fn new(kind: PluginTomlErrorKind) -> Self {
        Self { kind }
    }
}
impl std::error::Error for PluginTomlError {}

impl Display for PluginTomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            PluginTomlErrorKind::MissingKey(key) => {
                write!(f, "Missing {} which is required!", key)
            }
            PluginTomlErrorKind::MissingTableKey(key, table, position) => {
                if let Some(position) = position {
                    write!(
                        f,
                        "Missing {} for array table \"{}\" at position {} which is required",
                        key, table, position
                    )
                } else {
                    write!(
                        f,
                        "Missing {} for table \"{}\" which is required.",
                        key, table
                    )
                }
            }
            PluginTomlErrorKind::ToStringErr(key, Some(table), Some(position)) => {
                write!(
                    f,
                    "Failed to convert {} value as string in the table \"{}\" at position {}.",
                    key, table, position
                )
            }
            PluginTomlErrorKind::ToStringErr(key, None, _) => {
                write!(f, "Failed to convert {} value as string.", key)
            }
            PluginTomlErrorKind::ToBoolErr(key, Some(table), Some(position)) => {
                write!(
                    f,
                    "Failed to convert {} value as Boolean in the table \"{}\" at position {}.",
                    key, table, position
                )
            }
            PluginTomlErrorKind::ToBoolErr(key, None, _) => {
                write!(f, "Failed to convert {} value as boolean.", key)
            }
            PluginTomlErrorKind::InvalidSdkVersionString(key) => {
                write!(f, "Invalid version string for sdk dependency {}", key)
            }
            PluginTomlErrorKind::InvalidSdkKey(key, value) => {
                write!(
                    f,
                    "Invalid sdk key value for {}. Expected format is path:version:channel but found {}.",
                    key,value
                )
            }
            PluginTomlErrorKind::InvalidChannel(key) => {
                write!(f, "Invalid channel name for {} sdk dependency", key)
            }
            _ => {
                write!(f, "Unhandled error occured while parsing plugin.toml")
            }
        }
    }
}

impl Display for PluginToml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut doc = Document::new();
        doc.insert(NAME, value(self.name.as_str()));
        doc.insert(VERSION, value(self.version.as_str()));

        let mut sdk_table = toml_edit::Table::new();
        for sdk in &self.sdk {
            let mut table = toml_edit::InlineTable::new();
            table.insert(PATH, sdk.path.as_str().into());
            table.insert(VERSION, sdk.version.to_string().into());
            table.insert(CHANNEL, sdk.channel.to_string().into());
            sdk_table.insert(&sdk.name, value(table));
        }
        let mut stages = toml_edit::Table::new();
        let mut show_stage = |stage: Step| {
            if let Some(s) = self.stages.get(&stage) {
                let mut table = toml_edit::Table::new();
                table.insert(FILE, value(s.file.to_string_lossy().to_string().as_str()));
                table.insert(PRIORITY, value(s.priority as i64));
                if let Some(inputs) = &s.inputs {
                    let array = toml_edit::Array::from_iter(inputs.iter().map(|p| p.as_str()));
                    table.insert(INPUTS, value(array));
                }
                if let Some(outputs) = &s.outputs {
                    let array = toml_edit::Array::from_iter(outputs.iter().map(|p| p.as_str()));
                    table.insert(OUTPUTS, value(array));
                }
                if s.enable_unsafe {
                    table.insert(UNSAFE, value(true));
                }
                stages.insert(stage.to_string().as_str(), toml_edit::Item::Table(table));
            }
        };
        show_stage(Step::PRE);
        show_stage(Step::AAPT);
        show_stage(Step::COMPILE);
        show_stage(Step::DEX);
        show_stage(Step::BUNDLE);
        show_stage(Step::POST);

        doc.insert(SDK, toml_edit::Item::Table(sdk_table));
        doc.insert(STAGE, toml_edit::Item::Table(stages));
        write!(f, "{}", doc)
    }
}

impl FromStr for PluginToml {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let doc: Document = s.parse().context("Failed to parse plugin.toml file")?;

        let name = if doc.contains_key(NAME) {
            doc[NAME]
                .as_str()
                .ok_or_else(|| {
                    PluginTomlError::new(PluginTomlErrorKind::ToStringErr(NAME, None, None))
                })?
                .to_string()
        } else {
            bail!(PluginTomlError::new(PluginTomlErrorKind::MissingKey(NAME)));
        };

        let version = if doc.contains_key(VERSION) {
            doc[VERSION]
                .as_str()
                .ok_or_else(|| {
                    PluginTomlError::new(PluginTomlErrorKind::ToStringErr(NAME, None, None))
                })?
                .to_string()
        } else {
            bail!(PluginTomlError::new(PluginTomlErrorKind::MissingKey(
                VERSION
            )));
        };

        let enable_unsafe = if doc.contains_key(UNSAFE) {
            doc[UNSAFE].as_bool().ok_or_else(|| {
                PluginTomlError::new(PluginTomlErrorKind::ToBoolErr(NAME, None, None))
            })?
        } else {
            false
        };

        let labt_version = if doc.contains_key(LABT) {
            let v = doc[LABT]
                .as_str()
                .ok_or_else(|| {
                    PluginTomlError::new(PluginTomlErrorKind::ToStringErr(NAME, None, None))
                })?
                .to_string();

            Some(v.parse::<VersionRange>()?)
        } else {
            None
        };

        let package_paths = doc
            .get(PACKAGE_PATHS)
            .and_then(|f| f.as_array())
            .map(|paths| paths.iter().map(|p| PathBuf::from(p.to_string())).collect());

        let mut stages_map: HashMap<Step, PluginStage> = HashMap::new();
        if let Some(stages) = doc.get(STAGE).and_then(|s| s.as_table()) {
            let load_stage = |stage_name: &'static str, stages: &toml_edit::Table| {
                if let Some(stage) = stages.get(stage_name).and_then(|f| f.as_table()) {
                    let file = if let Some(file) = stage.get(FILE) {
                        PathBuf::from(
                            file.as_str()
                                .ok_or_else(|| {
                                    PluginTomlError::new(PluginTomlErrorKind::ToStringErr(
                                        FILE,
                                        Some(STAGE),
                                        None,
                                    ))
                                })?
                                .to_string(),
                        )
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                            FILE,
                            stage_name.to_string(),
                            None
                        )))
                    };

                    let priority = if let Some(priority) = stage.get(PRIORITY) {
                        priority.as_integer().ok_or_else(|| {
                            PluginTomlError::new(PluginTomlErrorKind::ToStringErr(
                                PRIORITY,
                                Some(STAGE),
                                None,
                            ))
                        })? as i32
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                            PRIORITY,
                            STAGE.to_string(),
                            None
                        )))
                    };

                    let inputs: Option<Vec<String>> = if let Some(inputs) = stage.get(INPUTS) {
                        inputs
                            .as_array()
                            .map(|array| array.iter().map(|a| a.to_string()).collect())
                    } else {
                        None
                    };

                    let outputs: Option<Vec<String>> = if let Some(outputs) = stage.get(OUTPUTS) {
                        outputs
                            .as_array()
                            .map(|array| array.iter().map(|a| a.to_string()).collect())
                    } else {
                        None
                    };
                    let enabe_unsafe_stage = if let Some(unsafe_mode) = stage.get(UNSAFE) {
                        unsafe_mode.as_bool().ok_or_else(|| {
                            PluginTomlError::new(PluginTomlErrorKind::ToBoolErr(NAME, None, None))
                        })?
                    } else {
                        false
                    };

                    Ok(Some(PluginStage {
                        file,
                        priority,
                        inputs,
                        outputs,
                        enable_unsafe: enabe_unsafe_stage,
                    }))
                } else {
                    Ok(None)
                }
            };

            let mut map_stage = |step: Step, key: &'static str| {
                if let Some(stage) = load_stage(key, stages)? {
                    stages_map.insert(step, stage);
                }
                Ok::<(), anyhow::Error>(())
            };

            map_stage(Step::PRE, PRE)?;
            map_stage(Step::AAPT, AAPT)?;
            map_stage(Step::COMPILE, COMPILE)?;
            map_stage(Step::DEX, DEX)?;
            map_stage(Step::BUNDLE, BUNDLE)?;
            map_stage(Step::POST, POST)?;
        };

        let mut sdk_deps: Vec<SdkEntry> = Vec::new();
        if let Some(table) = doc.get(SDK).and_then(|s| s.as_table()) {
            for (key, value) in table.iter() {
                let mut sdk = SdkEntry {
                    name: key.to_string(),
                    ..Default::default()
                };
                if value.is_str() {
                    // must be a sdk package id path:version:channel
                    let value = value.as_str().unwrap();
                    let segments: Vec<&str> = value.splitn(4, ':').collect();
                    let length = segments.len();

                    let mut iter = segments.iter();
                    if length >= 4 {
                        // if length is at 4 then the first is
                        if let Some(repo) = iter.next() {
                            sdk.repo = repo.to_string();
                        } else {
                            bail!(PluginTomlError::new(PluginTomlErrorKind::InvalidSdkKey(
                                key.to_string(),
                                value.to_string()
                            )));
                        }
                    }

                    if let Some(path) = iter.next() {
                        sdk.path = path.to_string();
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::InvalidSdkKey(
                            key.to_string(),
                            value.to_string()
                        )));
                    }
                    // revision
                    if let Some(revision) = iter.next() {
                        sdk.version = revision.parse().context(PluginTomlError::new(
                            PluginTomlErrorKind::InvalidSdkVersionString(key.to_string()),
                        ))?;
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::InvalidSdkKey(
                            key.to_string(),
                            value.to_string()
                        )));
                    }
                    // channel
                    if let Some(channel) = iter.next() {
                        sdk.channel = channel.parse().context(PluginTomlError::new(
                            PluginTomlErrorKind::InvalidChannel(key.to_string()),
                        ))?;
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::InvalidSdkKey(
                            key.to_string(),
                            value.to_string()
                        )));
                    }
                } else if value.is_table_like() {
                    let value = value.as_table_like().unwrap();
                    if let Some(repo) = value.get(REPO).and_then(|p| p.as_str()) {
                        sdk.repo = repo.to_string();
                    }

                    if let Some(path) = value.get(PATH).and_then(|p| p.as_str()) {
                        sdk.path = path.to_string();
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                            PATH,
                            format!("sdk.{}", key),
                            None
                        )))
                    }

                    if let Some(version) = value.get(VERSION).and_then(|p| p.as_str()) {
                        sdk.version = version.parse().context(PluginTomlError::new(
                            PluginTomlErrorKind::InvalidSdkVersionString(key.to_string()),
                        ))?;
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                            VERSION,
                            format!("sdk.{}", key),
                            None
                        )))
                    }

                    if let Some(channel) = value.get(CHANNEL).and_then(|p| p.as_str()) {
                        sdk.channel = channel.parse().context(PluginTomlError::new(
                            PluginTomlErrorKind::InvalidChannel(key.to_string()),
                        ))?;
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                            CHANNEL,
                            format!("sdk.{}", key),
                            None
                        )))
                    }
                }
                sdk_deps.push(sdk);
            }
        }
        let mut repositories: HashMap<String, RepositoryInfo> = HashMap::new();
        if doc.contains_array_of_tables(toml_strings::REPOSITORY) {
            if let Some(repos) = doc[toml_strings::REPOSITORY].as_array_of_tables() {
                for (i, repo_table) in repos.iter().enumerate() {
                    let name = if let Some(name) = repo_table.get(toml_strings::NAME) {
                        name.as_str()
                            .ok_or_else(|| {
                                PluginTomlError::new(PluginTomlErrorKind::ToStringErr(
                                    toml_strings::NAME,
                                    Some(toml_strings::REPOSITORY),
                                    Some(i),
                                ))
                            })?
                            .to_string()
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                            toml_strings::NAME,
                            toml_strings::REPOSITORY.to_string(),
                            Some(i)
                        )));
                    };
                    let url = if let Some(url) = repo_table.get(toml_strings::URL) {
                        url.as_str()
                            .ok_or_else(|| {
                                PluginTomlError::new(PluginTomlErrorKind::ToStringErr(
                                    toml_strings::URL,
                                    None,
                                    Some(i),
                                ))
                            })?
                            .to_string()
                    } else {
                        bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                            toml_strings::URL,
                            toml_strings::REPOSITORY.to_string(),
                            Some(i)
                        )));
                    };

                    repositories.insert(
                        name,
                        RepositoryInfo {
                            url,
                            accepted_licenses: HashSet::new(),
                            path: PathBuf::default(),
                        },
                    );
                }
            }
        }

        let init = if doc.contains_table(INIT) {
            if let Some(table) = doc[INIT].as_table() {
                let file = if let Some(file) = table.get(FILE) {
                    file.as_str()
                        .ok_or_else(|| {
                            PluginTomlError::new(PluginTomlErrorKind::ToStringErr(FILE, None, None))
                        })?
                        .to_string()
                } else {
                    bail!(PluginTomlError::new(PluginTomlErrorKind::MissingTableKey(
                        FILE,
                        toml_strings::REPOSITORY.to_string(),
                        None
                    )));
                };
                let templates = if let Some(templates) = table.get(TEMPLATES) {
                    Some(
                        templates
                            .as_str()
                            .ok_or_else(|| {
                                PluginTomlError::new(PluginTomlErrorKind::ToStringErr(
                                    FILE, None, None,
                                ))
                            })?
                            .to_string(),
                    )
                } else {
                    None
                };

                Some(PluginInit {
                    file: PathBuf::from(file),
                    templates,
                })
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            name,
            version,
            init,
            stages: stages_map,
            path: PathBuf::default(),
            package_paths,
            sdk: sdk_deps,
            enable_unsafe,
            labt: labt_version,
            sdk_repo: repositories,
        })
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
pub fn load_package_paths(paths: &[PathBuf], plugin_root: &Path) -> Vec<PathBuf> {
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

#[test]
fn parse_plugin_toml_from_string() {
    let toml = r#"
name="example"
version="0.1.0"
author="omentum"
labt=">=0.3.4"

[sdk]
build = "build-tools:33.0.2:stable"
platform = {path = "platform-tools", version="35.0.2.0",  channel="stable"}

[sdk.cmd]
path = "cmdline-tools;latest"
version = "16.0.0.1"
channel = "stable"

[init]
file = "template.lua"
templates = "my-templates"

# pre build
[stage.pre]
file="pre.lua"
priority=1
unsafe = true

# android asset packaging tool step.
[stage.aapt]
file="aapt.lua"
priority=1

# java compilation
[stage.compile]
file="compile.lua"
priority=1

# dexing
[stage.dex]
file="dex.lua"
priority=1

# bundling
[stage.bundle]
file="bundle.lua"
priority=1

# post build
[stage.post]
file="post.lua"
priority=1       
"#;

    let plugin: PluginToml = toml.parse().unwrap();
    assert_eq!(plugin.name, String::from("example"));
    assert_eq!(plugin.version, String::from("0.1.0"));
    assert_eq!(plugin.path, PathBuf::default());
    assert_eq!(plugin.labt, Some(VersionRange::Ge(String::from("0.3.4"))));
    assert_eq!(plugin.sdk.len(), 3);

    assert_eq!(
        plugin.init,
        Some(PluginInit {
            file: PathBuf::from("template.lua"),
            templates: Some(String::from("my-templates"))
        })
    );

    let mut sdks = plugin.sdk.iter();
    assert_eq!(
        sdks.next(),
        Some(&SdkEntry {
            name: String::from("build"),
            path: String::from("build-tools"),
            version: "33.0.2".parse().unwrap(),
            channel: ChannelType::Stable,
            ..Default::default()
        })
    );
    assert_eq!(
        sdks.next(),
        Some(&SdkEntry {
            name: String::from("platform"),
            path: String::from("platform-tools"),
            version: "35.0.2.0".parse().unwrap(),
            channel: ChannelType::Stable,
            ..Default::default()
        })
    );
    assert_eq!(
        sdks.next(),
        Some(&SdkEntry {
            name: String::from("cmd"),
            path: String::from("cmdline-tools;latest"),
            version: "16.0.0.1".parse().unwrap(),
            channel: ChannelType::Stable,
            ..Default::default()
        })
    );
    assert_eq!(sdks.next(), None);

    assert_eq!(
        plugin.stages.get(&Step::AAPT),
        Some(&PluginStage {
            file: PathBuf::from("aapt.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        })
    );
    assert_eq!(
        plugin.stages.get(&Step::PRE),
        Some(&PluginStage {
            file: PathBuf::from("pre.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: true,
        })
    );
    assert_eq!(
        plugin.stages.get(&Step::COMPILE),
        Some(&PluginStage {
            file: PathBuf::from("compile.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        })
    );
    assert_eq!(
        plugin.stages.get(&Step::DEX),
        Some(&PluginStage {
            file: PathBuf::from("dex.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        })
    );
    assert_eq!(
        plugin.stages.get(&Step::BUNDLE),
        Some(&PluginStage {
            file: PathBuf::from("bundle.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        })
    );
    assert_eq!(
        plugin.stages.get(&Step::POST),
        Some(&PluginStage {
            file: PathBuf::from("post.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        })
    );
}

#[test]
fn plugin_toml_to_string() {
    let mut plugin: PluginToml = PluginToml {
        name: String::from("example"),
        version: String::from("0.1.0"),
        stages: HashMap::new(),
        sdk: Vec::new(),
        path: PathBuf::new(),
        package_paths: None,
        enable_unsafe: false,
        labt: None,
        sdk_repo: HashMap::new(),
        init: None,
    };

    plugin.sdk.push(SdkEntry {
        name: String::from("build"),
        path: String::from("build-tools"),
        version: "33.0.2".parse().unwrap(),
        channel: ChannelType::Stable,
        ..Default::default()
    });
    plugin.sdk.push(SdkEntry {
        name: String::from("platform"),
        path: String::from("platform-tools"),
        version: "35.0.2.0".parse().unwrap(),
        channel: ChannelType::Stable,
        ..Default::default()
    });
    plugin.sdk.push(SdkEntry {
        name: String::from("cmd"),
        path: String::from("cmdline-tools;latest"),
        version: "16.0.0.1".parse().unwrap(),
        channel: ChannelType::Stable,
        ..Default::default()
    });

    plugin.stages.insert(
        Step::AAPT,
        PluginStage {
            file: PathBuf::from("aapt.lua"),
            priority: 1,
            inputs: Some(vec![String::from("**/*.xml")]),
            outputs: Some(vec![String::from("build/res.apk")]),
            enable_unsafe: false,
        },
    );

    plugin.stages.insert(
        Step::PRE,
        PluginStage {
            file: PathBuf::from("pre.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        },
    );

    plugin.stages.insert(
        Step::COMPILE,
        PluginStage {
            file: PathBuf::from("compile.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        },
    );

    plugin.stages.insert(
        Step::DEX,
        PluginStage {
            file: PathBuf::from("dex.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        },
    );

    plugin.stages.insert(
        Step::BUNDLE,
        PluginStage {
            file: PathBuf::from("bundle.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: false,
        },
    );

    plugin.stages.insert(
        Step::POST,
        PluginStage {
            file: PathBuf::from("post.lua"),
            priority: 1,
            inputs: None,
            outputs: None,
            enable_unsafe: true,
        },
    );
    let toml = r#"name = "example"
version = "0.1.0"

[sdk]
build = { path = "build-tools", version = "33.0.2.0", channel = "stable" }
platform = { path = "platform-tools", version = "35.0.2.0", channel = "stable" }
cmd = { path = "cmdline-tools;latest", version = "16.0.0.1", channel = "stable" }

[stage]

[stage.pre]
file = "pre.lua"
priority = 1

[stage.aapt]
file = "aapt.lua"
priority = 1
inputs = ["**/*.xml"]
outputs = ["build/res.apk"]

[stage.compile]
file = "compile.lua"
priority = 1

[stage.dex]
file = "dex.lua"
priority = 1

[stage.bundle]
file = "bundle.lua"
priority = 1

[stage.post]
file = "post.lua"
priority = 1
unsafe = true
"#;
    assert_eq!(toml, plugin.to_string().as_str());
}
