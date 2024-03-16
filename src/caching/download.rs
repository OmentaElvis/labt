use std::io::{copy, BufReader, BufWriter};

use anyhow::Context;
use reqwest::Url;

use crate::submodules::resolve::ProjectDep;

use super::Cache;

pub fn download(project: &ProjectDep) -> anyhow::Result<u64> {
    let client = reqwest::blocking::ClientBuilder::new()
        .user_agent("Labt/1.0")
        .build()
        .context("Error creating download client")?;
    let base = Url::parse(&project.url).context("Error parsing repo url")?;
    let ext = if project.packaging.is_empty() {
        String::from("jar")
    } else {
        project.packaging.clone()
    };

    let url = base.join(format!("{}-{}.{}", project.artifact_id, project.version, ext).as_str())?;
    let res = client.get(url).send()?;
    if res.status().is_success() {
        let mut cache = Cache::from(project);
        cache.use_labt_home()?;
        let cache = cache.create()?;

        let mut writer = BufWriter::new(cache);
        let mut reader = BufReader::new(res);
        return copy(&mut reader, &mut writer)
            .context("Failed copying network bytes to cached file");
    }
    res.error_for_status()
        .context("Failed to complete request")?;
    Ok(0)
}
