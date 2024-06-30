use std::{
    collections::HashSet,
    fs::{create_dir, create_dir_all, File},
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{bail, Context};
use clap::{Args, Subcommand};
use log::info;
use reqwest::Url;
use toml_edit::{value, Document};

use crate::{
    config::repository::{parse_repository_xml, Archive, RemotePackage, RepositoryXml, Revision},
    get_home,
    tui::{self, sdkmanager::SdkManager, Tui},
};

// consts
const DEFAULT_RESOURCES_URL: &str = "https://dl.google.com/android/repository/repository2-1.xml";
const SDKMANAGER_TARGET: &str = "sdkmanager";

use super::sdkmanager::filters::FilteredPackages;
use super::sdkmanager::read_installed_list;
use super::Submodule;

pub use super::sdkmanager::InstalledPackage;

#[derive(Clone, Args)]
pub struct SdkArgs {
    /// The repository.xml url to fetch sdk list
    #[arg(long)]
    repository_xml: Option<String>,
    /// Force updates the android repository xml
    #[arg(long, action)]
    update_repository_list: bool,
}

pub struct Sdk {
    url: String,
    update: bool,
}

impl Sdk {
    pub fn new(args: &SdkArgs) -> Self {
        let url = if let Some(url) = args.repository_xml.clone() {
            url
        } else {
            String::from(DEFAULT_RESOURCES_URL)
        };
        Self {
            url,
            update: args.update_repository_list,
        }
    }
    pub fn start_tui(
        &self,
        repo: RepositoryXml,
        list: HashSet<InstalledPackage>,
    ) -> io::Result<()> {
        let mut terminal: Tui = tui::init()?;
        terminal.clear()?;
        SdkManager::new(Rc::new(repo), Rc::new(list)).run(&mut terminal)?;
        tui::restore()?;

        Ok(())
    }
    pub fn get_url(&self) -> &String {
        &self.url
    }
}

mod toml_strings {
    pub const PATH: &str = "path";
    pub const VERSION: &str = "version";
    pub const DISPLAY_NAME: &str = "display_name";
    pub const LICENSE: &str = "license";
    pub const CHANNEL: &str = "channel";
    pub const URL: &str = "url";
    pub const CHECKSUM: &str = "checksum";
    pub const SIZE: &str = "size";
    pub const OS: &str = "os";
    pub const BITS: &str = "bits";
    pub const ARCHIVE: &str = "archive";
    pub const OBSOLETE: &str = "obsolete";
    pub const REMOTE_PACKAGE: &str = "remote_package";
    pub const CONFIG_FILE: &str = "repository.toml";
}

// Entry point
impl Submodule for Sdk {
    fn run(&mut self) -> anyhow::Result<()> {
        // check for sdk folder
        let sdk = get_sdk_path().context(super::sdkmanager::installed_list::SDK_PATH_ERR_STRING)?;

        let url = Url::parse(&self.url).context("Failed to parse repository url")?;

        // confirm a repository.toml exists
        let mut toml = sdk.clone();
        toml.push(toml_strings::CONFIG_FILE);

        let repo = if !toml.exists() || self.update {
            info!(target: SDKMANAGER_TARGET, "Fetching android repository xml from {}", url.as_str());
            let client = reqwest::blocking::Client::builder()
                .user_agent(crate::USER_AGENT)
                .build()
                .context(format!(
                    "Failed to create http client to fetch {}",
                    url.as_str()
                ))?;
            let resp = client
                .get(url.clone())
                .send()
                .context(format!("Failed to complete request to {}", url.as_str()))?;
            let reader = BufReader::new(resp);
            let repo = parse_repository_xml(reader).context(format!(
                "Failed to parse android repository from {}",
                url.as_str()
            ))?;
            write_repository_config(&repo)
                .context("Failed to write repository config to LABt home cache")?;
            repo
        } else {
            info!(target: SDKMANAGER_TARGET, "Fetching cached repository config file");
            parse_repository_toml(&toml).context("Failed to parse android repository config from cache. try --update-repository-list to force update config.")?
        };

        let list = read_installed_list().context("Failed reading installed packages list")?;

        self.start_tui(repo, list)?;
        Ok(())
    }
}

                self.list_packages(args, repo, list)

        Ok(())
    }
}

/// Returns LABt root android sdk folder
pub fn get_sdk_path() -> anyhow::Result<PathBuf> {
    let mut sdk = get_home().context("Failed to get LABt home")?;
    sdk.push("sdk");

    // create sdk folder
    if !sdk.exists() {
        create_dir_all(&sdk).context("Failed to create sdk path in LABt home")?;
    }

    Ok(sdk)
}

pub fn write_repository_config(repo: &RepositoryXml) -> anyhow::Result<()> {
    use toml_strings::*;
    // Check for sdk folder
    let sdk = get_sdk_path().context("Failed to get android sdk path")?;

    // Create licenses page
    let mut licenses = sdk.clone();
    licenses.push("licenses");

    if !licenses.exists() {
        create_dir(&licenses).context("Failed to create licenses path in LABt home")?;
    }

    for (key, license) in repo.get_licenses() {
        let mut path = licenses.clone();
        path.push(key.clone());
        let mut file =
            File::create(&path).context(format!("Failed to open {} license file", key))?;

        file.write_all(license.as_bytes()).context(format!(
            "Failed to write to license file: {}",
            path.to_string_lossy()
        ))?;
    }

    // write the toml to file
    let mut doc = toml_edit::Document::new();
    let mut remotes = toml_edit::ArrayOfTables::new();
    for package in repo.get_remote_packages() {
        let mut table = toml_edit::Table::new();
        table.insert(PATH, value(package.get_path()));
        table.insert(VERSION, value(package.get_revision().to_string()));
        table.insert(DISPLAY_NAME, value(package.get_display_name()));
        table.insert(LICENSE, value(package.get_uses_license()));
        table.insert(CHANNEL, value(package.get_channel_ref()));
        let mut archive_entries = toml_edit::ArrayOfTables::new();
        for archive in package.get_archives() {
            let mut archive_table = toml_edit::Table::new();
            archive_table.insert(URL, value(archive.get_url()));
            archive_table.insert(CHECKSUM, value(archive.get_checksum()));
            archive_table.insert(SIZE, value(archive.get_size() as i64));

            if !archive.get_host_os().is_empty() {
                archive_table.insert(OS, value(archive.get_host_os()));
            }
            match archive.get_host_bits() {
                crate::config::repository::BitSizeType::Bit64 => {
                    archive_table.insert(BITS, value(64));
                }
                crate::config::repository::BitSizeType::Bit32 => {
                    archive_table.insert(BITS, value(32));
                }
                _ => {}
            }
            archive_entries.push(archive_table);
        }
        table[ARCHIVE] = toml_edit::Item::ArrayOfTables(archive_entries);
        if package.get_obsolete() {
            table.insert(OBSOLETE, value(true));
        }
        remotes.push(table);
    }
    doc[REMOTE_PACKAGE] = toml_edit::Item::ArrayOfTables(remotes);

    let mut repository = sdk.clone();
    repository.push("repository.toml");
    let mut file = File::create(&repository).context(format!(
        "Failed to open repository config at {}",
        repository.to_string_lossy()
    ))?;
    file.write_all(doc.to_string().as_bytes()).context(format!(
        "Failed to write config to {}",
        repository.to_string_lossy()
    ))?;
    Ok(())
}

pub fn parse_repository_toml(path: &Path) -> anyhow::Result<RepositoryXml> {
    let mut file = File::open(path).context(format!(
        "Failed to open android repository config at {}",
        path.to_string_lossy()
    ))?;

    let mut doc = String::new();
    file.read_to_string(&mut doc).context(format!(
        "Failed to read config file {}",
        path.to_string_lossy()
    ))?;
    let toml: Document = doc.parse().context(format!(
        "Failed to parse repository config file {}",
        path.to_string_lossy()
    ))?;

    use toml_strings::*;
    let missing_err = |key: &str, position: usize| -> anyhow::Result<()> {
        bail!(
            "repository.toml: Missing {} in table at position {} ",
            key,
            position
        );
    };

    let mut repo = RepositoryXml::new();
    if toml.contains_array_of_tables(REMOTE_PACKAGE) {
        if let Some(packages) = toml[REMOTE_PACKAGE].as_array_of_tables() {
            for p in packages {
                let mut package = RemotePackage::new();
                let position = p.position().unwrap_or(0);

                // parse path
                if let Some(path) = p.get(PATH) {
                    package.set_path(
                        path.as_value()
                            .unwrap_or(&toml_edit::Value::String(toml_edit::Formatted::new(
                                String::new(),
                            )))
                            .as_str()
                            .unwrap()
                            .to_string(),
                    );
                } else {
                    missing_err(PATH, position)?;
                }
                // Parse version
                if let Some(version) = p.get(VERSION) {
                    let version = version.as_str().unwrap();
                    let revision: Revision = version
                        .parse()
                        .context(format!("Failed to parse version string: {}", version))?;

                    package.set_revision(revision);
                } else {
                    missing_err(VERSION, position)?;
                }
                // Parse display name
                if let Some(display_name) = p.get(DISPLAY_NAME) {
                    package.set_display_name(
                        display_name
                            .as_value()
                            .unwrap_or(&toml_edit::Value::String(toml_edit::Formatted::new(
                                String::new(),
                            )))
                            .as_str()
                            .unwrap()
                            .to_string(),
                    );
                } else {
                    missing_err(DISPLAY_NAME, position)?;
                }

                // Parse license
                if let Some(license) = p.get(LICENSE) {
                    package.set_license(
                        license
                            .as_value()
                            .unwrap_or(&toml_edit::Value::String(toml_edit::Formatted::new(
                                String::new(),
                            )))
                            .as_str()
                            .unwrap()
                            .to_string(),
                    );
                }
                // Parse channel
                if let Some(channel) = p.get(CHANNEL) {
                    package.set_channel_ref(
                        channel
                            .as_value()
                            .unwrap_or(&toml_edit::Value::String(toml_edit::Formatted::new(
                                String::new(),
                            )))
                            .as_str()
                            .unwrap()
                            .to_string(),
                    )
                }

                // parse archives
                if let Some(archives) = p.get(ARCHIVE) {
                    let archives_array = archives.as_array_of_tables().unwrap();
                    for entry in archives_array {
                        let mut archive = Archive::default();
                        // url
                        if let Some(url) = entry.get(URL) {
                            archive.set_url(url.as_str().unwrap().to_string());
                        }
                        // checksum
                        if let Some(checksum) = entry.get(CHECKSUM) {
                            archive.set_checksum(checksum.as_str().unwrap().to_string());
                        }

                        // os
                        if let Some(os) = entry.get(OS) {
                            archive.set_host_os(os.as_str().unwrap().to_string());
                        }
                        // size
                        if let Some(size) = entry.get(SIZE) {
                            archive.set_size(size.as_integer().unwrap() as usize)
                        }

                        // bits
                        if let Some(bits) = entry.get(BITS) {
                            archive.set_host_bits(match bits.as_integer().unwrap() {
                                64 => crate::config::repository::BitSizeType::Bit64,
                                32 => crate::config::repository::BitSizeType::Bit32,
                                _ => crate::config::repository::BitSizeType::Unset,
                            });
                        }

                        package.add_archive(archive);
                    }
                }
                repo.add_remote_package(package);
            }
        }
    }

    Ok(repo)
}
