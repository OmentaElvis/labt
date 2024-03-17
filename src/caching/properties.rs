use std::{
    fmt::Display,
    io::{Read, Write},
};

use anyhow::Context;
use toml_edit::{value, Document};

use crate::{
    config::lock::strings::{ARTIFACT_ID, DEPENDENCIES, GROUP_ID, PACKAGING, URL, VERSION},
    submodules::resolve::ProjectDep,
};

use super::Cache;

#[derive(Debug)]
pub enum PropertiesError {
    ParseError,
    IOError(String),
    LabtHomeError,
}

impl Display for PropertiesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LabtHomeError => {
                writeln!(
                    f,
                    "Labt home not initialized therefore missing cache folder"
                )
            }
            Self::ParseError => writeln!(f, "Failed to parse properties toml file"),
            Self::IOError(msg) => writeln!(f, "{}", msg),
        }
    }
}

pub fn write_properties(project: &ProjectDep) -> anyhow::Result<()> {
    let mut cache = Cache::new(
        project.group_id.clone(),
        project.artifact_id.clone(),
        project.version.clone(),
        super::CacheType::PROPERTIES,
    );
    cache
        .use_labt_home()
        .with_context(|| PropertiesError::LabtHomeError)?;
    // create .toml properties file
    let mut cache = cache.create().with_context(|| {
        PropertiesError::IOError("Failed to create properties toml file".to_string())
    })?;

    let mut table = toml_edit::table();
    table[GROUP_ID] = value(&project.group_id);
    table[ARTIFACT_ID] = value(&project.artifact_id);
    table[VERSION] = value(&project.version);
    table[URL] = value(&project.base_url);
    table[PACKAGING] = value(&project.packaging);

    let mut deps_array = toml_edit::Array::new();
    deps_array.extend(project.dependencies.iter());
    table[DEPENDENCIES] = value(deps_array);

    cache
        .write_all(table.to_string().as_bytes())
        .context(PropertiesError::IOError(
            "Failed to write properties file".to_string(),
        ))?;

    Ok(())
}

pub fn read_properties(project: &mut ProjectDep) -> anyhow::Result<()> {
    let mut cache = Cache::new(
        project.group_id.clone(),
        project.artifact_id.clone(),
        project.version.clone(),
        super::CacheType::PROPERTIES,
    );
    cache
        .use_labt_home()
        .context(PropertiesError::LabtHomeError)?;
    // create .toml properties file
    let mut cache = cache.open()?;
    let mut toml = String::new();
    cache
        .read_to_string(&mut toml)
        .context(PropertiesError::IOError(
            "Failed to read cache properties file".to_string(),
        ))?;

    let toml = toml
        .parse::<Document>()
        .context(PropertiesError::ParseError)?;

    if let Some(url) = toml.get(URL) {
        project.base_url = url
            .as_value()
            .unwrap_or(&toml_edit::Value::from(""))
            .as_str()
            .unwrap_or("")
            .to_string();
    }

    if let Some(url) = toml.get(PACKAGING) {
        project.packaging = url
            .as_value()
            .unwrap_or(&toml_edit::Value::from("jar"))
            .as_str()
            .unwrap_or("jar")
            .to_string();
    }

    if let Some(dependencies) = toml.get(DEPENDENCIES) {
        if let Some(array) = dependencies.as_array() {
            let mut deps = Vec::new();
            deps.extend(array.iter().map(|d| d.as_str().unwrap_or("").to_string()));
            project.dependencies = deps;
        }
    }

    Ok(())
}
