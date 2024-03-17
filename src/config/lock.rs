use std::io::{Read, Seek, Write};
use std::{env::current_dir, fs::File, io, path::PathBuf};

use anyhow::bail;
use anyhow::Context;
use toml_edit::value;
use toml_edit::Array;
use toml_edit::ArrayOfTables;
use toml_edit::Document;
use toml_edit::Formatted;
use toml_edit::Item;
use toml_edit::Table;

use crate::{pom::Scope, submodules::resolve::ProjectDep};

use self::strings::{
    ARTIFACT_ID, DEPENDENCIES, GROUP_ID, LOCK_FILE, PACKAGING, PROJECT, SCOPE, URL, VERSION,
};

/// containst string constants to be used in writing
/// and parsing lock files
pub mod strings {
    pub const GROUP_ID: &str = "group_id";
    pub const ARTIFACT_ID: &str = "artifact_id";
    pub const VERSION: &str = "version";
    pub const DEPENDENCIES: &str = "dependencies";
    pub const PROJECT: &str = "project";
    pub const SCOPE: &str = "scope";
    pub const URL: &str = "url";
    pub const PACKAGING: &str = "packaging";
    pub const LOCK_FILE: &str = "Labt.lock";
}

pub fn load_lock_dependencies() -> anyhow::Result<Vec<ProjectDep>> {
    let mut path: PathBuf = current_dir().context("Unable to open current directory")?;
    path.push(LOCK_FILE);

    let mut file = File::open(path).context("Unable to open lock file")?;

    let resolved: Vec<ProjectDep> = load_lock_dependencies_with(&mut file)?;

    Ok(resolved)
}

pub fn load_lock_dependencies_with(file: &mut File) -> anyhow::Result<Vec<ProjectDep>> {
    let mut resolved: Vec<ProjectDep> = vec![];

    let mut lock = String::new();
    file.read_to_string(&mut lock)
        .context("Unable to read lock file contents")?;

    let lock = lock
        .parse::<Document>()
        .context("Unable to parse lock file")?;

    if lock.contains_array_of_tables(PROJECT) {
        if let Some(table_arrays) = lock[PROJECT].as_array_of_tables() {
            let missing_err = |key: &str, position: usize| -> anyhow::Result<()> {
                bail!(
                    "Labt.lock: Missing {} in table at position {} ",
                    key,
                    position
                );
            };

            for dep in table_arrays.iter() {
                let mut project = ProjectDep::default();
                let position = dep.position().unwrap_or(0);

                // check for artifact_id
                if let Some(artifact_id) = dep.get(ARTIFACT_ID) {
                    project.artifact_id = artifact_id
                        .as_value()
                        .unwrap_or(&toml_edit::Value::String(Formatted::new(String::new())))
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                } else {
                    missing_err(ARTIFACT_ID, position)?;
                }

                // check for group_id
                if let Some(group_id) = dep.get(GROUP_ID) {
                    project.group_id = group_id
                        .as_value()
                        .unwrap_or(&toml_edit::Value::String(Formatted::new(String::new())))
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                } else {
                    missing_err(GROUP_ID, position)?;
                }

                // check for version
                if let Some(version) = dep.get(VERSION) {
                    project.version = version
                        .as_value()
                        .unwrap_or(&toml_edit::Value::String(Formatted::new(String::new())))
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                } else {
                    missing_err(VERSION, position)?;
                }
                // check for scope
                if let Some(scope) = dep.get(SCOPE) {
                    project.scope = Scope::from(
                        scope
                            .as_value()
                            .unwrap_or(&toml_edit::Value::from("compile")),
                    );
                }
                if let Some(url) = dep.get(URL) {
                    let url = url
                        .as_value()
                        .unwrap_or(&toml_edit::Value::from(""))
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    project.set_base_url_from_root(url);
                } else {
                    missing_err(URL, position)?;
                }
                if let Some(url) = dep.get(PACKAGING) {
                    project.packaging = url
                        .as_value()
                        .unwrap_or(&toml_edit::Value::from("jar"))
                        .as_str()
                        .unwrap_or("jar")
                        .to_string();
                } else {
                    project.packaging = String::from("jar");
                }

                if let Some(dependencies) = dep.get(DEPENDENCIES) {
                    if let Some(array) = dependencies.as_array() {
                        let mut deps = Vec::new();
                        deps.extend(array.iter().map(|d| d.as_str().unwrap_or("").to_string()));
                        project.dependencies = deps;
                    }
                }

                resolved.push(project);
            }
        }
    }
    Ok(resolved)
}

pub fn write_lock(file: &mut File, resolved: &[ProjectDep]) -> anyhow::Result<()> {
    let mut lock = String::new();
    file.read_to_string(&mut lock)
        .context("Unable to read lock file contents")?;

    let mut lock = lock
        .parse::<Document>()
        .context("Unable to parse lock file")?;

    // map dependencies ProjectTable to Tables and extend
    // the ArrayOfTables with the resulting iterator
    let mut tables_array = ArrayOfTables::new();
    tables_array.extend(resolved.iter().map(|dep| {
        let mut deps_array = Array::new();
        deps_array.decor_mut().set_suffix("\n");
        deps_array.extend(dep.dependencies.iter());

        let mut table = Table::new();
        table.insert(ARTIFACT_ID, value(&dep.artifact_id));
        table.insert(GROUP_ID, value(&dep.group_id));
        table.insert(VERSION, value(&dep.version));
        table.insert(SCOPE, value(&dep.scope));
        table.insert(URL, value(&dep.get_root_url()));
        table.insert(PACKAGING, value(&dep.packaging));
        table.insert(DEPENDENCIES, value(deps_array));
        table
    }));

    lock["project"] = Item::ArrayOfTables(tables_array);

    file.seek(io::SeekFrom::Start(0))?;
    file.write_all(lock.to_string().as_bytes())
        .context("Error writing lock file")?;

    Ok(())
}

impl From<&Scope> for toml_edit::Value {
    fn from(scope: &Scope) -> Self {
        match scope {
            Scope::COMPILE => Self::from("compile"),
            Scope::TEST => Self::from("test"),
            Scope::RUNTIME => Self::from("runtime"),
            Scope::SYSTEM => Self::from("system"),
            Scope::PROVIDED => Self::from("provided"),
            Scope::IMPORT => Self::from("import"),
        }
    }
}

impl From<&toml_edit::Value> for Scope {
    fn from(value: &toml_edit::Value) -> Self {
        let scope = value.as_str().unwrap_or("compile").to_lowercase();
        match scope.as_str() {
            "compile" => Self::COMPILE,
            "test" => Self::TEST,
            "runtime" => Self::RUNTIME,
            "system" => Self::SYSTEM,
            "provided" => Self::PROVIDED,
            "import" => Self::IMPORT,
            _ => Self::COMPILE,
        }
    }
}
