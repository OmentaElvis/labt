use std::{
    fs::{create_dir_all, File},
    io::{Read, Write},
    path::PathBuf,
};

use anyhow::Context;

use crate::get_home;
#[derive(Clone, Debug)]
pub enum CacheType {
    POM,
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
    pub fn from(cache: &Cache) -> Self {
        Cache {
            group_id: cache.group_id.clone(),
            artifact_id: cache.artifact_id.clone(),
            version: cache.version.clone(),
            cache_type: cache.cache_type.clone(),
            path: cache.path.clone(),
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
        match self.cache_type {
            CacheType::POM => format!("{}-{}.pom", self.artifact_id, self.version),
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
