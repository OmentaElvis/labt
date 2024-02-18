use serde::{Deserialize, Serialize};

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
