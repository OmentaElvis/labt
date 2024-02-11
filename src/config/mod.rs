use serde::Serialize;

/// The entire project toml file,
/// This contains details about the project configurations,
/// dependencies and plugins
#[derive(Serialize)]
pub struct LabToml {
    /// Project details
    pub project: Project,
}

/// The project details
#[derive(Serialize)]
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
