use std::io::{BufReader, Read};

use anyhow::{bail, Context, Result};
use quick_xml::{events::Event, Reader};
use version_compare::Cmp;

use crate::pom::VersionRequirement;

const METADATA: &[u8] = b"metadata";
const GROUP_ID: &[u8] = b"groupId";
const ARTIFACT_ID: &[u8] = b"artifactId";
const VERSIONING: &[u8] = b"versioning";
const LATEST: &[u8] = b"latest";
const RELEASE: &[u8] = b"release";
const VERSIONS: &[u8] = b"versions";
const VERSION: &[u8] = b"version";
const NO_SELECTABLE_VERSION_ERROR: &str =
    "No appropriate version could be selected from maven-metadata.xml";

/// The maven metadata xml object
/// check more at [maven repository metadata reference](https://maven.apache.org/ref/3.9.6/maven-repository-metadata/repository-metadata.html)
#[derive(Default, Debug, PartialEq)]
pub struct MavenMetadata {
    /// group id under
    ///<groupId></groupId>
    pub group_id: String,
    /// artifact id under
    ///<artifactId></artifactId>
    pub artifact_id: String,
    /// versions under
    /// <versions></versions>
    pub versions: Vec<String>,
    /// version number under metadata
    /// <version></version>
    pub version: Option<String>,
    /// The latest version
    /// <latest></latest>
    pub latest: Option<String>,
    /// The release version
    /// <release></release>
    pub release: Option<String>,
}

impl MavenMetadata {
    pub fn new(group_id: String, artifact_id: String) -> Self {
        Self {
            group_id,
            artifact_id,
            versions: vec![],
            version: None,
            latest: None,
            release: None,
        }
    }
    /// chooses appropriate version based on constraints
    /// If a soft version is specified, we immediately return it
    /// If no target version is select, we return release version or latest version if release is not set.
    /// If a hard version is specified we match through all available versions and filter out unwanted versions
    /// If no appropriate version is found we return an error
    pub fn select_version(&self, constraints: &VersionRequirement) -> anyhow::Result<String> {
        match constraints {
            VersionRequirement::Soft(version) => Ok(version.clone()),
            VersionRequirement::Unset => {
                if let Some(release) = &self.release {
                    Ok(release.clone())
                } else if let Some(latest) = &self.latest {
                    return Ok(latest.clone());
                } else if self.versions.is_empty() {
                    bail!(NO_SELECTABLE_VERSION_ERROR);
                } else {
                    // try to select the latest version
                    let mut versions = self.versions.clone();
                    versions.sort_unstable_by(|a, b| match version_compare::compare(b, a) {
                        Ok(order) => match order {
                            Cmp::Eq | Cmp::Le | Cmp::Ge => std::cmp::Ordering::Equal,
                            Cmp::Lt => std::cmp::Ordering::Less,
                            Cmp::Gt => std::cmp::Ordering::Greater,
                            Cmp::Ne => {
                                // what ami supposed to do with this,
                                std::cmp::Ordering::Less
                            }
                        },
                        Err(_) => {
                            //TODO very unfortunate we have to panic
                            unreachable!();
                        }
                    });

                    return Ok(versions.first().unwrap().to_owned());
                }
            }
            VersionRequirement::Hard(hard_constraints) => {
                // filter through versions while short circuiting to reduce checks
                let mut versions: Vec<&String> = self
                    .versions
                    .iter()
                    .filter(|version| {
                        // for each version check that it matches all constraints. Reject it on first failure.
                        for c in hard_constraints {
                            // version compare does return a Result for invalid versions. for now ignore the error and filter out the package
                            match c {
                                crate::pom::VersionRange::Eq(target_version) => {
                                    if !version_compare::compare_to(
                                        version,
                                        target_version,
                                        Cmp::Eq,
                                    )
                                    .unwrap_or(false)
                                    {
                                        return false;
                                    }
                                }
                                crate::pom::VersionRange::Gt(target_version) => {
                                    if !version_compare::compare_to(
                                        version,
                                        target_version,
                                        Cmp::Gt,
                                    )
                                    .unwrap_or(false)
                                    {
                                        return false;
                                    }
                                }
                                crate::pom::VersionRange::Ge(target_version) => {
                                    if !version_compare::compare_to(
                                        version,
                                        target_version,
                                        Cmp::Ge,
                                    )
                                    .unwrap_or(false)
                                    {
                                        return false;
                                    }
                                }
                                crate::pom::VersionRange::Lt(target_version) => {
                                    if !version_compare::compare_to(
                                        version,
                                        target_version,
                                        Cmp::Lt,
                                    )
                                    .unwrap_or(false)
                                    {
                                        return false;
                                    }
                                }
                                crate::pom::VersionRange::Le(target_version) => {
                                    if !version_compare::compare_to(
                                        version,
                                        target_version,
                                        Cmp::Le,
                                    )
                                    .unwrap_or(false)
                                    {
                                        return false;
                                    }
                                }
                            }
                        }
                        true
                    })
                    .collect();

                if versions.is_empty() {
                    bail!("No appropriate version could be selected from maven-metadata.xml");
                }

                versions.sort_unstable_by(|a, b| match version_compare::compare(b, a) {
                    Ok(order) => match order {
                        Cmp::Eq | Cmp::Le | Cmp::Ge => std::cmp::Ordering::Equal,
                        Cmp::Lt => std::cmp::Ordering::Less,
                        Cmp::Gt => std::cmp::Ordering::Greater,
                        Cmp::Ne => {
                            // what ami supposed to do with this,
                            std::cmp::Ordering::Less
                        }
                    },
                    Err(_) => {
                        //TODO very unfortunate we have to panic
                        unreachable!();
                    }
                });

                // select the latest of the selected.
                let first = versions.first().unwrap().to_owned();

                Ok(first.clone())
            }
        }
    }
}

#[derive(Clone)]
enum ParserState {
    /// Top level tag, the metadata tag
    /// <metadata></metadata>
    Metadata,
    /// The group id
    /// <groupId></groupId>
    ReadGroupId,
    /// The artifact Id
    /// <artifactId></artifactId>
    ReadArtifactId,
    /// Read version under metadata
    /// <version></version>
    ReadVersion,
    /// Handles tags under versioning
    Versioning(VersioningState),
}

#[derive(Clone)]
enum VersioningState {
    /// Versioning information
    /// <versioning></versioning>
    Versioning,
    /// Read latest tag
    /// <latest></latest>
    ReadLatest,
    /// Read release tag
    /// <release></release>
    ReadRelease,
    /// Versions tag
    Versions(VersionsState),
}
#[derive(Clone)]
enum VersionsState {
    /// Versions lists
    /// <versions></versions>
    Versions,
    /// Read version under versioning
    /// <version></version>
    ReadVersion,
}

struct Parser {
    metadata: MavenMetadata,
    /// Tracks the parsing state of tge metadata
    state: ParserState,
    /// Tracks the current version read under versioning
    current_version: String,
}

impl Parser {
    pub fn new(metadata: MavenMetadata) -> Self {
        Self {
            metadata,
            state: ParserState::Metadata,
            current_version: String::new(),
        }
    }
    fn parse_versions(&mut self, event: Event, state: VersionsState) -> Result<VersionsState> {
        let state = match state {
            VersionsState::Versions => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    VERSION => VersionsState::ReadVersion,
                    _ => VersionsState::Versions,
                },
                _ => VersionsState::Versions,
            },
            // <version></version>
            VersionsState::ReadVersion => match event {
                Event::End(end) if end.local_name().into_inner() == VERSION => {
                    self.metadata.versions.push(self.current_version.clone());
                    VersionsState::Versions
                }
                Event::Text(text) => {
                    self.current_version = text.unescape()?.to_string();
                    VersionsState::ReadVersion
                }
                _ => VersionsState::ReadVersion,
            },
        };
        Ok(state)
    }
    fn parse_versioning(
        &mut self,
        event: Event,
        state: VersioningState,
    ) -> Result<VersioningState> {
        let state = match state {
            VersioningState::Versioning => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    LATEST => VersioningState::ReadLatest,
                    RELEASE => VersioningState::ReadRelease,
                    VERSIONS => VersioningState::Versions(VersionsState::Versions),
                    _ => VersioningState::Versioning,
                },
                _ => VersioningState::Versioning,
            },
            // <latest></latest>
            VersioningState::ReadLatest => match event {
                Event::End(end) if end.local_name().into_inner() == LATEST => {
                    VersioningState::Versioning
                }
                Event::Text(text) => {
                    self.metadata.latest = Some(text.unescape()?.to_string());
                    VersioningState::ReadLatest
                }
                _ => VersioningState::ReadLatest,
            },
            // <release></release>
            VersioningState::ReadRelease => match event {
                Event::End(end) if end.local_name().into_inner() == RELEASE => {
                    VersioningState::Versioning
                }
                Event::Text(text) => {
                    self.metadata.release = Some(text.unescape()?.to_string());
                    VersioningState::ReadRelease
                }
                _ => VersioningState::ReadRelease,
            },
            // <versions></versions>
            VersioningState::Versions(state) => match event {
                Event::End(end) if end.local_name().into_inner() == VERSIONS => {
                    VersioningState::Versioning
                }
                event => VersioningState::Versions(self.parse_versions(event, state)?),
            },
        };

        Ok(state)
    }
    pub fn process(&mut self, event: Event) -> Result<()> {
        self.state = match self.state.clone() {
            ParserState::Metadata => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    METADATA => ParserState::Metadata,
                    GROUP_ID => ParserState::ReadGroupId,
                    ARTIFACT_ID => ParserState::ReadArtifactId,
                    VERSION => ParserState::ReadVersion,
                    VERSIONING => ParserState::Versioning(VersioningState::Versioning),
                    _ => ParserState::Metadata,
                },
                _ => ParserState::Metadata,
            },
            // <groupId></groupId>
            ParserState::ReadGroupId => match event {
                Event::End(end) if end.local_name().into_inner() == GROUP_ID => {
                    ParserState::Metadata
                }
                Event::Text(text) => {
                    self.metadata.group_id = text.unescape()?.to_string();
                    ParserState::ReadGroupId
                }
                _ => ParserState::ReadGroupId,
            },
            // <artifactId></artifactId>
            ParserState::ReadArtifactId => match event {
                Event::End(end) if end.local_name().into_inner() == ARTIFACT_ID => {
                    ParserState::Metadata
                }
                Event::Text(text) => {
                    self.metadata.artifact_id = text.unescape()?.to_string();
                    ParserState::ReadArtifactId
                }
                _ => ParserState::ReadArtifactId,
            },
            // <versioning></versioning>
            ParserState::ReadVersion => match event {
                Event::End(end) if end.local_name().into_inner() == VERSION => {
                    ParserState::Metadata
                }
                Event::Text(text) => {
                    self.metadata.version = Some(text.unescape()?.to_string());
                    ParserState::ReadVersion
                }
                _ => ParserState::ReadVersion,
            },
            ParserState::Versioning(state) => match event {
                Event::End(end) if end.local_name().into_inner() == VERSIONING => {
                    ParserState::Metadata
                }
                event => ParserState::Versioning(self.parse_versioning(event, state)?),
            },
        };
        Ok(())
    }
}

pub fn parse_maven_metadata<R>(reader: BufReader<R>) -> anyhow::Result<MavenMetadata>
where
    R: Read,
{
    let mut reader = Reader::from_reader(reader);
    const BUFFER_SIZE: usize = 4096;
    let mut buf = Vec::with_capacity(BUFFER_SIZE);
    let metadata = MavenMetadata::default();

    let mut parser = Parser::new(metadata);

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

    Ok(parser.metadata)
}

#[test]
fn maven_metadata_parsing() {
    let file = r#"
<?xml version="1.0" encoding="UTF-8"?>
<metadata modelVersion="1.1.0">
  <groupId>com.gitlab.labt</groupId>
  <artifactId>labt</artifactId>
  <version>6.9.0</version>
  <versioning>
    <latest>6.9.0</latest>
    <release>6.9.0</release>
    <versions>
      <version>6.9.0</version>
      <version>6.8.4</version>
      <version>6.8.2</version>
      <version>6.8.0</version>
      <version>6.7.0</version>
      <version>6.6.0</version>
    </versions>
  </versioning>
</metadata>
"#
    .as_bytes();
    let reader = BufReader::new(file);
    let metadata = parse_maven_metadata(reader).unwrap();

    let expected = MavenMetadata {
        group_id: "com.gitlab.labt".to_string(),
        artifact_id: "labt".to_string(),
        version: Some("6.9.0".to_string()),
        latest: Some("6.9.0".to_string()),
        release: Some("6.9.0".to_string()),
        versions: vec![
            "6.9.0".to_string(),
            "6.8.4".to_string(),
            "6.8.2".to_string(),
            "6.8.0".to_string(),
            "6.7.0".to_string(),
            "6.6.0".to_string(),
        ],
    };

    assert_eq!(metadata, expected);
}

#[test]
fn maven_metadata_select_version() {
    let metadata = MavenMetadata {
        group_id: "com.gitlab.labt".to_string(),
        artifact_id: "labt".to_string(),
        version: Some("6.9.0".to_string()),
        latest: Some("6.9.1-SNAPSHOT".to_string()),
        release: Some("6.9.0".to_string()),
        versions: vec![
            "6.9.1-SNAPSHOT".to_string(),
            "6.9.0".to_string(),
            "6.8.4".to_string(),
            "6.8.2".to_string(),
            "6.8.0".to_string(),
            "6.7.0".to_string(),
            "6.6.0".to_string(),
            "5.9.0".to_string(),
            "5.8.4".to_string(),
            "5.8.2".to_string(),
            "5.8.0".to_string(),
            "5.7.0".to_string(),
            "5.6.0".to_string(),
            "4.9.0".to_string(),
            "4.8.4".to_string(),
            "4.8.2".to_string(),
            "4.8.0".to_string(),
            "4.7.0".to_string(),
            "4.6.0".to_string(),
        ],
    };

    assert_eq!(
        metadata
            .select_version(&"5.9".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        "5.9".to_string()
    );
    // eq
    assert_eq!(
        metadata
            .select_version(&"[5.9]".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        "5.9.0".to_string()
    );
    // Le
    assert_eq!(
        metadata
            .select_version(&"(,5.9]".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        "5.9.0".to_string()
    );
    // Lt
    assert_eq!(
        metadata
            .select_version(&"(,5.9)".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        "5.8.4".to_string()
    );
    // Ge
    assert_eq!(
        metadata
            .select_version(&"[5.9,)".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        "6.9.1-SNAPSHOT".to_string()
    );

    // Gt
    assert_eq!(
        metadata
            .select_version(&"(5.9,)".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        "6.9.1-SNAPSHOT".to_string()
    );
    // impossible
    assert!(metadata
        .select_version(&"(6.9.1,)".parse::<VersionRequirement>().unwrap())
        .is_err());

    assert_eq!(
        metadata.select_version(&VersionRequirement::Unset).unwrap(),
        "6.9.0".to_string()
    );
}
