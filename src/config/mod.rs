use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
};
pub mod lock;
pub mod maven_metadata;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use toml_edit::Document;

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
}

const LABT_TOML_FILE_NAME: &str = "Labt.toml";
const VERSION_STRING: &str = "version";
const GROUP_ID_STRING: &str = "group_id";
const DEPENDENCIES_STRING: &str = "dependencies";

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
