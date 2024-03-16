use std::io::{BufReader, Read};

use anyhow::{Context, Result};
use quick_xml::{events::Event, Reader};

const METADATA: &[u8] = b"metadata";
const GROUP_ID: &[u8] = b"groupId";
const ARTIFACT_ID: &[u8] = b"artifactId";
const VERSIONING: &[u8] = b"versioning";
const LATEST: &[u8] = b"latest";
const RELEASE: &[u8] = b"release";
const VERSIONS: &[u8] = b"versions";
const VERSION: &[u8] = b"version";

/// The maven metadata xml object
/// check more at [maven repository metadata reference](https://maven.apache.org/ref/3.9.6/maven-repository-metadata/repository-metadata.html)
#[derive(Default, Debug, PartialEq)]
pub struct MavenMetadata {
    /// group id under
    ///<groupId></groupId>
    group_id: String,
    /// artifact id under
    ///<artifactId></artifactId>
    artifact_id: String,
    /// versions under
    /// <versions></versions>
    versions: Vec<String>,
    /// version number under metadata
    /// <version></version>
    version: Option<String>,
    /// The latest version
    /// <latest></latest>
    latest: Option<String>,
    /// The release version
    /// <release></release>
    release: Option<String>,
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
