use std::borrow::Borrow;
use std::fmt::Display;
use std::io::{self, BufReader};
use std::{error::Error, io::BufWriter};

use anyhow::Context;
use log::warn;
use reqwest::StatusCode;

use crate::caching::properties::{read_properties, PropertiesError};
use crate::{
    caching::Cache,
    caching::CacheType,
    pom::{parse_pom, Project},
};

use super::resolve::ProjectDep;

pub trait Resolver {
    fn fetch(&self, project: &mut Project) -> Result<String, ResolverError>;
    fn get_name(&self) -> String;
}
#[derive(Default)]
pub struct CacheResolver {}
pub struct NetResolver {
    base_url: String,
    name: String,
    client: reqwest::blocking::Client,
}

#[derive(Debug, Clone, Copy)]
pub enum ResolverErrorKind {
    NotFound,
    Internal,
    ParseError,
    ResponseError,
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
        let mut project_dep = ProjectDep::from(project.borrow());

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

            Project::new(group_id, artifact_id, version)
        });

        project.set_packaging(project_dep.packaging);
        project.get_dependencies_mut().extend(deps);

        Ok(project_dep.url)
    }
    fn get_name(&self) -> String {
        String::from("cache")
    }
}

impl Resolver for NetResolver {
    fn fetch(&self, project: &mut Project) -> Result<String, ResolverError> {
        let url = format!(
            "{0}/{1}/{2}/{3}/{2}-{3}.pom",
            self.base_url,
            project.get_group_id().replace('.', "/"),
            project.get_artifact_id(),
            project.get_version(),
        );

        let res = self.client.get(&url).send().map_err(|err| {
            ResolverError::new(
                "Failed to complete the HTTP request for the resolver client",
                ResolverErrorKind::Internal,
                Some(err.into()),
            )
        })?;

        if res.status().is_success() {
            let mut reader = io::BufReader::new(res);
            let mut cache = Cache::new(
                project.get_group_id(),
                project.get_artifact_id(),
                project.get_version(),
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
            project.set_packaging(p.get_packaging());
            project
                .get_dependencies_mut()
                .extend(p.get_dependencies().iter().map(|dep| dep.to_owned()));
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
        let base_url = format!(
            "{0}/{1}/{2}/{3}/",
            self.base_url,
            project.get_group_id().replace('.', "/"),
            project.get_artifact_id(),
            project.get_version(),
        );
        Ok(base_url)
    }
    fn get_name(&self) -> String {
        self.name.clone()
    }
}

impl NetResolver {
    pub fn init(name: &str, base_url: &str) -> anyhow::Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("Labt/1.1")
            .build()
            .context("Failed to initialize Net resolver client")?;

        Ok(NetResolver {
            client,
            name: name.to_string(),
            base_url: base_url.to_string(),
        })
    }
}
