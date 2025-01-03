use std::borrow::Borrow;
use std::fmt::Display;
use std::io::{self, BufReader};
use std::{error::Error, io::BufWriter};

use anyhow::Context;
use log::warn;
use reqwest::StatusCode;

use crate::caching::properties::{read_properties, PropertiesError};
use crate::config::maven_metadata::parse_maven_metadata;
use crate::pom::VersionRequirement;
use crate::{
    caching::Cache,
    caching::CacheType,
    pom::{parse_pom, Project},
};

use super::resolve::ProjectDep;
pub const CACHE_REPO_STR: &str = "cache";
pub const CENTRAL_REPO_STR: &str = "central";
pub const CENTRAL_REPO_URL: &str = "https://repo1.maven.org/maven2/";
pub const GOOGLE_REPO_STR: &str = "google";
pub const GOOGLE_REPO_URL: &str = "https://maven.google.com/";

pub trait Resolver {
    fn fetch(&self, project: &mut Project) -> Result<String, ResolverError>;
    fn get_name(&self) -> &str;
    fn get_priority(&self) -> i32;
    /// Resolves the correct version of a dependency through use of maven-metadata.xml
    /// Returns the computed version
    fn calculate_version(&self, project: &Project) -> Result<String, ResolverError>;
}
#[derive(Default)]
pub struct CacheResolver {}
pub struct NetResolver {
    base_url: String,
    name: String,
    client: reqwest::blocking::Client,
    priority: i32,
}

#[derive(Debug, Clone, Copy)]
pub enum ResolverErrorKind {
    NotFound,
    Internal,
    ParseError,
    ResponseError,
    NoSelectedVersion,
}

#[derive(Debug)]
pub struct ResolverError {
    message: String,
    kind: ResolverErrorKind,
    source: Option<anyhow::Error>,
}

impl ResolverError {
    pub fn new(message: &str, kind: ResolverErrorKind, source: Option<anyhow::Error>) -> Self {
        ResolverError {
            message: message.to_string(),
            kind,
            source,
        }
    }
    pub fn kind(&self) -> ResolverErrorKind {
        self.kind
    }
}

impl Display for ResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.message)
    }
}

impl Error for ResolverError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|e| &**e as _)
    }
}
impl CacheResolver {
    pub fn new() -> Self {
        CacheResolver {}
    }
}
impl Resolver for CacheResolver {
    fn fetch(&self, project: &mut Project) -> Result<String, ResolverError> {
        // initialize projectDep from project object
        let mut project_dep = ProjectDep::try_from(project.borrow()).map_err(|err| {
            ResolverError::new(
                "Failed to convert Project to ProjectDep.",
                ResolverErrorKind::NoSelectedVersion,
                Some(err),
            )
        })?;

        // try reading the properties from cache checking if error occured
        read_properties(&mut project_dep).map_err(move |err| {
            // check for other errors
            if let Some(prop_error) = err.downcast_ref::<PropertiesError>() {
                match prop_error {
                    // A cache miss
                    PropertiesError::IOError(msg) => {
                        return ResolverError::new(
                            msg.to_string().as_str(),
                            ResolverErrorKind::NotFound,
                            Some(err),
                        )
                    }
                    // A malformed toml error so ideally if it is a cache resolver
                    // we should proceed to do a network fetch. hopefully it should
                    // fix the syntax errors
                    PropertiesError::ParseError => ResolverError::new(
                        "Failed to parse cache properties file",
                        ResolverErrorKind::ParseError,
                        Some(err),
                    ),
                    // home not found, so cache dir is not present
                    PropertiesError::LabtHomeError => ResolverError::new(
                        "Failed to fetch from cache dir",
                        ResolverErrorKind::Internal,
                        Some(err),
                    ),
                }
            } else {
                // failed to resolve from cache,
                // FIXME TODO see why the first condition fails and we result into this else
                ResolverError::new(
                    "Failed to resolve from cache",
                    ResolverErrorKind::NotFound,
                    Some(err),
                )
            }
        })?;

        let deps = project_dep.dependencies.iter().map(|dep| {
            let split: Vec<&str> = dep.splitn(3, ':').collect();
            let group_id = split[0];
            let artifact_id = split[1];
            let version = split[2];

            let mut p = Project::new(group_id, artifact_id, version);
            p.set_selected_version(Some(version.to_string()));
            p
        });

        project.set_packaging(project_dep.packaging);
        project.get_dependencies_mut().extend(deps);

        Ok(project_dep.base_url)
    }
    fn get_name(&self) -> &str {
        CACHE_REPO_STR
    }
    fn get_priority(&self) -> i32 {
        10
    }
    fn calculate_version(&self, project: &Project) -> Result<String, ResolverError> {
        // if it is a softie return imediately unless they specify LATEST or RELEASE for backward compatibility
        if let VersionRequirement::Soft(v) = project.get_version() {
            if v != "LATEST" && v != "RELEASE" {
                return Ok(v.to_string());
            }
        }

        let mut cache = Cache::new(
            project.get_group_id(),
            project.get_artifact_id(),
            String::new(),
            CacheType::METADATA,
        );

        if let Err(err) = cache.use_labt_home() {
            return Err(ResolverError::new(
                "Failed to init LABt home for caching.",
                ResolverErrorKind::Internal,
                Some(err),
            ));
        }

        if !cache.exists() {
            return Err(ResolverError::new(
                "Failed to get maven-metadata.xml from cache",
                ResolverErrorKind::NotFound,
                None,
            ));
        }
        let cache = cache.open().map_err(|err| {
            ResolverError::new(
                "Failed to open maven-metadata.xml cache file",
                ResolverErrorKind::Internal,
                Some(err.into()),
            )
        })?;
        let reader = io::BufReader::new(cache);
        let metadata = parse_maven_metadata(reader).map_err(|err| {
            ResolverError::new(
                format!(
                    "Failed to parse maven-metadata.xml for {}:{}",
                    project.get_group_id(),
                    project.get_artifact_id()
                )
                .as_str(),
                ResolverErrorKind::Internal,
                Some(err),
            )
        })?;

        let selected_version = metadata
            .select_version(project.get_version())
            .map_err(|err| {
                ResolverError::new(
                    format!(
                        "Failed to select correct version for {}:{} from metadata with: latest: {}, release: {} and available {:?}",
                        project.get_group_id(),
                        project.get_artifact_id(),
                        metadata.latest.unwrap_or("None".to_string()),
                        metadata.release.unwrap_or("None".to_string()),
                        metadata.versions
                    )
                    .as_str(),
                    ResolverErrorKind::NoSelectedVersion,
                    Some(err),
                )
            })?;

        Ok(selected_version)
    }
}

impl Resolver for NetResolver {
    fn fetch(&self, project: &mut Project) -> Result<String, ResolverError> {
        let version = project
            .get_selected_version()
            .clone()
            .ok_or(ResolverError::new(
                format!(
                    "Failed to obtain selected version from package {}:{}",
                    project.get_group_id(),
                    project.get_artifact_id()
                )
                .as_str(),
                ResolverErrorKind::NoSelectedVersion,
                None,
            ))?;

        let url = if self.base_url.ends_with('/') {
            format!(
                "{0}{1}/{2}/{3}/{2}-{3}.pom",
                self.base_url,
                project.get_group_id().replace('.', "/"),
                project.get_artifact_id(),
                version
            )
        } else {
            format!(
                "{0}/{1}/{2}/{3}/{2}-{3}.pom",
                self.base_url,
                project.get_group_id().replace('.', "/"),
                project.get_artifact_id(),
                version
            )
        };

        let res = self.client.get(&url).send().map_err(|err| {
            ResolverError::new(
                "Failed to complete the HTTP request for the resolver client",
                ResolverErrorKind::Internal,
                Some(err.into()),
            )
        })?;

        log::trace!(target: "fetch", "{url} {}", res.status());

        if res.status().is_success() {
            let mut reader = io::BufReader::new(res);
            let mut cache = Cache::new(
                project.get_group_id(),
                project.get_artifact_id(),
                version,
                CacheType::POM,
            );

            let parse_result = if let Err(err) = cache.use_labt_home() {
                // if we are unable to initialize cache file, just ignore it.
                // TODO have an effective way to error on this
                warn!("Unable to cache response \n {:?}", err);
                parse_pom(reader, project.to_owned())
            } else {
                // labt home exists so it looks good to pipe everything to cache and return handle to cache
                // no need to check if file exists since its a network resolver anyway

                let mut writer = BufWriter::new(Cache::from(&cache).create().map_err(|err| {
                    ResolverError::new(
                        "Failed to create cache file",
                        ResolverErrorKind::Internal,
                        Some(err.into()),
                    )
                })?);
                std::io::copy(&mut reader, &mut writer).map_err(|err| {
                    ResolverError::new(
                        "Failed to copy network contents to cache file",
                        ResolverErrorKind::Internal,
                        Some(err.into()),
                    )
                })?;
                drop(writer);

                let cache = cache.open().map_err(|err| {
                    ResolverError::new(
                        "Failed to open cache file",
                        ResolverErrorKind::Internal,
                        Some(err.into()),
                    )
                })?;

                let reader = BufReader::new(cache);

                parse_pom(reader, project.to_owned())
            };

            let p = parse_result.map_err(|err| {
                ResolverError::new(
                    format!("Failed to parse pom file at {}", url).as_str(),
                    ResolverErrorKind::Internal,
                    Some(err),
                )
            })?;
            *project = p;
        } else if matches!(res.status(), StatusCode::NOT_FOUND) {
            // 404 not found
            return Err(ResolverError::new(
                format!("{}: Failed to fetch {} ", res.status().as_u16(), url).as_str(),
                ResolverErrorKind::NotFound,
                None,
            ));
        } else {
            return Err(ResolverError::new(
                format!("{}: Failed to fetch {}", res.status().as_u16(), url).as_str(),
                ResolverErrorKind::ResponseError,
                None,
            ));
        }
        Ok(self.base_url.clone())
    }
    fn get_name(&self) -> &str {
        self.name.as_str()
    }
    fn get_priority(&self) -> i32 {
        self.priority
    }
    fn calculate_version(&self, project: &Project) -> Result<String, ResolverError> {
        // if it is a softie return imediately unless they specify LATEST or RELEASE for backward compatibility
        if let VersionRequirement::Soft(v) = project.get_version() {
            if v != "LATEST" && v != "RELEASE" {
                return Ok(v.to_string());
            }
        }

        let url = if self.base_url.ends_with('/') {
            format!(
                "{0}{1}/{2}/maven-metadata.xml",
                self.base_url,
                project.get_group_id().replace('.', "/"),
                project.get_artifact_id(),
            )
        } else {
            format!(
                "{0}/{1}/{2}/maven-metadata.xml",
                self.base_url,
                project.get_group_id().replace('.', "/"),
                project.get_artifact_id(),
            )
        };

        let res = self.client.get(&url).send().map_err(|err| {
            ResolverError::new(
                "Failed to complete the HTTP request for the version resolver client",
                ResolverErrorKind::Internal,
                Some(err.into()),
            )
        })?;

        if res.status().is_success() {
            let mut reader = io::BufReader::new(res);
            let mut cache = Cache::new(
                project.get_group_id(),
                project.get_artifact_id(),
                String::new(),
                CacheType::METADATA,
            );

            let metadata_result = if let Err(err) = cache.use_labt_home() {
                // if we are unable to initialize cache file, just ignore it.
                // TODO have an effective way to error on this
                warn!("Unable to cache response \n {:?}", err);
                parse_maven_metadata(reader)
            } else {
                // we are able to cache, so download and save to cache, then parse from there

                let mut writer = BufWriter::new(Cache::from(&cache).create().map_err(|err| {
                    ResolverError::new(
                        "Failed to create cache file",
                        ResolverErrorKind::Internal,
                        Some(err.into()),
                    )
                })?);
                std::io::copy(&mut reader, &mut writer).map_err(|err| {
                    ResolverError::new(
                        "Failed to copy network contents to cache file",
                        ResolverErrorKind::Internal,
                        Some(err.into()),
                    )
                })?;
                drop(writer);

                let cache = cache.open().map_err(|err| {
                    ResolverError::new(
                        "Failed to open cache file",
                        ResolverErrorKind::Internal,
                        Some(err.into()),
                    )
                })?;

                let reader = BufReader::new(cache);
                parse_maven_metadata(reader)
            };
            let metadata = metadata_result.map_err(|err| {
                ResolverError::new(
                    format!(
                        "Failed to parse maven-metadata.xml for {}:{}",
                        project.get_group_id(),
                        project.get_artifact_id()
                    )
                    .as_str(),
                    ResolverErrorKind::Internal,
                    Some(err),
                )
            })?;

            // Now the reason why we are here
            let versions = project.get_version();

            let selected_version = metadata.select_version(versions).map_err(|err| {
                ResolverError::new(
                    format!(
                        "Failed to select correct version for {}:{} from metadata with: latest: {}, release: {} and available {:?}",
                        project.get_group_id(),
                        project.get_artifact_id(),
                        metadata.latest.unwrap_or("None".to_string()),
                        metadata.release.unwrap_or("None".to_string()),
                        metadata.versions
                    )
                    .as_str(),
                    ResolverErrorKind::NoSelectedVersion,
                    Some(err),
                )
            })?;

            Ok(selected_version)
        } else if matches!(res.status(), StatusCode::NOT_FOUND) {
            // 404 not found
            return Err(ResolverError::new(
                format!("{}: Failed to fetch {} ", res.status().as_u16(), url).as_str(),
                ResolverErrorKind::NotFound,
                None,
            ));
        } else {
            return Err(ResolverError::new(
                format!("{}: Failed to fetch {}", res.status().as_u16(), url).as_str(),
                ResolverErrorKind::ResponseError,
                None,
            ));
        }
    }
}

impl NetResolver {
    pub fn init(name: &str, base_url: &str) -> anyhow::Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(crate::USER_AGENT)
            .build()
            .context("Failed to initialize Net resolver client")?;

        Ok(NetResolver {
            client,
            name: name.to_string(),
            base_url: base_url.to_string(),
            priority: 1,
        })
    }
    pub fn set_priority(&mut self, priority: i32) {
        self.priority = priority;
    }
}

/// Returns the default resolvers
/// Currently this includes cache, central and google
// Returns an error if one of the default resolvers fails initialization
pub fn get_default_resolvers() -> anyhow::Result<Vec<Box<dyn Resolver>>> {
    let cache: Box<dyn Resolver> = Box::new(CacheResolver::new());

    let central: Box<dyn Resolver> =
        Box::new(NetResolver::init(CENTRAL_REPO_STR, CENTRAL_REPO_URL)?);

    let google: Box<dyn Resolver> = Box::new(NetResolver::init(GOOGLE_REPO_STR, GOOGLE_REPO_URL)?);
    Ok(vec![cache, central, google])
}
