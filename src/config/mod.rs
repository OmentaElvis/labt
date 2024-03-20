use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
};
pub mod lock;
pub mod maven_metadata;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use toml_edit::{Document, TableLike};

use crate::submodules::resolvers::{get_default_resolvers, NetResolver, Resolver};

/// The entire project toml file,
/// This contains details about the project configurations,
/// dependencies and plugins
#[derive(Serialize, Deserialize, Debug)]
pub struct LabToml {
    /// Project details
    pub project: Project,
    /// The project dependencies in the form of
    /// \[dependencies\]
    /// artifactId = {dependency inline table}
    pub dependencies: Option<HashMap<String, Dependency>>,
    /// The list of configured resolvers
    /// ```toml
    /// [resolvers]
    /// central = {url= "https://repo1.maven.org/maven2", default= true}
    /// ```
    pub resolvers: Option<HashMap<String, ResolverTable>>,
    /// Defines a list of plugins to use for this project
    /// ```toml
    /// [plugins]
    /// core-java = {url = "https://gitlab.com/lab-tool/core-java", version="v0.1.0"}
    /// ```
    pub plugins: Option<HashMap<String, PluginTable>>,
}

/// The project details
#[derive(Serialize, Deserialize, Debug)]
pub struct Project {
    /// Name of the project
    pub name: String,
    /// A brief description of the project
    pub description: String,
    /// Android project version number
    pub version_number: i32,
    /// Android project version name
    pub version: String,
    /// The application package name
    pub package: String,
}

// a project build dependency
#[derive(Serialize, Deserialize, Debug)]
pub struct Dependency {
    /// A redundant artifact id since it can be infered from the
    /// toml dependency key. If specified, then use it instead of infered key
    pub artifact_id: Option<String>,
    /// The project group id
    pub group_id: String,
    /// Project version
    pub version: String,
    /// The project dependency type i.e. jar, aar etc.
    pub dep_type: Option<String>,
    /// Where to fetch the project
    pub resolver: Option<String>,
}

/// A resolver table
#[derive(Serialize, Deserialize, Debug)]
pub struct ResolverTable {
    /// The repo url
    pub url: String,
    /// Is this repo to be treated as a default resolver
    /// for unspecified dependencies
    #[serde(default)]
    pub priority: i32,
}

/// The plugin toml table,
/// Either url or path should be provided for valid declaration
#[derive(Serialize, Deserialize, Debug)]
pub struct PluginTable {
    /// The repo url or local path where to fetch the plugin
    pub location: Option<String>,
    /// The plugin version to fetch
    pub version: String,
}

const LABT_TOML_FILE_NAME: &str = "Labt.toml";
const VERSION_STRING: &str = "version";
const GROUP_ID_STRING: &str = "group_id";
const DEPENDENCIES_STRING: &str = "dependencies";
const LOCATION_STRING: &str = "location";
const PLUGINS_STRING: &str = "plugins";

/// Reads Labt.toml from the current working directory, and returns
/// its contents as string
///
/// # Errors
///
/// This function will return an error if IO related error is encountered.
pub fn get_config_string() -> anyhow::Result<String> {
    let mut path = std::env::current_dir().context(format!(
        "Failed opening current working directory for {}",
        LABT_TOML_FILE_NAME
    ))?;
    path.push(LABT_TOML_FILE_NAME);

    let mut file = File::open(&path).context(format!(
        "Failed opening {} at {} is this a valid Labt project directory?",
        LABT_TOML_FILE_NAME,
        path.into_os_string().into_string().unwrap_or(String::new())
    ))?;

    let mut toml_string = String::new();
    file.read_to_string(&mut toml_string)
        .context(format!("Failed reading {}", LABT_TOML_FILE_NAME))?;

    Ok(toml_string)
}

/// Serializes Labt.toml in the current directory to a [`LabToml`] object
///
/// # Errors
///
/// This function will return an error if Serialization fails or IO error is
/// encountered from [`get_config_string()`]
pub fn get_config() -> anyhow::Result<LabToml> {
    let toml_string = get_config_string()?;
    let toml: LabToml =
        toml::from_str(&toml_string).context(format!("Failed parsing {}", LABT_TOML_FILE_NAME))?;
    Ok(toml)
}

/// Reads Labt.toml and serializes it to a [`toml_edit::Document`]. This is editable
/// and should be used to write to the toml
///
/// # Errors
///
/// This function will return an error if Serialization fails or IO error ia
/// encountered from [`get_config_string()`]
pub fn get_editable_config() -> anyhow::Result<Document> {
    let toml_string = get_config_string()?;
    let toml = toml_string
        .parse::<Document>()
        .context(format!("Failed parsing {}", LABT_TOML_FILE_NAME))?;

    Ok(toml)
}

/// Reads Labt.toml for configured resolvers and adds them to the default
/// resolvers or overrides them if matched with internal resolvers
pub fn get_resolvers() -> anyhow::Result<Vec<Box<dyn Resolver>>> {
    let config = get_config().context("Failed to get the project config")?;
    get_resolvers_from_config(&config).context("Failed to get resolvers from project config")
}

/// Reads config for configured resolvers and adds them to the default
/// resolvers or overrides them if matched with internal resolvers
/// useful to avoid parsing Labt.toml again if already parsed
pub fn get_resolvers_from_config(config: &LabToml) -> anyhow::Result<Vec<Box<dyn Resolver>>> {
    let mut resolvers =
        get_default_resolvers().context("Failed to initialize default resolvers")?;

    if let Some(config_resolvers) = &config.resolvers {
        for (name, resolver) in config_resolvers {
            let mut net_resolver = NetResolver::init(name.as_str(), resolver.url.as_str())
                .context(format!(
                    "Failed to initialize resolver {} for repo at {}",
                    name, resolver.url
                ))?;
            // update priority as configured
            net_resolver.set_priority(resolver.priority);

            let m_resolver: Box<dyn Resolver> = Box::new(net_resolver);

            // check default resolvers if this resolver exists,
            if let Some((index, _)) = resolvers
                .iter()
                .enumerate()
                .find(|(_, res)| res.get_name() == name.clone())
            {
                // just override the default resolver
                resolvers[index] = m_resolver;
            } else {
                // the resolver does not exist on default resolvers
                resolvers.push(m_resolver);
            }
        }
    }

    // reverse sort the resolvers based on priority
    // highest priority value = top of vec
    resolvers.sort_by_key(|b| std::cmp::Reverse(b.get_priority()));

    Ok(resolvers)
}

pub fn add_dependency_to_config(
    group_id: String,
    artifact_id: String,
    version: String,
) -> anyhow::Result<()> {
    use toml_edit::value;
    use toml_edit::InlineTable;
    use toml_edit::Item;
    use toml_edit::Table;
    // now add the dependency to toml
    let mut config = get_editable_config()?;

    let mut inline_table = InlineTable::new();
    inline_table.insert(VERSION_STRING, version.into());
    inline_table.insert(GROUP_ID_STRING, group_id.into());

    if config.contains_table(DEPENDENCIES_STRING) {
        config[DEPENDENCIES_STRING][artifact_id] = value(inline_table);
    } else {
        let mut table = Table::new();
        table.insert(&artifact_id, value(inline_table));
        config.insert(DEPENDENCIES_STRING, Item::Table(table));
    }

    let mut path = std::env::current_dir()?;
    path.push(LABT_TOML_FILE_NAME);
    let mut file = File::create(path)?;
    file.write_all(config.to_string().as_bytes())?;

    Ok(())
}
/// Adds this plugin to the project config
/// Returns an error if underlying IO and parsing operations fail.
pub fn add_plugin_to_config(name: String, version: String, location: String) -> anyhow::Result<()> {
    use toml_edit::value;
    use toml_edit::InlineTable;
    use toml_edit::Item;
    use toml_edit::Table;

    let mut config = get_editable_config().context("Failed to get project config")?;
    let mut inline_table = InlineTable::new();
    inline_table.insert(VERSION_STRING, version.into());
    inline_table.insert(LOCATION_STRING, location.into());

    if config.contains_table(PLUGINS_STRING) {
        config[PLUGINS_STRING][name] = value(inline_table);
    } else {
        let mut table = Table::new();
        table.insert(&name, value(inline_table));
        config.insert(PLUGINS_STRING, Item::Table(table));
    }

    let mut path = std::env::current_dir().context("Failed to get current working directory")?;
    path.push(LABT_TOML_FILE_NAME);
    let mut file = File::create(path).context(format!(
        "Failed to create {} config file",
        LABT_TOML_FILE_NAME
    ))?;
    file.write_all(config.to_string().as_bytes())
        .context(format!("Failed to write to {} file", LABT_TOML_FILE_NAME))?;

    Ok(())
}
/// Removes plugin from the project config
pub fn remove_plugin_from_config(name: String) -> anyhow::Result<()> {
    let mut config = get_editable_config().context("Failed to get project config")?;
    // Remove plugin from config
    if let Some(table) = config.get_mut(PLUGINS_STRING) {
        if let Some(table) = table.as_table_mut() {
            // remove this entry
            table.remove(name.as_str());

            let mut path =
                std::env::current_dir().context("Failed to get current working directory")?;
            path.push(LABT_TOML_FILE_NAME);
            // open config dile
            let mut file = File::create(path).context(format!(
                "Failed to create {} config file",
                LABT_TOML_FILE_NAME
            ))?;
            file.write_all(config.to_string().as_bytes())
                .context(format!("Failed to write to {} file", LABT_TOML_FILE_NAME))?;
        }
    }

    Ok(())
}

#[test]
fn get_resolvers_from_config_test() {
    let config = LabToml {
        dependencies: None,
        project: Project {
            name: String::from("labt"),
            description: String::new(),
            version_number: 0,
            version: String::from("0.0"),
            package: String::from("com.gitlab.labtool"),
        },
        resolvers: Some(HashMap::from([
            (
                String::from("local"),
                ResolverTable {
                    url: String::from("http://localhost/maven2"),
                    priority: 99,
                },
            ),
            (
                String::from("repo1"),
                ResolverTable {
                    url: String::from("http://example.com/maven2"),
                    priority: 2,
                },
            ),
            // ovveride internal resolver
            (
                String::from("google"),
                ResolverTable {
                    // change the url
                    url: String::from("https://maven.google.com/new-url"),
                    // above cache resolver
                    priority: 11,
                },
            ),
        ])),
        plugins: None,
    };

    let resolvers = get_resolvers_from_config(&config).expect("Failed to get resolvers");

    // local should be at top
    assert_eq!(resolvers[0].get_name(), String::from("local"));
    // followed by google resolver
    assert_eq!(resolvers[1].get_name(), String::from("google"));
    assert_eq!(resolvers[2].get_name(), String::from("cache"));
    assert_eq!(resolvers[3].get_name(), String::from("repo1"));
    assert_eq!(resolvers[4].get_name(), String::from("central"));

    // check priorities
    assert_eq!(
        resolvers
            .iter()
            .map(|res| res.get_priority())
            .collect::<Vec<i32>>(),
        vec![99, 11, 10, 2, 1]
    );

    // TODO check urls since i did not add an easy way of getting back urls from resolves
}
