use std::fmt::Display;
use std::io::{Read, Write};
use std::str::FromStr;
use std::{fs::File, path::PathBuf};

use anyhow::bail;
use anyhow::Context;
use toml_edit::value;
use toml_edit::Array;
use toml_edit::ArrayOfTables;
use toml_edit::Document;
use toml_edit::Formatted;
use toml_edit::Item;
use toml_edit::Table;

use crate::get_project_root;
use crate::pom::VersionRange;
use crate::submodules::resolve::Constraint;
use crate::{pom::Scope, submodules::resolve::ProjectDep};

use self::strings::{
    ARTIFACT_ID, CONSTRAINTS, DEPENDENCIES, EXACT, EXCLUDES, GROUP_ID, LOCK_FILE, MAX, MIN,
    PACKAGING, PROJECT, SCOPE, URL, VERSION,
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
    pub const CONSTRAINTS: &str = "constraints";
    pub const MIN: &str = "min";
    pub const MAX: &str = "max";
    pub const EXACT: &str = "exact";
    pub const EXCLUDES: &str = "excludes";
    pub const LOCK_FILE: &str = "Labt.lock";
}
#[derive(Default, Clone, Debug)]
pub struct LabtLock {
    pub resolved: Vec<ProjectDep>,
}

impl FromStr for LabtLock {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lock = s.parse::<Document>().context("Unable to parse lock file")?;

        let mut m_lock = LabtLock::default();

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

                    if let Some(constraint) = dep.get(CONSTRAINTS) {
                        if let Some(table) = constraint.as_table_like() {
                            let mut c = Constraint::default();
                            // min
                            if let Some(min) = table.get(MIN).and_then(|d| d.as_str()) {
                                if min.starts_with('=') {
                                    c.min = Some((
                                        true,
                                        min.trim_start_matches('=').trim().to_string(),
                                    ));
                                } else {
                                    c.min = Some((false, min.to_string()));
                                }
                            }
                            // max
                            if let Some(max) = table.get(MAX).and_then(|d| d.as_str()) {
                                if max.starts_with('=') {
                                    c.max = Some((
                                        true,
                                        max.trim_start_matches('=').trim().to_string(),
                                    ));
                                } else {
                                    c.max = Some((false, max.to_string()));
                                }
                            }
                            // exact
                            if let Some(exact) = table.get(EXACT).and_then(|d| d.as_str()) {
                                c.exact = Some(exact.to_string());
                            }
                            // excludes
                            if let Some(excludes) = table.get(EXCLUDES).and_then(|d| d.as_array()) {
                                for exclude in excludes {
                                    if let Some(exclude) = exclude.as_str() {
                                        let mut split = exclude.split(',');
                                        let start = split.next().context(
                                            "Constraint exclude start range is not defined",
                                        )?;
                                        let start = start.parse::<VersionRange>().context("Failed to parse start range for an exclude from the given string.")?;

                                        let end = split.next().context(
                                            "Constraint exclude end range is not defined",
                                        )?;
                                        let end = end.parse::<VersionRange>().context("Failed to parse end range for an exclude from the given string.")?;

                                        c.exclusions.push((start, end));
                                    }
                                }
                            }
                            if !table.is_empty() {
                                project.constraints = Some(c);
                            }
                        }
                    }

                    m_lock.resolved.push(project);
                }
            }
        }
        Ok(m_lock)
    }
}

impl Display for LabtLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut doc = Document::new();
        let mut tables_array = ArrayOfTables::new();

        for dep in &self.resolved {
            let mut deps_array = Array::new();
            deps_array.decor_mut().set_suffix("\n");
            deps_array.extend(dep.dependencies.iter());

            let mut table = Table::new();
            table.insert(ARTIFACT_ID, value(&dep.artifact_id));
            table.insert(GROUP_ID, value(&dep.group_id));
            table.insert(VERSION, value(&dep.version));
            table.insert(SCOPE, value(&dep.scope));
            table.insert(URL, value(dep.get_root_url()));
            table.insert(PACKAGING, value(&dep.packaging));
            if let Some(constraint) = &dep.constraints {
                let mut c_table = toml_edit::InlineTable::new();
                if let Some((inclusive, min)) = &constraint.min {
                    if *inclusive {
                        c_table
                            .insert(MIN, value(format!("={min}").as_str()).into_value().unwrap());
                    } else {
                        c_table.insert(MIN, value(min.as_str()).into_value().unwrap());
                    }
                }
                if let Some((inclusive, max)) = &constraint.max {
                    if *inclusive {
                        c_table
                            .insert(MAX, value(format!("={max}").as_str()).into_value().unwrap());
                    } else {
                        c_table.insert(MAX, value(max.as_str()).into_value().unwrap());
                    }
                }
                if let Some(exact) = &constraint.exact {
                    c_table.insert(EXACT, value(exact.as_str()).into_value().unwrap());
                }

                let mut excludes = toml_edit::Array::new();
                for (start, end) in &constraint.exclusions {
                    excludes.push(format!("{},{}", start, end));
                }
                if !excludes.is_empty() {
                    c_table.insert(EXCLUDES, excludes.into());
                }

                table.insert(CONSTRAINTS, value(c_table));
            }

            table.insert(DEPENDENCIES, value(deps_array));
            tables_array.push(table);
        }

        doc.insert(PROJECT, Item::ArrayOfTables(tables_array));
        write!(f, "{}", doc)
    }
}

pub fn load_labt_lock() -> anyhow::Result<LabtLock> {
    let mut path: PathBuf = get_project_root()
        .context("Unable to get project root directory.")?
        .clone();
    path.push(LOCK_FILE);

    let mut file = File::open(path).context("Unable to open lock file")?;

    let resolved: LabtLock = load_lock_dependencies_with(&mut file)?;

    Ok(resolved)
}

pub fn load_lock_dependencies_with(file: &mut File) -> anyhow::Result<LabtLock> {
    let mut lock = String::new();
    file.read_to_string(&mut lock)
        .context("Unable to read lock file contents")?;

    let lock = lock
        .parse::<LabtLock>()
        .context("Unable to parse lock file ")?;

    Ok(lock)
}

pub fn write_lock(file: &mut File, lock: &LabtLock) -> anyhow::Result<()> {
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

#[test]
fn labt_lock_from_string() {
    let lock_str = r#"
[[project]]
artifact_id = "grpc-stub"
group_id = "io.grpc"
version = "1.66.0"
scope = "compile"
url = "https://repo1.maven.org/maven2/io/grpc/grpc-stub/1.66.0/"
constraints = { exact = "1.66.0" }
packaging = "jar"
dependencies = []


[[project]]
artifact_id = "javax.annotation-api"
group_id = "javax.annotation"
version = "1.3.2"
scope = "compile"
constraints = { min = "1.0.0", max = "=1.3.2"}
url = "https://repo1.maven.org/maven2/javax/annotation/javax.annotation-api/1.3.2/"
packaging = "jar"
dependencies = []


[[project]]
artifact_id = "annotation"
group_id = "androidx.annotation"
version = "1.1.0"
scope = "compile"
url = "https://maven.google.com/androidx/annotation/annotation/1.1.0/"
packaging = "jar"
constraints = { min = "=1.0.0", max = "1.5.0", excludes = [">1.2.0,<=1.3.0", ">=1.4,<=1.4"]}
dependencies = []


[[project]]
artifact_id = "cardview"
group_id = "androidx.cardview"
version = "1.0.0"
scope = "compile"
url = "https://maven.google.com/androidx/cardview/cardview/1.0.0/"
packaging = "aar"
dependencies = ["androidx.annotation:annotation:1.0.0"]
"#;
    let lock: LabtLock = lock_str.parse().unwrap();
    let mut deps = lock.resolved.iter();
    let project = deps.next().unwrap();
    assert_eq!(project.artifact_id, "grpc-stub".to_string());
    assert_eq!(project.group_id, "io.grpc".to_string());
    assert_eq!(project.version, "1.66.0".to_string());
    assert_eq!(project.scope, Scope::COMPILE);
    assert_eq!(
        project.base_url,
        "https://repo1.maven.org/maven2/".to_string()
    );
    assert_eq!(project.packaging, "jar".to_string());
    assert_eq!(
        project.constraints,
        Some(Constraint {
            exact: Some(String::from("1.66.0")),
            ..Default::default()
        })
    );

    // must be correct on all other fields so i am not testing everything
    let project = deps.next().unwrap();
    assert_eq!(
        project.constraints,
        Some(Constraint {
            min: Some((false, "1.0.0".to_string())),
            max: Some((true, "1.3.2".to_string())),
            ..Default::default()
        })
    );
    let project = deps.next().unwrap();
    assert_eq!(
        project.constraints,
        Some(Constraint {
            min: Some((true, "1.0.0".to_string())),
            max: Some((false, "1.5.0".to_string())),
            exclusions: vec![
                (
                    VersionRange::Gt("1.2.0".to_string()),
                    VersionRange::Le("1.3.0".to_string())
                ),
                (
                    VersionRange::Ge("1.4".to_string()),
                    VersionRange::Le("1.4".to_string())
                )
            ],
            ..Default::default()
        })
    );
    let project = deps.next().unwrap();
    assert_eq!(
        project.dependencies,
        vec!["androidx.annotation:annotation:1.0.0"]
    );
}
#[test]
fn labt_lock_to_string() {
    let expected = r#"[[project]]
artifact_id = "grpc-stub"
group_id = "io.grpc"
version = "1.66.0"
scope = "compile"
url = "https://repo1.maven.org/maven2/io/grpc/grpc-stub/1.66.0/"
packaging = "jar"
constraints = { exact = "1.66.0" }
dependencies = []


[[project]]
artifact_id = "javax.annotation-api"
group_id = "javax.annotation"
version = "1.3.2"
scope = "compile"
url = "https://repo1.maven.org/maven2/javax/annotation/javax.annotation-api/1.3.2/"
packaging = "jar"
constraints = { min = "1.0.0", max = "=1.3.2" }
dependencies = []


[[project]]
artifact_id = "annotation"
group_id = "androidx.annotation"
version = "1.1.0"
scope = "compile"
url = "https://maven.google.com/androidx/annotation/annotation/1.1.0/"
packaging = "jar"
constraints = { min = "=1.0.0", max = "1.5.0", excludes = [">1.2.0,<=1.3.0", ">=1.4,<=1.4"] }
dependencies = []


[[project]]
artifact_id = "cardview"
group_id = "androidx.cardview"
version = "1.0.0"
scope = "compile"
url = "https://maven.google.com/androidx/cardview/cardview/1.0.0/"
packaging = "aar"
dependencies = ["androidx.annotation:annotation:1.0.0"]

"#;

    let lock = LabtLock {
        resolved: vec![
            ProjectDep {
                artifact_id: "grpc-stub".to_string(),
                group_id: "io.grpc".to_string(),
                version: "1.66.0".to_string(),
                scope: Scope::COMPILE,
                base_url: "https://repo1.maven.org/maven2/".to_string(),
                constraints: Some(Constraint {
                    exact: Some("1.66.0".to_string()),
                    ..Default::default()
                }),
                packaging: "jar".to_string(),
                ..Default::default()
            },
            ProjectDep {
                artifact_id: "javax.annotation-api".to_string(),
                group_id: "javax.annotation".to_string(),
                version: "1.3.2".to_string(),
                scope: Scope::COMPILE,
                base_url: "https://repo1.maven.org/maven2/".to_string(),
                constraints: Some(Constraint {
                    min: Some((false, "1.0.0".to_string())),
                    max: Some((true, "1.3.2".to_string())),
                    ..Default::default()
                }),
                packaging: "jar".to_string(),
                ..Default::default()
            },
            ProjectDep {
                artifact_id: "annotation".to_string(),
                group_id: "androidx.annotation".to_string(),
                version: "1.1.0".to_string(),
                scope: Scope::COMPILE,
                base_url: "https://maven.google.com/".to_string(),
                constraints: Some(Constraint {
                    min: Some((true, "1.0.0".to_string())),
                    max: Some((false, "1.5.0".to_string())),
                    exclusions: vec![
                        (
                            VersionRange::Gt("1.2.0".to_string()),
                            VersionRange::Le("1.3.0".to_string()),
                        ),
                        (
                            VersionRange::Ge("1.4".to_string()),
                            VersionRange::Le("1.4".to_string()),
                        ),
                    ],
                    ..Default::default()
                }),
                packaging: "jar".to_string(),
                ..Default::default()
            },
            ProjectDep {
                artifact_id: "cardview".to_string(),
                group_id: "androidx.cardview".to_string(),
                version: "1.0.0".to_string(),
                scope: Scope::COMPILE,
                base_url: "https://maven.google.com/".to_string(),
                dependencies: vec!["androidx.annotation:annotation:1.0.0".to_string()],
                packaging: "aar".to_string(),
                ..Default::default()
            },
        ],
    };
    // println!("<<<Generated>>>\n{}<<expected>>\n{}", lock, expected);

    assert_eq!(lock.to_string(), expected.to_string());
}
