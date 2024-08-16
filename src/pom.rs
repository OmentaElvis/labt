use anyhow::Context;
use anyhow::Result;
use quick_xml::{events::Event, Reader};
use serde::Serialize;
use std::io::BufReader;
use std::io::Read;
use std::str::FromStr;
use tokio::io::AsyncRead;

/// constants for common tags
mod tags {
    pub const ARTIFACT_ID: &[u8] = b"artifactId";
    pub const GROUP_ID: &[u8] = b"groupId";
    pub const VERSION: &[u8] = b"version";
    pub const DEPENDENCIES: &[u8] = b"dependencies";
    pub const PROJECT: &[u8] = b"project";
    pub const DEPENDENCY: &[u8] = b"dependency";
    pub const EXCLUSIONS: &[u8] = b"exclusions";
    pub const EXCLUSION: &[u8] = b"exclusion";
    pub const PACKAGING: &[u8] = b"packaging";
    pub const SCOPE: &[u8] = b"scope";
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
}

#[derive(Debug, PartialEq, Eq)]
pub enum VersionRange {
    Gt(String),
    Ge(String),
    Lt(String),
    Le(String),
    Eq(String),
}

#[derive(Default, Debug, PartialEq, Eq)]
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
        println!(">>> {}", s);

        for (i, c) in s.chars().enumerate() {
            println!("{i} {} {:?}", c, current_state);
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
                            if n != ')' {
                                versions.push(VersionRange::Ge(
                                    s[start_index..i]
                                        .trim()
                                        .trim_end_matches(',')
                                        .trim_end()
                                        .to_string(),
                                ));
                                current_state = VersionParserState::Lt;
                                start_index = i + 1;
                                break;
                            }
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
                            if n != ')' {
                                versions.push(VersionRange::Gt(
                                    s[start_index..i]
                                        .trim()
                                        .trim_end_matches(',')
                                        .trim_end()
                                        .to_string(),
                                ));
                                current_state = VersionParserState::Lt;
                                start_index = i + 1;
                                break;
                            }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// The actual project name
    artifact_id: String,
    /// The project version number
    version: String,
    /// The organization name/package name
    group_id: String,
    /// The project main dependencies
    dependencies: Vec<Project>,
    /// This module excludes
    excludes: Vec<Exclusion>,
    /// The scope of the project
    scope: Scope,
    /// The packaging of the project
    packaging: String,
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
            version: "1.0.0".to_string(),
            group_id: "com.my_organization.name".to_string(),
            dependencies: vec![],
            excludes: vec![],
            scope: Scope::COMPILE,
            packaging: String::from("jar"),
        }
    }
}

impl Project {
    /// Initializes a new project with the provided arguments
    pub fn new(group_id: &str, artifact_id: &str, version: &str) -> Self {
        Project {
            group_id: String::from(group_id),
            artifact_id: String::from(artifact_id),
            version: String::from(version),
            ..Default::default()
        }
    }
    /// Returns the artifact id of the project
    pub fn get_artifact_id(&self) -> String {
        self.artifact_id.clone()
    }
    /// Returns the version of the project
    pub fn get_version(&self) -> String {
        self.version.clone()
    }
    /// Returns the group id of the project
    pub fn get_group_id(&self) -> String {
        self.group_id.clone()
    }
    /// Adds a dependency to this project
    pub fn add_dependency(&mut self, dep: Project) {
        self.dependencies.push(dep);
    }
    pub fn get_dependencies(&self) -> &Vec<Project> {
        &self.dependencies
    }
    pub fn get_dependencies_mut(&mut self) -> &mut Vec<Project> {
        &mut self.dependencies
    }
    pub fn qualified_name(&self) -> String {
        format!("{}:{}:{}", self.group_id, self.artifact_id, self.version)
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
    /// The packaging of this project
    /// <packaging></packaging>
    ReadPackaging,
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
}

impl Parser {
    /// Initializes a new project
    pub fn new(project: Project) -> Self {
        Parser {
            state: ParserState::Project,
            project,
            current_dependency: None,
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
                        dep.version = e.unescape()?.to_string();
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
                        // FIXME fix this conversion from Cow<_, str> to str without
                        // unnecessary cloning
                        dep.scope = match scope.to_string().as_str() {
                            "compile" => Scope::COMPILE,
                            "test" => Scope::TEST,
                            "provided" => Scope::PROVIDED,
                            "import" => Scope::IMPORT,
                            "system" => Scope::SYSTEM,
                            "runtime" => Scope::RUNTIME,
                            _ => Scope::COMPILE,
                        }
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

    /// Processes the xml stream events into its respective tags.
    /// The matched tags are used to update the state machine.
    pub fn process(&mut self, event: Event) -> Result<()> {
        self.state = match self.state.clone() {
            ParserState::Project => match event {
                // check for project level start tags
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::PROJECT => ParserState::Project,
                    tags::DEPENDENCIES => ParserState::Dependencies(DependencyState::Dependencies),
                    tags::ARTIFACT_ID => ParserState::ReadArtifactId,
                    tags::GROUP_ID => ParserState::ReadGroupId,
                    tags::VERSION => ParserState::ReadVersion,
                    tags::PACKAGING => ParserState::ReadPackaging,
                    _ => ParserState::Project,
                },
                _ => ParserState::Project,
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
                    self.project.version = e.unescape()?.to_string();
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
        };
        Ok(())
    }
    // pub fn get_project(&self) -> &Project {
    //     return &self.project;
    // }
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

    Ok(parser.project)
}

#[test]
fn parse_pom_version_requirements() {
    let soft = "1.0";
    let lt = "(,1.0)";
    let le = "(,1.0]";
    let gt = "(1.0,)";
    let ge = "[1.0,)";
    let eq = "[1.0]";
    let incle = "[1.0,2.0]";
    let inc2 = "[1.0,2.0)";
    let inc3 = "(1.0,2.0]";
    let incl = "(1.0,2.0)";
    let or = "(,1.0],[1.2,)";
    let not = "(,1.1),(1.1,)";

    assert_eq!(
        soft.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Soft(String::from("1.0"))
    );

    assert_eq!(
        lt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Lt(String::from("1.0"))])
    );
    assert_eq!(
        le.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Le(String::from("1.0"))])
    );
    assert_eq!(
        gt.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Gt(String::from("1.0"))])
    );
    assert_eq!(
        ge.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Ge(String::from("1.0"))])
    );
    assert_eq!(
        eq.parse::<VersionRequirement>().unwrap(),
        VersionRequirement::Hard(vec![VersionRange::Eq(String::from("1.0"))])
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
// su
