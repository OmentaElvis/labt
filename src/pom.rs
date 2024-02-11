use anyhow::Context;
use anyhow::Result;
use quick_xml::{events::Event, Reader};
use std::io::BufReader;
use std::io::Read;
use tokio::io::AsyncRead;

//// constants for common tags
mod tags {
    pub const ARTIFACT_ID: &[u8] = b"artifactId";
    pub const GROUP_ID: &[u8] = b"groupId";
    pub const VERSION: &[u8] = b"version";
    pub const DEPENDENCIES: &[u8] = b"dependencies";
    pub const PROJECT: &[u8] = b"project";
    pub const DEPENDENCY: &[u8] = b"dependency";
    pub const EXCLUSIONS: &[u8] = b"exclusions";
    pub const EXCLUSION: &[u8] = b"exclusion";
    pub const SCOPE: &[u8] = b"scope";
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    COMPILE,
    TEST,
    RUNTIME,
    SYSTEM,
    PROVIDED,
    IMPORT,
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
            dependencies: vec![],
            excludes: vec![],
            scope: Scope::COMPILE,
        }
    }
    /// Returns the artifact id of the project
    pub fn get_artifact_id(&self) -> String {
        return self.artifact_id.clone();
    }
    /// Returns the version of the project
    pub fn get_version(&self) -> String {
        return self.version.clone();
    }
    /// Returns the group id of the project
    pub fn get_group_id(&self) -> String {
        return self.group_id.clone();
    }
    /// Adds a dependency to this project
    pub fn add_dependency(&mut self, dep: Project) {
        self.dependencies.push(dep);
    }
    pub fn get_dependencies(&self) -> &Vec<Project> {
        return &self.dependencies;
    }
    pub fn qualified_name(&self) -> String {
        format!("{}:{}:{}", self.group_id, self.artifact_id, self.version)
    }
    pub fn get_excludes(&self) -> &Vec<Exclusion> {
        return &self.excludes;
    }
    pub fn add_exclusion(&mut self, exclude: Exclusion) {
        self.excludes.push(exclude);
    }
    pub fn get_scope(&self) -> &Scope {
        return &self.scope;
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
                // check for dependecies
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
                    // FIXME It doesnt feel correct that i had to clone this field
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
                        // unnecessary clonning
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
        return Ok(new_state);
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
// su