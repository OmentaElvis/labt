use std::{
    fs::{create_dir_all, File},
    io::{Read, Write},
    path::PathBuf,
};

pub mod download;
pub mod properties;

use anyhow::{bail, Context};
use indicatif::{HumanBytes, ProgressBar};
use log::info;

use crate::{get_home, submodules::resolve::ProjectDep, MULTI_PROGRESS_BAR};

use self::{download::download, properties::write_properties};
#[derive(Clone, Debug)]
pub enum CacheType {
    POM,
    AAR,
    JAR,
    SOURCE,
    PROPERTIES,
    UNKNOWN(String),
}
#[derive(Debug)]
pub struct Cache {
    group_id: String,
    artifact_id: String,
    version: String,
    cache_type: CacheType,
    path: Option<PathBuf>,
    file: Option<File>,
}

impl Cache {
    pub fn new(
        group_id: String,
        artifact_id: String,
        version: String,
        cache_type: CacheType,
    ) -> Self {
        Cache {
            group_id,
            artifact_id,
            version,
            cache_type,
            path: None,
            file: None,
        }
    }
    pub fn get_cache_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }
    pub fn set_cache_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }
    pub fn use_labt_home(&mut self) -> anyhow::Result<()> {
        let mut path = get_home().context("Unable to get home dir for caching")?;
        path.push("cache");
        self.path = Some(path);
        Ok(())
    }
    fn get_name_from_type(&self) -> String {
        match &self.cache_type {
            CacheType::POM => format!("{}-{}.pom", self.artifact_id, self.version),
            CacheType::AAR => format!("{}-{}.aar", self.artifact_id, self.version),
            CacheType::JAR => format!("{}-{}.jar", self.artifact_id, self.version),
            CacheType::SOURCE => format!("{}-{}-source.jar", self.artifact_id, self.version),
            CacheType::UNKNOWN(ext) => {
                format!("{}-{}.{}", self.artifact_id, self.version, ext)
            }
            CacheType::PROPERTIES => format!("{}-{}.toml", self.artifact_id, self.version),
        }
    }
    fn build_path(&self) -> std::io::Result<PathBuf> {
        if self.path.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Labt home path not intitialized",
            ));
        }

        let mut path = self.path.clone().unwrap();
        path.push(&self.group_id);
        path.push(&self.artifact_id);
        path.push(&self.version);
        if !path.exists() {
            create_dir_all(&path)?;
        }
        path.push(self.get_name_from_type());

        Ok(path)
    }
    pub fn create(self) -> std::io::Result<Cache> {
        let mut cache = self;
        let path = cache.build_path()?;
        let file = File::create(path)?;
        cache.file = Some(file);
        Ok(cache)
    }
    pub fn open(self) -> std::io::Result<Cache> {
        let mut cache = self;
        let path = cache.build_path()?;
        let file = File::open(path)?;
        cache.file = Some(file);
        Ok(cache)
    }
    /// Checks if this cache entry exists
    /// returns false if Labt home is not initialized
    pub fn exists(&self) -> bool {
        if let Ok(path) = self.build_path() {
            path.exists()
        } else {
            false
        }
    }
    /// Returns the expected path representation of this cache entry.
    /// This is not the actual location of an existing file but rather where
    /// it is expected to be if it exists.
    /// # Errors
    /// Returns an error if the base cache directory was not initialized before
    /// calling this function
    pub fn get_path(&self) -> anyhow::Result<PathBuf> {
        if self.path.is_none() {
            bail!("Cache base dir not specified.");
        }
        let mut path = self.path.clone().unwrap();
        path.push(&self.group_id);
        path.push(&self.artifact_id);
        path.push(&self.version);
        path.push(self.get_name_from_type());

        Ok(path)
    }
}

impl Write for Cache {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(file) = &mut self.file {
            file.write(buf)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid state: cache file not initialized",
            ))
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush()
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid state: cache file not initialized",
            ))
        }
    }
}
impl Read for Cache {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(file) = &mut self.file {
            file.read(buf)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid state: cache file not initialized",
            ))
        }
    }
}
impl From<ProjectDep> for Cache {
    /// initialize a new Cache file from a ProjectDep
    fn from(value: ProjectDep) -> Self {
        Cache::new(
            value.group_id,
            value.artifact_id,
            value.version,
            CacheType::from(value.packaging),
        )
    }
}
impl From<&ProjectDep> for Cache {
    /// initialize a new Cache file from a ProjectDep reference
    fn from(value: &ProjectDep) -> Self {
        Cache::new(
            value.group_id.clone(),
            value.artifact_id.clone(),
            value.version.clone(),
            CacheType::from(value.packaging.clone()),
        )
    }
}

impl From<String> for CacheType {
    /// Converts cachetype from string to CacheType enum
    fn from(value: String) -> Self {
        match value.as_str() {
            "pom" => CacheType::POM,
            "aar" => CacheType::AAR,
            "jar" => CacheType::JAR,
            "source" => CacheType::SOURCE,
            "toml" => CacheType::PROPERTIES,
            _ => CacheType::UNKNOWN(value),
        }
    }
}
impl From<&Cache> for Cache {
    // recycle properties of provided Cache to create a new one
    fn from(cache: &Cache) -> Self {
        Cache {
            group_id: cache.group_id.clone(),
            artifact_id: cache.artifact_id.clone(),
            version: cache.version.clone(),
            cache_type: cache.cache_type.clone(),
            path: cache.path.clone(),
            file: None,
        }
    }
}

pub fn save_dependencies(deps: &Vec<ProjectDep>) -> anyhow::Result<()> {
    // if it was a cache miss, then write properties to file for the next resolution
    for project in deps.iter().filter(|p| !p.cache_hit) {
        write_properties(project)?;
    }
    // initialize a new progressbar
    let pb =
        MULTI_PROGRESS_BAR.with(|multi| multi.borrow().add(ProgressBar::new(deps.len() as u64)));
    // begin the download  of the dependencies
    for project in deps {
        let mut cache = Cache::from(project);
        cache.use_labt_home().context(format!(
            "Unable to access Labt home for {}:{}:{}",
            project.group_id, project.artifact_id, project.version
        ))?;
        // increment progressbar
        pb.inc(1);
        // if it is a cache hit, skip
        if cache.exists() {
            info!(target: "fetch", "Cache hit {}", cache.get_name_from_type());
            continue;
        }
        let size = download(project).context(format!(
            "Failed to download dependency from [{}]",
            project.get_root_url()
        ))?;
        info!(target: "fetch", "Downloaded {} {}", cache.get_name_from_type(), HumanBytes(size));
    }

    Ok(())
}
