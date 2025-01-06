use anyhow::Context;
use anyhow::Result;
use quick_xml::{events::Event, Reader};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::io::BufReader;
use std::io::Read;
use std::str::FromStr;
use tokio::io::AsyncRead;
use version_compare::Version;

/// constants for common tags
mod tags {
    pub const ARTIFACT_ID: &[u8] = b"artifactId";
    pub const GROUP_ID: &[u8] = b"groupId";
    pub const VERSION: &[u8] = b"version";
    pub const DEPENDENCIES: &[u8] = b"dependencies";
    pub const DEPENDENCY_MANAGEMENT: &[u8] = b"dependencyManagement";
    pub const PROJECT: &[u8] = b"project";
    pub const DEPENDENCY: &[u8] = b"dependency";
    pub const EXCLUSIONS: &[u8] = b"exclusions";
    pub const EXCLUSION: &[u8] = b"exclusion";
    pub const PACKAGING: &[u8] = b"packaging";
    pub const OPTIONAL: &[u8] = b"optional";
    pub const SCOPE: &[u8] = b"scope";
    pub const COMPILE: &[u8] = b"compile";
    pub const TEST: &[u8] = b"test";
    pub const PROVIDED: &[u8] = b"provided";
    pub const IMPORT: &[u8] = b"import";
    pub const SYSTEM: &[u8] = b"system";
    pub const RUNTIME: &[u8] = b"runtime";
    pub const PROPERTIES: &[u8] = b"properties";
    pub const PARENT: &[u8] = b"parent";
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize)]
pub enum Scope {
    #[default]
    COMPILE,
    TEST,
    RUNTIME,
    SYSTEM,
    PROVIDED,
    IMPORT,
    UNKOWN(String),
}
impl FromStr for Scope {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::prelude::v1::Result<Self, Self::Err> {
        match s.as_bytes() {
            tags::COMPILE => Ok(Scope::COMPILE),
            tags::TEST => Ok(Scope::TEST),
            tags::PROVIDED => Ok(Scope::PROVIDED),
            tags::IMPORT => Ok(Scope::IMPORT),
            tags::SYSTEM => Ok(Scope::SYSTEM),
            tags::RUNTIME => Ok(Scope::RUNTIME),
            b"" => Ok(Scope::COMPILE),
            _ => Ok(Self::UNKOWN(s.to_string())),
        }
    }
}

impl Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scope = match self {
            Scope::COMPILE => tags::COMPILE,
            Scope::TEST => tags::TEST,
            Scope::PROVIDED => tags::PROVIDED,
            Scope::IMPORT => tags::IMPORT,
            Scope::SYSTEM => tags::SYSTEM,
            Scope::RUNTIME => tags::RUNTIME,
            Self::UNKOWN(s) => return write!(f, "{}", s),
        };

        write!(f, "{}", String::from_utf8_lossy(scope))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum VersionRange {
    Gt(String),
    Ge(String),
    Lt(String),
    Le(String),
    Eq(String),
}

impl PartialOrd for VersionRange {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let version_a = match self {
            VersionRange::Gt(v)
            | VersionRange::Ge(v)
            | VersionRange::Lt(v)
            | VersionRange::Le(v)
            | VersionRange::Eq(v) => v,
        };
        let version_b = match other {
            VersionRange::Gt(v)
            | VersionRange::Ge(v)
            | VersionRange::Lt(v)
            | VersionRange::Le(v)
            | VersionRange::Eq(v) => v,
        };

        let a = Version::from(version_a).unwrap();
        let b = Version::from(version_b).unwrap();

        // Since we are working with a virtual number line here, inequality symbols should be taken into account.
        // > is greater that >= as it moves up by 1
        // < is less than <= as it moves down
        // |------|------|------|------|
        // >=     >             <      <=

        match a.partial_cmp(&b) {
            Some(std::cmp::Ordering::Equal) => {
                // differentiate between symbols
                match (self, other) {
                    (VersionRange::Gt(_), VersionRange::Ge(_)) => Some(std::cmp::Ordering::Greater),
                    (VersionRange::Ge(_), VersionRange::Gt(_)) => Some(std::cmp::Ordering::Less),
                    (VersionRange::Le(_), VersionRange::Lt(_)) => Some(std::cmp::Ordering::Greater),
                    (VersionRange::Lt(_), VersionRange::Le(_)) => Some(std::cmp::Ordering::Less),
                    _ => Some(std::cmp::Ordering::Equal),
                }
            }
            other => other,
        }
    }
}

impl Display for VersionRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gt(v) => write!(f, ">{v}"),
            Self::Ge(v) => write!(f, ">={v}"),
            Self::Lt(v) => write!(f, "<{v}"),
            Self::Le(v) => write!(f, "<={v}"),
            Self::Eq(v) => write!(f, "{v}"),
        }
    }
}

impl FromStr for VersionRange {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::prelude::v1::Result<Self, Self::Err> {
        enum VersionRangeState {
            Start,
            Gt,
            Lt,
            Ge,
            Le,
            Eq,
        }
        let mut current_state = VersionRangeState::Start;
        let mut start_index = 0;

        for (i, c) in s.chars().enumerate() {
            match current_state {
                VersionRangeState::Start => match c {
                    ' ' => {
                        continue;
                    }
                    '>' => {
                        current_state = VersionRangeState::Gt;
                        start_index = i + 1;
                    }
                    '<' => {
                        current_state = VersionRangeState::Lt;
                        start_index = i + 1;
                    }
                    '=' => {
                        current_state = VersionRangeState::Eq;
                        start_index = i + 1;
                    }
                    _ => {
                        current_state = VersionRangeState::Eq;
                        start_index = i;
                    }
                },
                VersionRangeState::Gt => match c {
                    '=' => {
                        current_state = VersionRangeState::Ge;
                        start_index = i + 1;
                    }
                    _ => continue,
                },
                VersionRangeState::Lt => match c {
                    '=' => {
                        current_state = VersionRangeState::Le;
                        start_index = i + 1;
                    }
                    _ => {
                        continue;
                    }
                },
                _ => {}
            }
        }
        if start_index < s.len() {
            let version = s[start_index..].trim();

            if version.is_empty() {
                anyhow::bail!("An empty version was encountered which is invalid. ");
            }
            match current_state {
                VersionRangeState::Start => {
                    unreachable!(); // we already errored out above
                }
                VersionRangeState::Gt => Ok(Self::Gt(version.to_string())),
                VersionRangeState::Ge => Ok(Self::Ge(version.to_string())),
                VersionRangeState::Lt => Ok(Self::Lt(version.to_string())),
                VersionRangeState::Le => Ok(Self::Le(version.to_string())),
                VersionRangeState::Eq => Ok(Self::Eq(version.to_string())),
            }
        } else {
            anyhow::bail!("Encountered an inequality symbol without a version. ");
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq, Clone)]
/// Represents the different types of version requirements for dependencies.
///  `(,1.0]`  x <= 1.0
/// `1.0`  "Soft" requirement on 1.0 (just a recommendation - helps select the correct version if it matches all ranges)
/// `[1.0]` Hard requirement on 1.0
/// `[1.2,1.3]` is 1.2 <= x <= 1.3
/// `[1.0,2.0)` is 1.0 <= x < 2.0
/// `[1.5,)` is x >= 1.5
/// `(,1.0],[1.2,)` is x <= 1.0 or x >= 1.2. Multiple sets are comma-separated
/// `(,1.1),(1.1,)` is This excludes 1.1 if it is known not to work in combination with this library
pub enum VersionRequirement {
    /// Soft requirement for a specific version.
    ///
    /// This indicates a preference for the specified version, but allows
    /// for other versions to be used if they are required by other dependencies.
    ///
    /// Example: `1.0`
    Soft(String),

    Hard(Vec<VersionRange>),

    #[default]
    // The version was never set, so we should try to use already available hard
    // or the latest available
    Unset,
}

impl VersionRequirement {
    pub fn is_soft(&self) -> bool {
        matches!(self, Self::Soft(_))
    }
    pub fn is_hard(&self) -> bool {
        matches!(self, Self::Hard(_))
    }
    pub fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }
    fn flip_range(range: VersionRange) -> VersionRange {
        match range {
            VersionRange::Eq(v) => VersionRange::Eq(v),
            VersionRange::Gt(v) => VersionRange::Le(v),
            VersionRange::Ge(v) => VersionRange::Lt(v),
            VersionRange::Lt(v) => VersionRange::Ge(v),
            VersionRange::Le(v) => VersionRange::Gt(v),
        }
    }
}
impl Display for VersionRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Soft(soft) => write!(f, "{}", soft),
            Self::Hard(hard) => {
                // This is a nightmare
                // Here is what we are going to do. We are going to dump the ranges as is
                // without caring about syntatic sugars like [1.0, 2.0] which is seen by the parser as [1.0,),(,2.0]
                // because its the parser going to parse it anyway.
                let mut ranges: Vec<String> = Vec::new();
                for range in hard {
                    match range {
                        // We can only have one eq so short circuit here
                        VersionRange::Eq(eq) => return write!(f, "[{}]", eq),
                        VersionRange::Gt(v) => {
                            ranges.push(format!("({v},)"));
                        }
                        VersionRange::Ge(v) => {
                            ranges.push(format!("[{v},)"));
                        }
                        VersionRange::Lt(v) => {
                            ranges.push(format!("(,{v})"));
                        }
                        VersionRange::Le(v) => {
                            ranges.push(format!("(,{v}]"));
                        }
                    }
                }
                write!(f, "{}", ranges.join(","))
            }
            Self::Unset => write!(f, ""),
        }
    }
}

impl From<&Constraint> for VersionRequirement {
    fn from(value: &Constraint) -> Self {
        let mut v = VersionRequirement::Unset;
        // min
        if let Some((inclusive, min)) = &value.min {
            if let VersionRequirement::Hard(v) = &mut v {
                v.push(if *inclusive {
                    VersionRange::Ge(min.to_string())
                } else {
                    VersionRange::Gt(min.to_string())
                });
            } else {
                v = VersionRequirement::Hard(vec![if *inclusive {
                    VersionRange::Ge(min.to_string())
                } else {
                    VersionRange::Gt(min.to_string())
                }]);
            }
        }
        // max
        if let Some((inclusive, max)) = &value.max {
            if let VersionRequirement::Hard(v) = &mut v {
                v.push(if *inclusive {
                    VersionRange::Le(max.to_string())
                } else {
                    VersionRange::Lt(max.to_string())
                });
            } else {
                v = VersionRequirement::Hard(vec![if *inclusive {
                    VersionRange::Le(max.to_string())
                } else {
                    VersionRange::Lt(max.to_string())
                }]);
            }
        }
        // exact
        if let Some(exact) = &value.exact {
            v = VersionRequirement::Hard(vec![VersionRange::Eq(exact.to_string())]);
        }

        // exclusions
        for (start, end) in &value.exclusions {
            let start = Self::flip_range(start.clone());
            let end = Self::flip_range(end.clone());

            if let VersionRequirement::Hard(v) = &mut v {
                v.push(start);
                v.push(end);
            } else {
                v = VersionRequirement::Hard(vec![start, end]);
            }
        }

        v
    }
}

impl FromStr for VersionRequirement {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::prelude::v1::Result<Self, Self::Err> {
        let s = s.trim();

        if s.is_empty() {
            return Ok(VersionRequirement::Unset);
        }
        if !s.starts_with('(') && !s.starts_with('[') {
            return Ok(VersionRequirement::Soft(s.to_string()));
        }

        let mut versions = Vec::new();

        #[derive(Debug)]
        enum VersionParserState {
            InequalityStart,
            EqualityStart,
            Lt,
            Gt,
            Eq,
            Start,
        }

        let mut current_state = VersionParserState::Start;
        let mut start_index = 0;

        for (i, c) in s.chars().enumerate() {
            match current_state {
                VersionParserState::Start => match c {
                    '(' => current_state = VersionParserState::InequalityStart,
                    '[' => current_state = VersionParserState::EqualityStart,
                    _ => {}
                },
                VersionParserState::InequalityStart => match c {
                    ',' => {
                        current_state = VersionParserState::Lt;
                        start_index = i + 1;
                    }
                    ' ' => {
                        continue;
                    }
                    _ => {
                        current_state = VersionParserState::Gt;
                        start_index = i;
                    }
                },
                VersionParserState::EqualityStart => match c {
                    ' ' => {
                        continue;
                    }
                    _ => {
                        current_state = VersionParserState::Eq;
                        start_index = i;
                    }
                },
                VersionParserState::Eq => match c {
                    ',' => {
                        // peek on next char
                        let chars = s.chars().skip(i + 1);

                        for n in chars {
                            if n == ' ' {
                                continue;
                            }
                            versions.push(VersionRange::Ge(
                                s[start_index..i]
                                    .trim()
                                    .trim_end_matches(',')
                                    .trim_end()
                                    .to_string(),
                            ));
                            if n == ')' {
                                current_state = VersionParserState::Start;
                            } else {
                                current_state = VersionParserState::Lt;
                            }
                            start_index = i + 1;
                            break;
                        }
                    }
                    ')' => {
                        versions.push(VersionRange::Ge(
                            s[start_index..i - 1]
                                .trim()
                                .trim_end_matches(',')
                                .trim_end()
                                .to_string(),
                        ));
                        current_state = VersionParserState::Start;
                    }
                    // just eq =
                    ']' => {
                        versions.push(VersionRange::Eq(
                            s[start_index..i]
                                .trim()
                                .trim_end_matches(',')
                                .trim_end()
                                .to_string(),
                        ));
                        current_state = VersionParserState::Start;
                    }
                    _ => {}
                },
                VersionParserState::Lt => match c {
                    // just <
                    ')' => {
                        versions.push(VersionRange::Lt(
                            s[start_index..i]
                                .trim()
                                .trim_end_matches(',')
                                .trim_end()
                                .to_string(),
                        ));
                        current_state = VersionParserState::Start; // reset
                    }
                    // just <=
                    ']' => {
                        versions.push(VersionRange::Le(
                            s[start_index..i]
                                .trim()
                                .trim_end_matches(',')
                                .trim_end()
                                .to_string(),
                        ));
                        current_state = VersionParserState::Start; // reset
                    }
                    _ => {}
                },
                VersionParserState::Gt => match c {
                    ',' => {
                        // peak on next character, but the next character might be space
                        let chars = s.chars().skip(i + 1);
                        for n in chars {
                            if n == ' ' {
                                continue; // a whitespace
                            }
                            versions.push(VersionRange::Gt(
                                s[start_index..i]
                                    .trim()
                                    .trim_end_matches(',')
                                    .trim_end()
                                    .to_string(),
                            ));
                            if n == ')' {
                                current_state = VersionParserState::Start;
                            } else {
                                current_state = VersionParserState::Lt;
                            }
                            start_index = i + 1;
                            break;
                        }
                    }
                    // just >
                    ')' => {
                        versions.push(VersionRange::Gt(
                            s[start_index..i - 1]
                                .trim()
                                .trim_end_matches(',')
                                .trim_end()
                                .to_string(),
                        ));
                        current_state = VersionParserState::Start; // reset
                    }
                    // just >=
                    ']' => {
                        versions.push(VersionRange::Ge(
                            s[start_index..i]
                                .trim()
                                .trim_end_matches(',')
                                .trim_end()
                                .to_string(),
                        ));
                        current_state = VersionParserState::Start;
                    }
                    _ => {}
                },
            }
        }

        Ok(VersionRequirement::Hard(versions))
    }
}

type Properties = HashMap<String, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentPom {
    /// The organization name/package name
    pub group_id: String,
    /// The actual project name
    pub artifact_id: String,
    /// The project version number
    pub version: String,
    /// Relative path
    pub relative_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// The actual project name
    artifact_id: String,
    /// The project version number
    version: VersionRequirement,
    /// The selected version. This was what was resolved
    selected_version: Option<String>,
    /// The organization name/package name
    group_id: String,
    /// The project main dependencies
    dependencies: Vec<Project>,
    /// This project's dependencyManagement section
    dependency_management: HashMap<String, Project>,
    /// This module excludes
    excludes: Vec<Exclusion>,
    /// The scope of the project
    scope: Scope,
    /// The packaging of the project
    packaging: String,
    /// Properties of the project
    properties: Properties,
    /// Is Optional
    optional: bool,
    /// Parent pom
    pub parent: Option<ParentPom>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exclusion {
    /// The actual project name
    pub artifact_id: String,
    /// The organization name/package name
    pub group_id: String,
}
impl Exclusion {
    pub fn new(group_id: &str, artifact_id: &str) -> Self {
        Exclusion {
            artifact_id: artifact_id.to_string(),
            group_id: group_id.to_string(),
        }
    }
    pub fn qualified_name(&self) -> String {
        format!("{}:{}", self.group_id, self.artifact_id)
    }
}
impl Default for Project {
    fn default() -> Self {
        // FIXME remove these funny default and use ones provided by maven
        Project {
            artifact_id: "my_app".to_string(),
            version: VersionRequirement::Unset,
            selected_version: None,
            group_id: "com.my_organization.name".to_string(),
            dependencies: vec![],
            dependency_management: HashMap::new(),
            excludes: vec![],
            scope: Scope::COMPILE,
            packaging: String::from("jar"),
            properties: HashMap::new(),
            parent: None,
            optional: false,
        }
    }
}

impl Project {
    /// Initializes a new project with the provided arguments
    pub fn new(group_id: &str, artifact_id: &str, version: &str) -> Self {
        let version: VersionRequirement = version.parse().unwrap();
        // temporarily try to select a suitable version while waiting for version calculation
        let selected = match &version {
            VersionRequirement::Soft(v) => Some(v.clone()),
            VersionRequirement::Unset => None,
            VersionRequirement::Hard(hard) => {
                if hard.len() == 1 {
                    if let Some(VersionRange::Eq(v)) = hard.first() {
                        Some(v.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };

        Project {
            group_id: String::from(group_id),
            artifact_id: String::from(artifact_id),
            version,
            selected_version: selected,
            ..Default::default()
        }
    }
    /// Returns the artifact id of the project
    pub fn get_artifact_id(&self) -> String {
        self.artifact_id.clone()
    }
    /// Returns the version of the project
    pub fn get_version(&self) -> &VersionRequirement {
        &self.version
    }
    /// Sets the version requirement of the project
    pub fn set_version(&mut self, version: VersionRequirement) -> &mut Project {
        self.version = version;
        self
    }
    /// Returns the version of the project
    pub fn get_selected_version(&self) -> &Option<String> {
        &self.selected_version
    }
    /// Selects a version calculated from version requirements
    pub fn set_selected_version(&mut self, version: Option<String>) {
        self.selected_version = version;
    }
    /// Returns the group id of the project
    pub fn get_group_id(&self) -> String {
        self.group_id.clone()
    }
    /// Adds a dependency to this project
    pub fn add_dependency(&mut self, dep: Project) {
        self.dependencies.push(dep);
    }
    /// Adds a dependency to this project
    pub fn add_to_dependency_management(&mut self, dep: Project) {
        self.dependency_management
            .insert(format!("{}:{}", dep.group_id, dep.artifact_id), dep);
    }
    pub fn get_dependencies(&self) -> &Vec<Project> {
        &self.dependencies
    }
    pub fn get_dependency_management(&self) -> &HashMap<String, Project> {
        &self.dependency_management
    }
    pub fn get_dependencies_mut(&mut self) -> &mut Vec<Project> {
        &mut self.dependencies
    }
    pub fn get_dependencies_owned(self) -> Vec<Project> {
        self.dependencies
    }
    pub fn copy_parent(&mut self, parent: &Project) {
        if let Some(version) = &parent.selected_version {
            self.version = VersionRequirement::Soft(version.clone());
        }
        self.selected_version = parent.selected_version.clone();
        if !parent.excludes.is_empty() {
            self.excludes.extend(parent.excludes.iter().cloned());
        }
        self.scope = parent.scope.clone();
    }
    pub fn qualified_name(&self) -> anyhow::Result<String> {
        let version = self
            .selected_version
            .clone()
            .context("The package has no resolved version.")?;
        Ok(format!(
            "{}:{}:{}",
            self.group_id, self.artifact_id, version
        ))
    }
    pub fn get_excludes(&self) -> &Vec<Exclusion> {
        &self.excludes
    }
    pub fn add_exclusion(&mut self, exclude: Exclusion) {
        self.excludes.push(exclude);
    }
    pub fn get_scope(&self) -> Scope {
        self.scope.clone()
    }
    pub fn get_packaging(&self) -> String {
        self.packaging.clone()
    }
    pub fn set_packaging(&mut self, packaging: String) {
        self.packaging = packaging;
    }
    pub fn is_optional(&self) -> bool {
        self.optional
    }
    pub fn get_property(&self, key: &str) -> Option<String> {
        // if we fail to get it from the map it must be one of those java, env or project things
        let value = self.properties.get(key);
        if value.is_some() {
            return value.cloned();
        }

        let segments = key.split_once(".");
        segments?;

        let (group, item) = segments.unwrap();

        match group {
            "env" => {
                // do env stuff
                match std::env::var(item) {
                    Ok(v) => Some(v),
                    _ => None,
                }
            }
            "project" => {
                // reply with project stuff
                match item {
                    "version" => Some(self.version.to_string()),
                    "artifactId" => Some(self.get_artifact_id()),
                    "groupId" => Some(self.get_group_id()),
                    "scope" => Some(self.get_scope().to_string()),
                    "packaging" => Some(self.get_packaging()),
                    _ => None,
                }
            }
            // TODO
            // "java" => {
            //     // reply with java stuff
            // }
            _ => {
                // you are lost
                None
            }
        }
    }
    pub fn substitute_string(&self, data: &str) -> String {
        // Parse the string for ${}
        // Yet another state machine
        let mut result = String::with_capacity(data.len());
        #[derive(Debug)]
        enum SubState {
            Normal,
            PlaceholderDollar,
            PlaceholderBody,
        }

        let mut state = SubState::Normal;
        let mut current_placeholder_start = 0;

        for (i, c) in data.chars().enumerate() {
            match state {
                SubState::Normal if c == '$' => {
                    state = SubState::PlaceholderDollar;
                    continue;
                }
                SubState::Normal => {
                    result.push(c);
                }
                SubState::PlaceholderDollar if c == '{' => {
                    state = SubState::PlaceholderBody;
                    current_placeholder_start = i;
                    continue;
                }
                SubState::PlaceholderDollar if c.is_whitespace() => {
                    continue;
                }
                SubState::PlaceholderDollar => {
                    result.push(c);
                    state = SubState::Normal;
                    continue;
                }
                SubState::PlaceholderBody if c == '}' => {
                    // The end of our tag
                    // obtain the tag
                    let substring = &data[(current_placeholder_start + 1)..i].trim();
                    if let Some(property) = self.get_property(substring) {
                        result.push_str(&property);
                    }
                    state = SubState::Normal;
                }
                SubState::PlaceholderBody => {
                    continue;
                }
            }
        }
        result
    }
}

/// Parser states, helps in keeping track of the current event
/// and its corresponding start and end tags
#[derive(Clone, Debug)]
enum ParserState {
    /// Root of the pom xml file
    /// <project></project>
    Project,
    /// The project artifactId/name
    /// <artifactId><)artifactId>
    ReadArtifactId,
    /// The project groupId/package name
    /// <groupId></groupId>
    ReadGroupId,
    /// The project version number
    /// <version></version>
    ReadVersion,
    /// Indicates that the state machine is handling a dependency
    /// <dependencies></dependencies>
    Dependencies(DependencyState),
    /// Indicates that the state machine is handling a dependencyManagement Section
    /// <dependencyManagement></dependencyManagement>
    DependencyManagement(DependencyState),
    /// Indicates that the state machine is handling a dependency
    /// <parent></parent>
    Parent(ParentState),
    /// The packaging of this project
    /// <packaging></packaging>
    ReadPackaging,
    /// The properties of this project
    /// <properties></properties>
    Properties(PropertiesState),
    /// Used to indicate that under project we are in a tag we dont care about
    /// The argument is the level of xml tree we are at. 0 is at project level.
    /// Increment if we go deeper (Start tag) and decrement when we go up (End tag)
    Other(usize),
}

/// Keeps track of the dependency specific events
#[derive(Clone, Debug)]
enum DependencyState {
    /// Root of the dependency tree
    /// <dependencies></dependencies>
    Dependencies,
    /// A single dependency node
    /// <dependency></dependency>
    Dependency,
    /// The Dependency artifactId/name
    /// <artifactId><)artifactId>
    ReadArtifactId,
    /// The Dependency groupId/package name
    /// <groupId></groupId>
    ReadGroupId,
    /// The Dependency version number
    /// <version></version>
    ReadVersion,
    /// The dependency exclusions
    /// <exclusions></exclusions>
    Exclusions(ExclusionsState),
    /// The scope
    /// <scope></scope>
    ReadScope,
    /// If not optional
    /// <optional></optional>
    ReadOptional,
}
/// Keeps track of the parent specific events
#[derive(Clone, Debug)]
enum ParentState {
    /// Root of the parent tag
    /// <parent></parent>
    Parent,
    /// The Dependency artifactId/name
    /// <artifactId><)artifactId>
    ReadArtifactId,
    /// The Dependency groupId/package name
    /// <groupId></groupId>
    ReadGroupId,
    /// The Dependency version number
    /// <version></version>
    ReadVersion,
}

/// Keeps track of the properties specific events
#[derive(Clone, Debug)]
enum PropertiesState {
    /// Root properties tag
    ///<properties></properties>
    Properties,
    /// Tries to read a specific property.
    /// Since these are dynamic tag names we just use this generic enum name
    ReadEntry,
}

/// Keeps track of the exclusions specific events
#[derive(Clone, Debug)]
enum ExclusionsState {
    /// The dependency exclusions
    /// <exclusions></exclusions>
    Exclusions,
    /// The dependency exclusion
    /// <exclusion></exclusion>
    Exclusion(Exclusion),
    /// The Dependency artifactId/name
    /// <artifactId><)artifactId>
    ReadArtifactId(Exclusion),
    /// The Dependency groupId/package name
    /// <groupId></groupId>
    ReadGroupId(Exclusion),
}

struct Parser {
    state: ParserState,
    project: Project,
    /// Used to keep track of a dependency while parsing xml
    current_dependency: Option<Project>,
    /// Used to store the currently processed property tag
    current_property_tag: Vec<u8>,
    /// Used to carry the current property tag value
    current_property_value: Vec<String>,
}

impl Parser {
    /// Initializes a new project
    pub fn new(project: Project) -> Self {
        Parser {
            state: ParserState::Project,
            project,
            current_dependency: None,
            current_property_tag: Vec::new(),
            current_property_value: Vec::new(),
        }
    }
    /// Filters through xml stream events matching through accepted dependency tags
    /// triggered when <dependencies></dependencies> tag is encountered
    fn parse_deps(&mut self, event: Event, state: DependencyState) -> Result<DependencyState> {
        let new_state = match state {
            DependencyState::Dependencies => match event {
                // check for dependencies
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::DEPENDENCY => {
                        self.current_dependency = Some(Project::default());
                        DependencyState::Dependency
                    }
                    _ => DependencyState::Dependencies,
                },
                _ => DependencyState::Dependencies,
            },
            // <dependency> </dependency>
            DependencyState::Dependency => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::ARTIFACT_ID => DependencyState::ReadArtifactId,
                    tags::GROUP_ID => DependencyState::ReadGroupId,
                    tags::VERSION => DependencyState::ReadVersion,
                    tags::EXCLUSIONS => DependencyState::Exclusions(ExclusionsState::Exclusions),
                    tags::SCOPE => DependencyState::ReadScope,
                    tags::OPTIONAL => DependencyState::ReadOptional,
                    _ => DependencyState::Dependency,
                },
                Event::End(end) if end.local_name().into_inner() == tags::DEPENDENCY => {
                    // FIXME It doesn't feel correct that i had to clone this field
                    if let Some(dep) = self.current_dependency.clone() {
                        self.project.add_dependency(dep);
                        self.current_dependency = None;
                    }
                    DependencyState::Dependencies
                }
                _ => DependencyState::Dependency,
            },
            // <artifactId> </artifactId>
            DependencyState::ReadArtifactId => match event {
                Event::End(end) if end.local_name().into_inner() == tags::ARTIFACT_ID => {
                    DependencyState::Dependency
                }
                Event::Text(e) => {
                    if let Some(dep) = &mut self.current_dependency {
                        dep.artifact_id = e.unescape()?.to_string();
                    }
                    DependencyState::ReadArtifactId
                }
                _ => DependencyState::ReadArtifactId,
            },
            // <groupId></groupId>
            DependencyState::ReadGroupId => match event {
                Event::End(end) if end.local_name().into_inner() == tags::GROUP_ID => {
                    DependencyState::Dependency
                }

                Event::Text(e) => {
                    if let Some(dep) = &mut self.current_dependency {
                        dep.group_id = e.unescape()?.to_string();
                    }
                    DependencyState::ReadGroupId
                }
                _ => DependencyState::ReadGroupId,
            },
            // <version></version>
            DependencyState::ReadVersion => match event {
                Event::End(end) if end.local_name().into_inner() == tags::VERSION => {
                    DependencyState::Dependency
                }
                Event::Text(e) => {
                    if let Some(dep) = &mut self.current_dependency {
                        dep.selected_version = Some(e.unescape()?.to_string());
                    }
                    DependencyState::ReadVersion
                }
                _ => DependencyState::ReadVersion,
            },

            // <scope></scope>
            DependencyState::ReadScope => match event {
                Event::End(end) if end.local_name().into_inner() == tags::SCOPE => {
                    DependencyState::Dependency
                }
                Event::Text(e) => {
                    if let Some(dep) = &mut self.current_dependency {
                        let scope = e.unescape()?;
                        dep.scope = scope.parse::<Scope>()?;
                    }
                    DependencyState::ReadScope
                }
                _ => DependencyState::ReadScope,
            },

            // <exclusions></exclusions>
            DependencyState::Exclusions(exclu_state) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::EXCLUSIONS => {
                    DependencyState::Dependency
                }
                event => DependencyState::Exclusions(self.parse_exclusions(event, exclu_state)?),
            },

            // <optional></optional>
            DependencyState::ReadOptional => match event {
                Event::End(end) if end.local_name().into_inner() == tags::OPTIONAL => {
                    DependencyState::Dependency
                }
                Event::Text(e) => {
                    if let Some(dep) = &mut self.current_dependency {
                        let optional = e.unescape()?;
                        if optional.trim() == "true" {
                            dep.optional = true;
                        }
                    }
                    DependencyState::ReadOptional
                }
                _ => DependencyState::ReadOptional,
            },
        };
        Ok(new_state)
    }
    /// Filters through xml stream events matching through accepted dependency tags
    /// triggered when <dependencies></dependencies> tag is encountered
    fn parse_dependency_management(
        &mut self,
        event: Event,
        state: DependencyState,
    ) -> Result<DependencyState> {
        let new_state = match state {
            // <dependency> </dependency>
            DependencyState::Dependency => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::ARTIFACT_ID => DependencyState::ReadArtifactId,
                    tags::GROUP_ID => DependencyState::ReadGroupId,
                    tags::VERSION => DependencyState::ReadVersion,
                    tags::SCOPE => DependencyState::ReadScope,
                    tags::EXCLUSIONS => DependencyState::Exclusions(ExclusionsState::Exclusions),
                    _ => DependencyState::Dependency,
                },
                Event::End(end) if end.local_name().into_inner() == tags::DEPENDENCY => {
                    // FIXME also fix this: It doesn't feel correct that i had to clone this field
                    if let Some(dep) = self.current_dependency.clone() {
                        self.project.add_to_dependency_management(dep);
                        self.current_dependency = None;
                    }
                    DependencyState::Dependencies
                }
                _ => DependencyState::Dependency,
            },
            _ => self.parse_deps(event, state)?,
        };
        Ok(new_state)
    }

    fn parse_exclusions(
        &mut self,
        event: Event,
        state: ExclusionsState,
    ) -> Result<ExclusionsState> {
        let new_state = match state {
            // <exclusions></exclusions>
            ExclusionsState::Exclusions => match event {
                Event::Start(start) => match start.local_name().into_inner() {
                    tags::EXCLUSION => ExclusionsState::Exclusion(Exclusion::default()),
                    _ => ExclusionsState::Exclusions,
                },
                _ => ExclusionsState::Exclusions,
            },

            // <exclusion></exclusion>
            ExclusionsState::Exclusion(exclusion) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::EXCLUSION => {
                    if let Some(mut dependency) = self.current_dependency.clone() {
                        dependency.add_exclusion(exclusion);
                        self.current_dependency = Some(dependency);
                    }
                    ExclusionsState::Exclusions
                }
                Event::Start(start) => match start.local_name().into_inner() {
                    tags::ARTIFACT_ID => ExclusionsState::ReadArtifactId(exclusion),
                    tags::GROUP_ID => ExclusionsState::ReadGroupId(exclusion),
                    _ => ExclusionsState::Exclusion(exclusion),
                },
                _ => ExclusionsState::Exclusion(exclusion),
            },

            // <artifactId> </artifactId>
            ExclusionsState::ReadArtifactId(mut exclusion) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::ARTIFACT_ID => {
                    ExclusionsState::Exclusion(exclusion)
                }
                Event::Text(e) => {
                    let artifact_id = e.unescape()?.to_string();
                    exclusion.artifact_id = artifact_id;
                    ExclusionsState::ReadArtifactId(exclusion)
                }
                _ => ExclusionsState::ReadArtifactId(exclusion),
            },

            // <groupId></groupId>
            ExclusionsState::ReadGroupId(mut exclusion) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::GROUP_ID => {
                    ExclusionsState::Exclusion(exclusion)
                }
                Event::Text(e) => {
                    let group_id = e.unescape()?.to_string();
                    exclusion.group_id = group_id;
                    ExclusionsState::ReadGroupId(exclusion)
                }
                _ => ExclusionsState::ReadGroupId(exclusion),
            },
        };

        Ok(new_state)
    }
    fn parse_props(&mut self, event: Event, state: PropertiesState) -> Result<PropertiesState> {
        let new_state = match state {
            // <properties></properties>
            PropertiesState::Properties => match event {
                Event::End(end) => match end.local_name().into_inner() {
                    tags::PROPERTIES => PropertiesState::Properties,
                    tag => {
                        self.current_property_tag = Vec::from(tag);
                        PropertiesState::ReadEntry
                    }
                },
                Event::Start(tag) => {
                    self.current_property_tag = tag.local_name().into_inner().to_vec();
                    PropertiesState::ReadEntry
                }
                _ => PropertiesState::Properties,
            },
            // Handle the current tag
            PropertiesState::ReadEntry => match event {
                Event::End(end) if end.local_name().into_inner() == self.current_property_tag => {
                    let tag = String::from_utf8_lossy(&self.current_property_tag);

                    let value: String = self.current_property_value.join("");
                    self.project.properties.insert(tag.to_string(), value);
                    self.current_property_value.clear();

                    PropertiesState::Properties
                }
                Event::Text(e) => {
                    let value = e.unescape()?.to_string();
                    self.current_property_value.push(value);

                    PropertiesState::ReadEntry
                }
                _ => PropertiesState::ReadEntry,
            },
        };

        Ok(new_state)
    }
    fn parse_parent(&mut self, event: Event, state: ParentState) -> Result<ParentState> {
        let new_state = match state {
            // <parent></parent>
            ParentState::Parent => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::ARTIFACT_ID => ParentState::ReadArtifactId,
                    tags::GROUP_ID => ParentState::ReadGroupId,
                    tags::VERSION => ParentState::ReadVersion,
                    _ => ParentState::Parent,
                },
                Event::End(end) if end.local_name().into_inner() == tags::PARENT => {
                    ParentState::Parent
                }
                _ => ParentState::Parent,
            },
            // <artifactId> </artifactId>
            ParentState::ReadArtifactId => match event {
                Event::End(end) if end.local_name().into_inner() == tags::ARTIFACT_ID => {
                    ParentState::Parent
                }
                Event::Text(e) => {
                    if let Some(parent) = &mut self.project.parent {
                        parent.artifact_id = e.unescape()?.to_string();
                    }
                    ParentState::Parent
                }
                _ => ParentState::Parent,
            },
            // <groupId></groupId>
            ParentState::ReadGroupId => match event {
                Event::End(end) if end.local_name().into_inner() == tags::GROUP_ID => {
                    ParentState::Parent
                }

                Event::Text(e) => {
                    if let Some(parent) = &mut self.project.parent {
                        parent.group_id = e.unescape()?.to_string();
                    }
                    ParentState::ReadGroupId
                }
                _ => ParentState::ReadGroupId,
            },
            // <version></version>
            ParentState::ReadVersion => match event {
                Event::End(end) if end.local_name().into_inner() == tags::VERSION => {
                    ParentState::Parent
                }
                Event::Text(e) => {
                    if let Some(parent) = &mut self.project.parent {
                        parent.version = e.unescape()?.to_string();
                    }
                    ParentState::ReadVersion
                }
                _ => ParentState::ReadVersion,
            },
        };
        Ok(new_state)
    }

    /// Processes the xml stream events into its respective tags.
    /// The matched tags are used to update the state machine.
    pub fn process(&mut self, event: Event) -> Result<()> {
        self.state = match self.state.clone() {
            ParserState::Project => match event {
                // check for project level start tags
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::PROJECT => ParserState::Project,
                    tags::DEPENDENCIES => ParserState::Dependencies(DependencyState::Dependencies),
                    tags::DEPENDENCY_MANAGEMENT => {
                        ParserState::DependencyManagement(DependencyState::Dependencies)
                    }
                    tags::ARTIFACT_ID => ParserState::ReadArtifactId,
                    tags::GROUP_ID => ParserState::ReadGroupId,
                    tags::VERSION => ParserState::ReadVersion,
                    tags::PACKAGING => ParserState::ReadPackaging,
                    tags::PARENT => {
                        self.project.parent = Some(ParentPom {
                            group_id: String::new(),
                            artifact_id: String::new(),
                            version: String::new(),
                            relative_path: None,
                        });
                        ParserState::Parent(ParentState::Parent)
                    }
                    tags::PROPERTIES => ParserState::Properties(PropertiesState::Properties),
                    _ => ParserState::Other(1),
                },
                _ => ParserState::Project,
            },
            ParserState::Other(level) => match event {
                // We dont care about these tags. So ignore them
                Event::Start(_) => ParserState::Other(level + 1),
                Event::End(_) if level == 1 => ParserState::Project,
                Event::End(_) => ParserState::Other(level - 1),
                _ => ParserState::Other(level),
            },

            // <artifactId> </artifactId>
            ParserState::ReadArtifactId => match event {
                // exit the tag state
                Event::End(end) if end.local_name().into_inner() == tags::ARTIFACT_ID => {
                    ParserState::Project
                }
                Event::Text(e) => {
                    self.project.artifact_id = e.unescape()?.to_string();
                    ParserState::ReadArtifactId
                }
                _ => ParserState::ReadArtifactId,
            },

            // <groupId></groupId>
            ParserState::ReadGroupId => match event {
                Event::End(end) if end.local_name().into_inner() == tags::GROUP_ID => {
                    ParserState::Project
                }
                Event::Text(e) => {
                    self.project.group_id = e.unescape()?.to_string();
                    ParserState::ReadGroupId
                }
                _ => ParserState::ReadGroupId,
            },

            // <version></version>
            ParserState::ReadVersion => match event {
                Event::End(end) if end.local_name().into_inner() == tags::VERSION => {
                    ParserState::Project
                }
                Event::Text(e) => {
                    self.project.selected_version = Some(e.unescape()?.to_string());
                    ParserState::ReadVersion
                }
                _ => ParserState::ReadVersion,
            },
            ParserState::ReadPackaging => match event {
                Event::End(end) if end.local_name().into_inner() == tags::PACKAGING => {
                    ParserState::Project
                }
                Event::Text(e) => {
                    self.project.packaging = e.unescape()?.to_string();
                    ParserState::ReadPackaging
                }
                _ => ParserState::ReadPackaging,
            },

            // <dependencies></dependencies>
            ParserState::Dependencies(dep_state) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::DEPENDENCIES => {
                    ParserState::Project
                }
                event => ParserState::Dependencies(self.parse_deps(event, dep_state)?),
            },
            // <dependencyManagement></dependencyManagement>
            ParserState::DependencyManagement(dep_state) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::DEPENDENCY_MANAGEMENT => {
                    ParserState::Project
                }
                event => ParserState::DependencyManagement(
                    self.parse_dependency_management(event, dep_state)?,
                ),
            },
            // <properties></properties>
            ParserState::Properties(prop_state) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::PROPERTIES => {
                    ParserState::Project
                }
                event => ParserState::Properties(self.parse_props(event, prop_state)?),
            },
            // <parent></parent>
            ParserState::Parent(parent_state) => match event {
                Event::End(end) if end.local_name().into_inner() == tags::PARENT => {
                    ParserState::Project
                }
                event => ParserState::Parent(self.parse_parent(event, parent_state)?),
            },
        };
        Ok(())
    }
    // pub fn get_project(&self) -> &Project {
    //     return &self.project;
    // }
}
fn substitute_properties_vars(project: &mut Project) -> anyhow::Result<()> {
    // try to substitute properties.
    // some basic intelligence can be applied here since not all projects use variables

    if !project.properties.is_empty() {
        project.group_id = project.substitute_string(project.group_id.as_str());
        project.artifact_id = project.substitute_string(project.artifact_id.as_str());
        if let Some(version) = &project.selected_version {
            project.selected_version = Some(project.substitute_string(version.as_str()));
        }
    }
    // loop through dependencyManagement dependencies
    let mut management: HashMap<String, Project> = HashMap::new();
    for (id, dep) in project.dependency_management.iter() {
        let mut dep = dep.clone();
        let artifact_id = project.substitute_string(&dep.artifact_id.to_string());
        let group_id = project.substitute_string(&dep.group_id);
        let version: Option<String> = dep
            .selected_version
            .as_ref()
            .map(|v| project.substitute_string(v));
        dep.artifact_id = artifact_id;
        dep.group_id = group_id;
        if let Some(version) = version {
            dep.version = version
                .parse()
                .context("Failed to select a suitable version for dependency")?;
            dep.selected_version = Some(version);
        }
        management.insert(id.to_string(), dep);
    }
    project.dependency_management = management;

    // loop through all dependencies
    for (i, dep) in project.dependencies.clone().iter().enumerate() {
        project.dependencies[i].artifact_id = project.substitute_string(&dep.artifact_id);
        project.dependencies[i].group_id = project.substitute_string(&dep.group_id);
        if let Some(v) = &project.dependencies[i].selected_version {
            project.dependencies[i].version = project
                .substitute_string(v)
                .parse()
                .context("Failed to select a suitable version for dependency")?;
        }
    }

    Ok(())
}

/// Parses a pom xml file from a given stream and produces a Result
/// containing the Project object
pub fn parse_pom<R>(r: BufReader<R>, project: Project) -> anyhow::Result<Project>
where
    R: Read,
{
    let mut reader = Reader::from_reader(r);
    const BUFFER_SIZE: usize = 4096;
    let mut buf = Vec::with_capacity(BUFFER_SIZE);

    let mut parser = Parser::new(project);

    loop {
        match reader
            .read_event_into(&mut buf)
            .context("Reading xml events")?
        {
            Event::Eof => {
                break;
            }
            ev => parser.process(ev).context("Processing xml events")?,
        }
        buf.clear()
    }
    substitute_properties_vars(&mut parser.project)?;
    Ok(parser.project)
}

pub async fn parse_pom_async<R: AsyncRead + Unpin>(
    r: tokio::io::BufReader<R>,
    project: Project,
) -> anyhow::Result<Project> {
    let mut reader = Reader::from_reader(r);
    const BUFFER_SIZE: usize = 4096;
    let mut buf = Vec::with_capacity(BUFFER_SIZE);

    let mut parser = Parser::new(project);

    loop {
        match reader
            .read_event_into_async(&mut buf)
            .await
            .context("Reading xml events")?
        {
            Event::Eof => {
                break;
            }
            ev => parser.process(ev).context("Processing xml events")?,
        }
        buf.clear()
    }

    substitute_properties_vars(&mut parser.project)?;
    Ok(parser.project)
}
#[cfg(test)]
use pretty_assertions::assert_eq;

use crate::submodules::resolve::Constraint;

#[test]
fn project_substitute_property_string() {
    let mut project = Project::new("com.example", "artifact_1", "22.0.0");
    project
        .properties
        .insert("glorious".to_string(), "bustard".to_string());

    assert_eq!(
        project.substitute_string("${glorious}"),
        "bustard".to_string()
    );
    assert_eq!(
        project.substitute_string("${   glorious}"),
        "bustard".to_string()
    );
    assert_eq!(
        project.substitute_string("${   glorious   }"),
        "bustard".to_string()
    );
    assert_eq!(
        project.substitute_string("${glorious   }"),
        "bustard".to_string()
    );
    assert_eq!(
        project.substitute_string("$   {glorious}"),
        "bustard".to_string()
    );
    assert_eq!(
        project.substitute_string("$x{glorious}"),
        "x{glorious}".to_string()
    );
    assert_eq!(
        project.substitute_string("${project.groupId}"),
        "com.example".to_string()
    );
    assert_eq!(
        project.substitute_string("${project.artifactId}"),
        "artifact_1".to_string()
    );
    assert_eq!(
        project.substitute_string("${project.version}"),
        "22.0.0".to_string()
    );
    assert_eq!(
        project.substitute_string("${project.groupId}:${project.artifactId}:${project.version}"),
        "com.example:artifact_1:22.0.0".to_string()
    );
    assert_eq!(project.substitute_string("pro"), "pro".to_string());
}

#[test]
fn parse_pom_version_requirements() {
    let soft = "1.0";
    let lt = "(,1.0)";
    let ltlt = "(,1.0),(,2.0)";
    let le = "(,1.0]";
    let lele = "(,1.0],(,2.0]";
    let gt = "(1.0,)";
    let gtgt = "(1.0,),(2.0,)";
    let ge = "[1.0,)";
    let gege = "[1.0,),[2.0,)";
    let eq = "[1.0]";
    let eqeq = "[1.0],[2.0]";
    let incle = "[1.0,2.0]";
    let inc2 = "[1.0,2.0)";
    let inc3 = "(1.0,2.0]";
    let incl = "(1.0,2.0)";
    let or = "(,1.0],[1.2,)";
    let not = "(,1.1),(1.1,)";

    assert_eq!(
        "".parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Unset
    );
    assert_eq!(
        "  ".parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Unset
    );

    assert_eq!(
        soft.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Soft(String::from("1.0"))
    );

    assert_eq!(
        lt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Lt(String::from("1.0"))])
    );
    assert_eq!(
        ltlt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Lt(String::from("1.0")),
            VersionRange::Lt(String::from("2.0"))
        ])
    );
    assert_eq!(
        le.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Le(String::from("1.0"))])
    );
    assert_eq!(
        lele.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Le(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    assert_eq!(
        gt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Gt(String::from("1.0"))])
    );
    assert_eq!(
        gtgt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Gt(String::from("1.0")),
            VersionRange::Gt(String::from("2.0"))
        ])
    );
    assert_eq!(
        ge.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Ge(String::from("1.0"))])
    );
    assert_eq!(
        gege.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Ge(String::from("2.0"))
        ])
    );
    assert_eq!(
        eq.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Eq(String::from("1.0"))])
    );
    assert_eq!(
        eqeq.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Eq(String::from("1.0")),
            VersionRange::Eq(String::from("2.0"))
        ])
    );
    assert_eq!(
        incle.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    assert_eq!(
        incl.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Gt(String::from("1.0")),
            VersionRange::Lt(String::from("2.0"))
        ])
    );
    assert_eq!(
        inc2.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Lt(String::from("2.0"))
        ])
    );
    assert_eq!(
        inc3.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Gt(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    assert_eq!(
        or.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Le(String::from("1.0")),
            VersionRange::Ge(String::from("1.2"))
        ])
    );
    assert_eq!(
        not.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Lt(String::from("1.1")),
            VersionRange::Gt(String::from("1.1"))
        ])
    );

    // with spaces
    let inc2 = "[1.0,2.0)";
    let inc3 = "(1.0,2.0]";
    let incl = "(1.0,2.0)";
    let or = "( , 1.0] , [1.2 , )";
    let not = "( , 1.1) , (1.1 , )";

    // softies
    let soft = "   1.0";
    let soft2 = "1.0   ";
    let soft3 = "   1.0   ";

    assert_eq!(
        soft.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Soft(String::from("1.0"))
    );
    assert_eq!(
        soft2.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Soft(String::from("1.0"))
    );
    assert_eq!(
        soft3.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Soft(String::from("1.0"))
    );

    // lt
    let lt = "(, 1.0)";
    let lt2 = "(,1.0  )";
    let lt3 = "(  ,1.0  )";
    assert_eq!(
        lt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Lt(String::from("1.0"))])
    );
    assert_eq!(
        lt2.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Lt(String::from("1.0"))])
    );
    assert_eq!(
        lt3.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Lt(String::from("1.0"))])
    );

    // le
    let le = "(, 1.0]";
    let le2 = "(,1.0 ] ";
    let le3 = "(  ,1.0  ]";

    assert_eq!(
        le.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Le(String::from("1.0"))])
    );
    assert_eq!(
        le2.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Le(String::from("1.0"))])
    );
    assert_eq!(
        le3.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Le(String::from("1.0"))])
    );

    // gt
    let gt = "( 1.0,)";
    let gt2 = "(1.0,  )";
    let gt3 = "(  1.0 , )";

    assert_eq!(
        gt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Gt(String::from("1.0"))])
    );
    assert_eq!(
        gt2.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Gt(String::from("1.0"))])
    );
    assert_eq!(
        gt3.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Gt(String::from("1.0"))])
    );

    // ge
    let ge = "[  1.0,)  ";

    assert_eq!(
        ge.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Ge(String::from("1.0"))])
    );
    let ge = "[1.0,  )";
    assert_eq!(
        ge.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Ge(String::from("1.0"))])
    );
    let ge = "[  1.0,  )";
    assert_eq!(
        ge.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Ge(String::from("1.0"))])
    );

    // eq
    let eq = "[ 1.0]";
    assert_eq!(
        eq.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Eq(String::from("1.0"))])
    );
    let eq = "[1.0  ]";
    assert_eq!(
        eq.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Eq(String::from("1.0"))])
    );
    let eq = "[  1.0  ]";
    assert_eq!(
        eq.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Eq(String::from("1.0"))])
    );

    let incle = "[  1.0,2.0]";
    assert_eq!(
        incle.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    let incle = "[  1.0,  2.0]";
    assert_eq!(
        incle.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    let incle = "[  1.0,  2.0  ]";
    assert_eq!(
        incle.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    let incle = "[  1.0,2.0  ]";
    assert_eq!(
        incle.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    assert_eq!(
        incl.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Gt(String::from("1.0")),
            VersionRange::Lt(String::from("2.0"))
        ])
    );

    assert_eq!(
        inc2.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Ge(String::from("1.0")),
            VersionRange::Lt(String::from("2.0"))
        ])
    );
    assert_eq!(
        inc3.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Gt(String::from("1.0")),
            VersionRange::Le(String::from("2.0"))
        ])
    );
    assert_eq!(
        or.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Le(String::from("1.0")),
            VersionRange::Ge(String::from("1.2"))
        ])
    );
    assert_eq!(
        not.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![
            VersionRange::Lt(String::from("1.1")),
            VersionRange::Gt(String::from("1.1"))
        ])
    );
}

/// Tests conversion between version requirement and string. Remember we dont
/// care about the various syntatic sugars. What we care about is if the parser
/// can reproduce the same result regardless of what the to_string produces
#[test]
fn version_requirement_to_string() {
    assert_eq!(
        VersionRequirement::Soft("4.0".to_string()).to_string(),
        String::from("4.0")
    );

    assert_eq!(VersionRequirement::Unset.to_string(), String::new());

    // Now the complicated stuff.
    // The test plan:
    //  - Convert to string
    //  - Convert the resulting string back to VersionRequirement.
    //  - Compare if the resulting VersionRequirement is identical

    // Eq
    let version = VersionRequirement::Hard(vec![VersionRange::Eq("4.0".to_string())]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );

    // Gt
    let version = VersionRequirement::Hard(vec![VersionRange::Gt("4.0".to_string())]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );

    // Ge
    let version = VersionRequirement::Hard(vec![VersionRange::Ge("4.0".to_string())]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );

    // Lt
    let version = VersionRequirement::Hard(vec![VersionRange::Lt("4.0".to_string())]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );

    // Le
    let version = VersionRequirement::Hard(vec![VersionRange::Lt("4.0".to_string())]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );

    // more complex stuff
    // [1.0, 2.0]
    let version = VersionRequirement::Hard(vec![
        VersionRange::Ge(String::from("1.0")),
        VersionRange::Le(String::from("2.0")),
    ]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );
    // [1.0,2.0)
    let version = VersionRequirement::Hard(vec![
        VersionRange::Ge(String::from("1.0")),
        VersionRange::Lt(String::from("2.0")),
    ]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );
    // (1.0,2.0)
    let version = VersionRequirement::Hard(vec![
        VersionRange::Gt(String::from("1.0")),
        VersionRange::Lt(String::from("2.0")),
    ]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );
    // (,1.0],[1.2,)
    let version = VersionRequirement::Hard(vec![
        VersionRange::Le(String::from("1.0")),
        VersionRange::Ge(String::from("1.2")),
    ]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );
    // (,1.1),(1.1,)
    let version = VersionRequirement::Hard(vec![
        VersionRange::Lt(String::from("1.1")),
        VersionRange::Gt(String::from("1.1")),
    ]);
    assert_eq!(
        version.to_string().parse::<VersionRequirement>().unwrap(),
        version
    );
}

#[test]
fn version_range_from_string() {
    assert_eq!(
        ">20.0".parse::<VersionRange>().unwrap(),
        VersionRange::Gt("20.0".to_string())
    );
    assert_eq!(
        ">=20.0".parse::<VersionRange>().unwrap(),
        VersionRange::Ge("20.0".to_string())
    );
    assert_eq!(
        "<20.0".parse::<VersionRange>().unwrap(),
        VersionRange::Lt("20.0".to_string())
    );
    assert_eq!(
        "<=20.0".parse::<VersionRange>().unwrap(),
        VersionRange::Le("20.0".to_string())
    );
    assert_eq!(
        "=20.0".parse::<VersionRange>().unwrap(),
        VersionRange::Eq("20.0".to_string())
    );
    assert_eq!(
        "20.0".parse::<VersionRange>().unwrap(),
        VersionRange::Eq("20.0".to_string())
    );
    assert!(">".parse::<VersionRange>().is_err(),);
    assert!(">=".parse::<VersionRange>().is_err(),);
    assert!("<".parse::<VersionRange>().is_err(),);
    assert!("<=".parse::<VersionRange>().is_err(),);
    assert!("   ".parse::<VersionRange>().is_err(),);
}
#[test]
fn version_range_to_string() {
    assert_eq!(
        VersionRange::Eq("2.0".to_string()).to_string().as_str(),
        "2.0"
    );
    assert_eq!(
        VersionRange::Gt("2.0".to_string()).to_string().as_str(),
        ">2.0"
    );
    assert_eq!(
        VersionRange::Ge("2.0".to_string()).to_string().as_str(),
        ">=2.0"
    );
    assert_eq!(
        VersionRange::Lt("2.0".to_string()).to_string().as_str(),
        "<2.0"
    );
    assert_eq!(
        VersionRange::Le("2.0".to_string()).to_string().as_str(),
        "<=2.0"
    );
}
#[test]
fn version_requirement_from_constraint() {
    let c = Constraint::default();
    assert!(VersionRequirement::from(&c).is_unset());

    let c = Constraint {
        min: Some((true, "1.0".to_string())),
        max: Some((true, "3.0".to_string())),
        ..Default::default()
    };

    let vr = VersionRequirement::from(&c);
    assert!(vr.is_hard());
    assert_eq!(
        vr,
        VersionRequirement::Hard(vec![
            VersionRange::Ge("1.0".to_string()),
            VersionRange::Le("3.0".to_string())
        ])
    );
    let c = Constraint {
        exact: Some("2.0".to_string()),
        ..Default::default()
    };

    let vr = VersionRequirement::from(&c);
    assert!(vr.is_hard());
    assert_eq!(
        vr,
        VersionRequirement::Hard(vec![VersionRange::Eq("2.0".to_string())])
    );

    let c = Constraint {
        // exclude version 1
        exclusions: vec![(
            VersionRange::Ge("1.0".to_string()),
            VersionRange::Le("1.0".to_string()),
        )],
        ..Default::default()
    };

    let vr = VersionRequirement::from(&c);
    assert!(vr.is_hard());
    assert_eq!(
        vr,
        VersionRequirement::Hard(vec![
            VersionRange::Lt("1.0".to_string()),
            VersionRange::Gt("1.0".to_string()),
        ])
    );
}
// su
