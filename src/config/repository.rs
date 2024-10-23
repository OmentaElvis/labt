// Android repository.xml schema referenced from
// https://android.googlesource.com/platform/tools/base/+/refs/heads/main/sdklib/src/main/java/com/android/sdklib/repository/sdk-repository-11.xsd
use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    io::{BufReader, Read},
    num::ParseIntError,
    str::FromStr,
};

use anyhow::{bail, Context};
use quick_xml::{events::Event, Reader};

use crate::submodules::sdkmanager::ToId;

mod tags {
    // pub const SDK_REPOSITORY: &[u8] = b"sdk-repository";
    pub const CHANNEL: &[u8] = b"channel";
    pub const DISPLAY_NAME: &[u8] = b"display-name";
    pub const REMOTE_PACKAGE: &[u8] = b"remotePackage";
    pub const CHANNEL_REF: &[u8] = b"channelRef";
    pub const USES_LICENSE: &[u8] = b"uses-license";
    pub const ARCHIVES: &[u8] = b"archives";
    pub const ARCHIVE: &[u8] = b"archive";
    pub const SIZE: &[u8] = b"size";
    pub const CHECKSUM: &[u8] = b"checksum";
    pub const URL: &[u8] = b"url";
    pub const HOST_OS: &[u8] = b"host-os";
    pub const HOST_BITS: &[u8] = b"host-bits";
    pub const REVISION: &[u8] = b"revision";
    pub const MAJOR: &[u8] = b"major";
    pub const MINOR: &[u8] = b"minor";
    pub const MICRO: &[u8] = b"micro";
    pub const PREVIEW: &[u8] = b"preview";
    pub const LICENSE: &[u8] = b"license";
}

mod channel_strings {

    pub const STABLE: &str = "stable";
    pub const BETA: &str = "beta";
    pub const DEV: &str = "dev";
    pub const CANARY: &str = "canary";
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub enum ChannelType {
    Stable,
    Beta,
    Dev,
    Canary,
    Unknown(String),
    #[default]
    Unset,
}
impl FromStr for ChannelType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            channel_strings::STABLE => Ok(ChannelType::Stable),
            channel_strings::BETA => Ok(ChannelType::Beta),
            channel_strings::DEV => Ok(ChannelType::Dev),
            channel_strings::CANARY => Ok(ChannelType::Canary),
            "" => Ok(ChannelType::Unset),
            _ => Ok(ChannelType::Unknown(s.to_string())),
        }
    }
}
impl From<&str> for ChannelType {
    fn from(value: &str) -> Self {
        match value {
            channel_strings::STABLE => ChannelType::Stable,
            channel_strings::BETA => ChannelType::Beta,
            channel_strings::DEV => ChannelType::Dev,
            channel_strings::CANARY => ChannelType::Canary,
            "" => ChannelType::Unset,
            _ => ChannelType::Unknown(value.to_string()),
        }
    }
}
impl From<String> for ChannelType {
    fn from(value: String) -> Self {
        match value.as_str() {
            channel_strings::STABLE => ChannelType::Stable,
            channel_strings::BETA => ChannelType::Beta,
            channel_strings::DEV => ChannelType::Dev,
            channel_strings::CANARY => ChannelType::Canary,
            "" => ChannelType::Unset,
            _ => ChannelType::Unknown(value),
        }
    }
}
impl Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stable => write!(f, "{}", channel_strings::STABLE),
            Self::Beta => write!(f, "{}", channel_strings::BETA),
            Self::Dev => write!(f, "{}", channel_strings::DEV),
            Self::Canary => write!(f, "{}", channel_strings::CANARY),
            Self::Unset => write!(f, ""),
            Self::Unknown(unknown) => write!(f, "{}", unknown),
        }
    }
}

#[derive(Debug)]
enum OsType {
    Linux,
    Macosx,
    Windows,
}

impl From<&str> for OsType {
    fn from(value: &str) -> Self {
        match value {
            "linux" => Self::Linux,
            "macosx" => Self::Macosx,
            "windows" => Self::Windows,
            _ => Self::Linux,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitSizeType {
    Bit64,
    Bit32,
    #[default]
    Unset,
}

impl FromStr for BitSizeType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "32" | "32bit" | "32 bit" => Ok(Self::Bit32),
            "64" | "64bit" | "64 bit" => Ok(Self::Bit64),
            "" => Ok(Self::Unset),
            _ => Err(anyhow::anyhow!("Unknown bit width or unsupported: {}", s)),
        }
    }
}
impl Display for BitSizeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bit64 => write!(f, "64"),
            Self::Bit32 => write!(f, "32"),
            _ => write!(f, ""),
        }
    }
}

#[derive(Debug)]
pub struct RepositoryXml {
    remote_packages: Vec<RemotePackage>,
    /// List of licenses
    licenses: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemotePackage {
    path: String,
    /// An optional element indicating the package is obsolete.
    /// The string content is however currently not defined and ignored.
    obsolete: bool,
    name: String,
    channel: ChannelType,
    /// The optional license of this package. If present, users will have to agree to it before downloading.
    uses_license: String,
    /// A list of file archives for this package.
    archives: Vec<Archive>,
    revision: Revision,
}

impl ToId for RemotePackage {
    fn create_id(&self) -> (&String, &Revision, &ChannelType) {
        (&self.path, &self.revision, &self.channel)
    }
}

#[derive(Debug, Clone, Default, Hash, PartialEq, Eq)]
pub struct Archive {
    size: usize,
    checksum: String,
    url: String,
    host_os: String,
    host_bits: BitSizeType,
}

impl RemotePackage {
    pub fn new() -> Self {
        Self {
            path: String::new(),
            obsolete: false,
            name: String::new(),
            uses_license: String::new(),
            archives: Vec::new(),
            revision: Revision::default(),
            channel: ChannelType::Unset,
        }
    }
    // The following methods are self explanatory
    pub fn set_path(&mut self, path: String) {
        self.path = path;
    }
    pub fn set_obsolete(&mut self, obsolete: bool) {
        self.obsolete = obsolete;
    }
    pub fn set_display_name(&mut self, name: String) {
        self.name = name;
    }
    pub fn set_channel(&mut self, channel: ChannelType) {
        self.channel = channel;
    }
    pub fn set_license(&mut self, license_ref: String) {
        self.uses_license = license_ref;
    }
    pub fn set_revision(&mut self, revision: Revision) {
        self.revision = revision;
    }

    pub fn get_path(&self) -> &String {
        &self.path
    }
    pub fn is_obsolete(&self) -> bool {
        self.obsolete
    }
    pub fn get_display_name(&self) -> &String {
        &self.name
    }
    pub fn get_channel(&self) -> &ChannelType {
        &self.channel
    }
    pub fn get_revision(&self) -> &Revision {
        &self.revision
    }
    pub fn get_archives(&self) -> &Vec<Archive> {
        &self.archives
    }
    pub fn get_uses_license(&self) -> &String {
        &self.uses_license
    }
    /// Adds am archive entry
    pub fn add_archive(&mut self, archive: Archive) {
        self.archives.push(archive);
    }
}

impl Default for RemotePackage {
    fn default() -> Self {
        Self::new()
    }
}

impl Archive {
    pub fn get_size(&self) -> usize {
        self.size
    }
    pub fn get_checksum(&self) -> &String {
        &self.checksum
    }
    pub fn get_url(&self) -> &String {
        &self.url
    }
    pub fn get_host_os(&self) -> &String {
        &self.host_os
    }
    pub fn get_host_bits(&self) -> BitSizeType {
        self.host_bits
    }

    pub fn set_size(&mut self, value: usize) {
        self.size = value;
    }
    pub fn set_checksum(&mut self, value: String) {
        self.checksum = value;
    }
    pub fn set_url(&mut self, value: String) {
        self.url = value;
    }
    pub fn set_host_os(&mut self, value: String) {
        self.host_os = value;
    }
    pub fn set_host_bits(&mut self, value: BitSizeType) {
        self.host_bits = value;
    }
}

impl RepositoryXml {
    pub fn new() -> Self {
        Self {
            remote_packages: Vec::new(),
            licenses: HashMap::new(),
        }
    }
    pub fn add_remote_package(&mut self, package: RemotePackage) {
        self.remote_packages.push(package);
    }
    pub fn get_remote_packages(&self) -> &Vec<RemotePackage> {
        &self.remote_packages
    }
    pub fn add_license(&mut self, id: String, license: String) {
        self.licenses.insert(id, license);
    }
    pub fn get_licenses(&self) -> &HashMap<String, String> {
        &self.licenses
    }
}

impl Default for RepositoryXml {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
enum ParserState {
    /// <sdk:sdk-repository>
    SdkRepository,
    /// <channel id=""></channel>
    Channel,
    /// <remotePackage>
    RemotePackage(RemotePackageState),
    /// <license></license>
    License,
}
#[derive(Clone, Copy)]
enum RemotePackageState {
    RemotePackage,
    ReadDisplayName,
    Archives(ArchiveState),
    Revision(RevisionState),
}

#[derive(Clone, Copy)]
enum ArchiveState {
    Archives,
    Archive,
    ReadSize,
    ReadChecksum,
    ReadUrl,
    ReadHostOs,
    ReadHostBits,
}
#[derive(Debug, Clone, Copy)]
enum RevisionState {
    Revision,
    ReadMajor,
    ReadMinor,
    ReadMicro,
    ReadPreview,
}
/// A full revision, with a major.minor.micro and an
/// optional preview number. The major number is mandatory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Revision {
    pub major: u32,
    pub minor: u32,
    pub micro: u32,
    pub preview: u32,
}

impl Revision {
    pub fn new(major: u32) -> Self {
        Self {
            major,
            ..Default::default()
        }
    }
}

impl Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.major, self.minor, self.micro, self.preview
        )
    }
}
impl PartialOrd for Revision {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // major
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Greater => return Some(std::cmp::Ordering::Greater),
            std::cmp::Ordering::Less => return Some(std::cmp::Ordering::Less),
            _ => {}
        }

        // minor
        match self.minor.cmp(&other.minor) {
            std::cmp::Ordering::Greater => return Some(std::cmp::Ordering::Greater),
            std::cmp::Ordering::Less => return Some(std::cmp::Ordering::Less),
            _ => {}
        }

        // micro
        match self.micro.cmp(&other.micro) {
            std::cmp::Ordering::Greater => return Some(std::cmp::Ordering::Greater),
            std::cmp::Ordering::Less => return Some(std::cmp::Ordering::Less),
            _ => {}
        }

        // preview
        match self.preview.cmp(&other.preview) {
            std::cmp::Ordering::Greater => return Some(std::cmp::Ordering::Greater),
            std::cmp::Ordering::Less => return Some(std::cmp::Ordering::Less),
            _ => {}
        }

        // must be equal
        Some(std::cmp::Ordering::Equal)
    }
}
#[derive(Debug)]
pub struct RevisionParseError {
    pub kind: RevisionParseErrorKind,
}

impl RevisionParseError {
    pub fn new(kind: RevisionParseErrorKind) -> Self {
        Self { kind }
    }
}

impl Display for RevisionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            RevisionParseErrorKind::Empty => {
                write!(f, "The provided string is empty")
            }
            RevisionParseErrorKind::MissingMajor => {
                write!(f, "Missing major version int which is required")
            }
            RevisionParseErrorKind::InvalidUnsignedInt(err) => {
                write!(f, "failed to parse unsigned int: {}", err)
            }
        }
    }
}

impl Error for RevisionParseError {}

#[derive(Debug)]
pub enum RevisionParseErrorKind {
    /// An empty string was provided
    Empty,
    /// Major version was not provided
    MissingMajor,
    /// Int parse error
    InvalidUnsignedInt(ParseIntError),
}

impl FromStr for Revision {
    type Err = RevisionParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut iter = value.splitn(4, '.');
        let major: u32 = if let Some(major) = iter.next() {
            major.trim().parse().map_err(|err| {
                RevisionParseError::new(RevisionParseErrorKind::InvalidUnsignedInt(err))
            })?
        } else {
            return Err(RevisionParseError::new(
                RevisionParseErrorKind::MissingMajor,
            ));
        };

        let mut revision = Revision::new(major);
        if let Some(minor) = iter.next() {
            revision.minor = minor.parse().map_err(|err| {
                RevisionParseError::new(RevisionParseErrorKind::InvalidUnsignedInt(err))
            })?;
        } else {
            return Ok(revision);
        }
        if let Some(micro) = iter.next() {
            revision.micro = micro.parse().map_err(|err| {
                RevisionParseError::new(RevisionParseErrorKind::InvalidUnsignedInt(err))
            })?;
        } else {
            return Ok(revision);
        }
        if let Some(preview) = iter.next() {
            revision.preview = preview.parse().map_err(|err| {
                RevisionParseError::new(RevisionParseErrorKind::InvalidUnsignedInt(err))
            })?;
        } else {
            return Ok(revision);
        }

        Ok(revision)
    }
}

/// Parses android repository xml for sdk manager
pub struct RepositoryXmlParser {
    repo: RepositoryXml,
    state: ParserState,

    /// Holds the current channel id attribute
    current_channel_id: Option<String>,
    current_channel_type: Option<ChannelType>,

    /// The current remotePackage type being handled
    current_package: RemotePackage,

    /// The current archive being read
    current_archive: Archive,

    /// The current revision we are working with
    current_revision: Revision,

    current_license_id: Option<String>,
    /// A reference table for channels
    channels: HashMap<String, ChannelType>,
}

impl RepositoryXmlParser {
    pub fn new() -> Self {
        Self {
            repo: RepositoryXml::default(),
            state: ParserState::SdkRepository,
            current_channel_id: None,
            current_channel_type: None,
            current_license_id: None,
            current_package: RemotePackage::default(),
            current_revision: Revision::default(),
            current_archive: Archive::default(),
            channels: HashMap::new(),
            // for some unknown reason, this Default thingy causes SIGSEGV
            // ..Default::default()
        }
    }
    fn parse_revision(
        &mut self,
        state: RevisionState,
        event: Event,
    ) -> anyhow::Result<RevisionState> {
        let new_state = match state {
            // <revision></revision>
            RevisionState::Revision => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::MAJOR => RevisionState::ReadMajor,
                    tags::MINOR => RevisionState::ReadMinor,
                    tags::MICRO => RevisionState::ReadMicro,
                    tags::PREVIEW => RevisionState::ReadPreview,
                    _ => RevisionState::Revision,
                },
                _ => RevisionState::Revision,
            },
            // <major></major>
            RevisionState::ReadMajor => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::MAJOR => {
                    RevisionState::Revision
                }
                Event::Text(text) => {
                    let number = text
                        .unescape()?
                        .to_string()
                        .parse::<u32>()
                        .context("Failed to parse revision major to int")?;
                    self.current_revision.major = number;
                    RevisionState::ReadMajor
                }
                _ => RevisionState::ReadMajor,
            },
            // <minor></minor>
            RevisionState::ReadMinor => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::MINOR => {
                    RevisionState::Revision
                }
                Event::Text(text) => {
                    let number = text
                        .unescape()?
                        .to_string()
                        .parse::<u32>()
                        .context("Failed to parse revision minor to int")?;
                    self.current_revision.minor = number;
                    RevisionState::ReadMinor
                }
                _ => RevisionState::ReadMinor,
            },
            // <micro></micro>
            RevisionState::ReadMicro => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::MICRO => {
                    RevisionState::Revision
                }
                Event::Text(text) => {
                    let number = text
                        .unescape()?
                        .to_string()
                        .parse::<u32>()
                        .context("Failed to parse revision micro to int")?;
                    self.current_revision.micro = number;
                    RevisionState::ReadMicro
                }
                _ => RevisionState::ReadMicro,
            },
            // <preview></preview>
            RevisionState::ReadPreview => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::PREVIEW => {
                    RevisionState::Revision
                }
                Event::Text(text) => {
                    let number = text
                        .unescape()?
                        .to_string()
                        .parse::<u32>()
                        .context("Failed to parse revision preview to int")?;
                    self.current_revision.preview = number;
                    RevisionState::ReadPreview
                }
                _ => RevisionState::ReadPreview,
            },
        };
        Ok(new_state)
    }
    fn parse_archive(&mut self, state: ArchiveState, event: Event) -> anyhow::Result<ArchiveState> {
        let new_state = match state {
            // <archives></archives>
            ArchiveState::Archives => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::ARCHIVE => {
                        // start a new archive entry
                        self.current_archive = Archive::default();
                        ArchiveState::Archive
                    }
                    _ => ArchiveState::Archives,
                },
                _ => ArchiveState::Archives,
            },

            // <archive></archive>
            ArchiveState::Archive => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::SIZE => ArchiveState::ReadSize,
                    tags::CHECKSUM => ArchiveState::ReadChecksum,
                    tags::URL => ArchiveState::ReadUrl,
                    tags::HOST_OS => ArchiveState::ReadHostOs,
                    tags::HOST_BITS => ArchiveState::ReadHostBits,
                    _ => ArchiveState::Archive,
                },
                Event::End(tag) if tag.local_name().into_inner() == tags::ARCHIVE => {
                    self.current_package
                        .add_archive(self.current_archive.clone());
                    ArchiveState::Archives
                }
                _ => ArchiveState::Archive,
            },

            // <size></size>
            ArchiveState::ReadSize => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::SIZE => {
                    ArchiveState::Archive
                }
                Event::Text(text) => {
                    let size = text
                        .unescape()?
                        .to_string()
                        .parse::<usize>()
                        .context("Failed to parse size string value to usize")?;
                    self.current_archive.set_size(size);
                    ArchiveState::ReadSize
                }
                _ => ArchiveState::ReadSize,
            },

            // <checksum></checksum>
            ArchiveState::ReadChecksum => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::CHECKSUM => {
                    ArchiveState::Archive
                }
                Event::Text(text) => {
                    let checksum = text.unescape()?.to_string();
                    self.current_archive.set_checksum(checksum);
                    ArchiveState::ReadChecksum
                }
                _ => ArchiveState::ReadChecksum,
            },

            // <url></url>
            ArchiveState::ReadUrl => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::URL => {
                    ArchiveState::Archive
                }
                Event::Text(text) => {
                    let url = text.unescape()?.to_string();
                    self.current_archive.set_url(url);
                    ArchiveState::ReadUrl
                }
                _ => ArchiveState::ReadUrl,
            },

            // <host-os></host-os>
            ArchiveState::ReadHostOs => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::HOST_OS => {
                    ArchiveState::Archive
                }
                Event::Text(text) => {
                    let host = text.unescape()?.to_string();
                    self.current_archive.set_host_os(host);
                    ArchiveState::ReadHostOs
                }
                _ => ArchiveState::ReadHostOs,
            },

            // <host-bits></host-bits>
            ArchiveState::ReadHostBits => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::HOST_BITS => {
                    ArchiveState::Archive
                }
                Event::Text(text) => {
                    let bits = text
                        .unescape()?
                        .to_string()
                        .parse::<u8>()
                        .context("Failed to parse host-bits value to u8")?;
                    match bits {
                        32 => self.current_archive.set_host_bits(BitSizeType::Bit32),
                        64 => self.current_archive.set_host_bits(BitSizeType::Bit64),
                        _ => self.current_archive.set_host_bits(BitSizeType::Unset),
                    }
                    ArchiveState::ReadHostBits
                }
                _ => ArchiveState::ReadHostBits,
            },
        };
        Ok(new_state)
    }
    fn process_remote_package(
        &mut self,
        state: RemotePackageState,
        event: Event,
    ) -> anyhow::Result<ParserState> {
        let new_state: RemotePackageState = match state {
            // <remotePackage></remotePackage>
            RemotePackageState::RemotePackage => match event {
                Event::Start(tag) => match tag.local_name().into_inner() {
                    tags::DISPLAY_NAME => RemotePackageState::ReadDisplayName,
                    tags::ARCHIVES => RemotePackageState::Archives(ArchiveState::Archives),
                    tags::REVISION => {
                        self.current_revision = Revision::default();
                        RemotePackageState::Revision(RevisionState::Revision)
                    }
                    _ => RemotePackageState::RemotePackage,
                },
                Event::Empty(tag) => match tag.local_name().into_inner() {
                    tags::CHANNEL_REF => {
                        if let Some(attr) = tag.try_get_attribute("ref")? {
                            self.current_package.channel = ChannelType::Unknown(
                                String::from_utf8_lossy(&attr.value).to_string(),
                            );
                            RemotePackageState::RemotePackage
                        } else {
                            bail!("Missing ref attribute for <channelRef/>");
                        }
                    }
                    tags::USES_LICENSE => {
                        if let Some(attr) = tag.try_get_attribute("ref")? {
                            self.current_package
                                .set_license(String::from_utf8_lossy(&attr.value).to_string());
                            RemotePackageState::RemotePackage
                        } else {
                            bail!("Missing ref attribute for <uses-license)>");
                        }
                    }
                    _ => RemotePackageState::RemotePackage,
                },
                _ => RemotePackageState::RemotePackage,
            },

            // <display-name></display-name>
            RemotePackageState::ReadDisplayName => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::DISPLAY_NAME => {
                    RemotePackageState::RemotePackage
                }
                Event::Text(text) => {
                    self.current_package
                        .set_display_name(text.unescape()?.to_string());
                    RemotePackageState::ReadDisplayName
                }
                _ => RemotePackageState::ReadDisplayName,
            },

            // <archives></archives>
            RemotePackageState::Archives(state) => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::ARCHIVES => {
                    RemotePackageState::RemotePackage
                }
                _ => RemotePackageState::Archives(self.parse_archive(state, event)?),
            },
            // <revision></revision>
            RemotePackageState::Revision(state) => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::REVISION => {
                    self.current_package.revision = self.current_revision.clone();
                    // self.current_revision = Revision::default();
                    RemotePackageState::RemotePackage
                }
                _ => RemotePackageState::Revision(self.parse_revision(state, event)?),
            },
        };

        Ok(ParserState::RemotePackage(new_state))
    }
    pub fn process(&mut self, event: Event) -> anyhow::Result<()> {
        self.state = match self.state {
            ParserState::SdkRepository => {
                // root tag
                match event {
                    Event::Start(tag) => match tag.local_name().into_inner() {
                        tags::CHANNEL => {
                            if let Some(attr) = tag.try_get_attribute("id")? {
                                self.current_channel_id =
                                    Some(String::from_utf8_lossy(&attr.value).to_string());
                            } else {
                                self.current_channel_id = None;
                            }
                            ParserState::Channel
                        }
                        // <remotePackage path="" />
                        tags::REMOTE_PACKAGE => {
                            self.current_package = RemotePackage::default();
                            // get the attributes: param
                            if let Some(attr) = tag.try_get_attribute("path")? {
                                self.current_package
                                    .set_path(String::from_utf8_lossy(&attr.value).to_string());
                            } else {
                                bail!("Missing path parameter for remotePackage");
                            }

                            // get the obsolete attribute
                            if let Some(attr) = tag.try_get_attribute("obsolete")? {
                                if String::from_utf8_lossy(&attr.value) == *"true" {
                                    self.current_package.set_obsolete(true);
                                }
                            }

                            ParserState::RemotePackage(RemotePackageState::RemotePackage)
                        }
                        // <license id=""></license>
                        tags::LICENSE => {
                            if let Some(attr) = tag.try_get_attribute("id")? {
                                self.current_license_id =
                                    Some(String::from_utf8_lossy(&attr.value).to_string());
                            } else {
                                bail!("Missing id parameter for license attribute");
                            }
                            ParserState::License
                        }
                        _ => ParserState::SdkRepository,
                    },
                    _ => ParserState::SdkRepository,
                }
            }
            ParserState::Channel => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::CHANNEL => {
                    if self.current_channel_id.is_none() {
                        bail!("channel id attribute missing");
                    }

                    if self.current_channel_type.is_none() {
                        unreachable!();
                    }
                    // TODO convert this to be a no clone/copy twice
                    self.channels.insert(
                        self.current_channel_id.clone().unwrap(),
                        self.current_channel_type.clone().unwrap(),
                    );

                    self.current_channel_id = None;
                    self.current_channel_type = None;
                    ParserState::SdkRepository
                }
                Event::Text(text) => {
                    self.current_channel_type = Some(text.unescape()?.to_string().into());
                    ParserState::Channel
                }
                _ => ParserState::Channel,
            },
            ParserState::RemotePackage(state) => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::REMOTE_PACKAGE => {
                    self.repo.add_remote_package(self.current_package.clone());
                    self.current_package = RemotePackage::default();
                    ParserState::SdkRepository
                }
                _ => self.process_remote_package(state, event)?,
            },
            ParserState::License => match event {
                Event::End(tag) if tag.local_name().into_inner() == tags::LICENSE => {
                    self.current_license_id = None;
                    ParserState::SdkRepository
                }
                Event::Text(text) => {
                    if let Some(id) = &self.current_license_id {
                        self.repo
                            .add_license(id.to_string(), text.unescape()?.to_string());
                    }
                    ParserState::License
                }
                _ => ParserState::License,
            },
        };
        Ok(())
    }
    /// Returns the complete repository with all channels resolved to their value
    pub fn get_repository(mut self) -> RepositoryXml {
        for package in self.repo.remote_packages.iter_mut() {
            if let ChannelType::Unknown(id) = &package.channel {
                if let Some(channel) = self.channels.get(id) {
                    package.set_channel(channel.clone());
                }
            }
        }
        self.repo
    }
}

impl Default for RepositoryXmlParser {
    fn default() -> Self {
        Self::new()
    }
}
/// Parses a repository.xml file from a given stream and produces a Result
/// containing the Repo object
pub fn parse_repository_xml<R>(r: BufReader<R>) -> anyhow::Result<RepositoryXml>
where
    R: Read,
{
    let mut reader = Reader::from_reader(r);
    const BUFFER_SIZE: usize = 4096;
    let mut buf = Vec::with_capacity(BUFFER_SIZE);

    let mut parser = RepositoryXmlParser::default();

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

    let repo = parser.get_repository();

    Ok(repo)
}

#[test]
fn test_revision_from_string() {
    let version = "1.0.0.0";
    let revision: Revision = version.parse().unwrap();

    assert_eq!(
        revision,
        Revision {
            major: 1,
            minor: 0,
            micro: 0,
            preview: 0
        }
    );

    let version = "1";
    let revision: Revision = version.parse().unwrap();
    assert_eq!(
        revision,
        Revision {
            major: 1,
            minor: 0,
            micro: 0,
            preview: 0,
        }
    );

    let version = "1.69";
    let revision: Revision = version.parse().unwrap();
    assert_eq!(
        revision,
        Revision {
            major: 1,
            minor: 69,
            micro: 0,
            preview: 0,
        }
    );

    let version = "0.1.22";
    let revision: Revision = version.parse().unwrap();
    assert_eq!(
        revision,
        Revision {
            major: 0,
            minor: 1,
            micro: 22,
            preview: 0,
        }
    );
    let version = "invalid";
    let revision: Result<Revision, RevisionParseError> = version.parse();
    assert!(revision.is_err());
}

#[test]
fn revision_to_string() {
    let revision = Revision {
        major: 6,
        minor: 78,
        ..Default::default()
    };
    assert_eq!(revision.to_string(), String::from("6.78.0.0"));
    let revision = Revision::default();
    assert_eq!(revision.to_string(), String::from("0.0.0.0"));
    let revision = Revision {
        preview: 9999999,
        ..Default::default()
    };
    assert_eq!(revision.to_string(), String::from("0.0.0.9999999"));
}

#[test]
fn revision_version_compare() {
    // Equal revisions
    assert_eq!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        },
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        }
    );
    assert_eq!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 1
        },
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 1
        }
    );

    // Major version differences
    assert!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        } < Revision {
            major: 2,
            minor: 2,
            micro: 3,
            preview: 0
        }
    );
    assert!(
        Revision {
            major: 2,
            minor: 2,
            micro: 3,
            preview: 0
        } > Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        }
    );

    // Minor version differences
    assert!(
        Revision {
            major: 1,
            minor: 1,
            micro: 3,
            preview: 0
        } < Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        }
    );
    assert!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        } > Revision {
            major: 1,
            minor: 1,
            micro: 3,
            preview: 0
        }
    );

    // Micro version differences
    assert!(
        Revision {
            major: 1,
            minor: 2,
            micro: 2,
            preview: 0
        } < Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        }
    );
    assert!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        } > Revision {
            major: 1,
            minor: 2,
            micro: 2,
            preview: 0
        }
    );

    // Preview version differences
    assert!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        } < Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 1
        }
    );
    assert!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 1
        } > Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 0
        }
    );

    // Mixed version differences
    assert!(
        Revision {
            major: 1,
            minor: 1,
            micro: 2,
            preview: 0
        } < Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 1
        }
    );
    assert!(
        Revision {
            major: 1,
            minor: 2,
            micro: 3,
            preview: 1
        } > Revision {
            major: 1,
            minor: 1,
            micro: 2,
            preview: 0
        }
    );
}
