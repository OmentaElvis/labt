use std::error::Error;
use std::fmt::Display;
use std::io;

use anyhow::Context;
use reqwest::StatusCode;

use crate::{
    pom::{parse_pom, Project},
};

pub trait Resolver {
    fn fetch(&self, project: &mut Project) -> Result<(), ResolverError>;
    fn get_name(&self) -> String;
}

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

impl Resolver for CacheResolver {
    fn fetch(&self, _project: &mut Project) -> Result<(), ResolverError> {
        todo!();
    }
    fn get_name(&self) -> String {
        String::from("cache")
    }
}

impl Resolver for NetResolver {
    fn fetch(&self, project: &mut Project) -> Result<(), ResolverError> {
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
            let reader = io::BufReader::new(res);
            let p = parse_pom(reader, project.to_owned()).map_err(|err| {
                ResolverError::new(
                    format!("Failed to parse pom file at {}", url).as_str(),
                    ResolverErrorKind::Internal,
                    Some(err),
                )
            })?;
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
        Ok(())
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
