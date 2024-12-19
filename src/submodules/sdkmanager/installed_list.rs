use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context};
use toml_edit::{value, ArrayOfTables, Document, Table};

use crate::config::repository::{ChannelType, Revision};
use crate::submodules::sdk::{get_sdk_path, toml_strings};

use super::ToId;

const INSTALLED_LIST: &str = "installed.toml";
const INSTALLED_LIST_OPEN_ERR: &str = "Failed to open sdk installed.toml";
const PACKAGE: &str = "package";
const ACCEPTED_LICENSES: &str = "accepted_licenses";
pub const SDK_PATH_ERR_STRING: &str = "Failed to get android sdk path";

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub struct InstalledPackage {
    pub repository_name: String,
    pub path: String,
    pub version: Revision,
    pub channel: ChannelType,
    pub url: String,
    pub directory: Option<PathBuf>,
}
impl InstalledPackage {
    pub fn new(
        path: String,
        version: Revision,
        channel: ChannelType,
        repository_name: String,
    ) -> Self {
        Self {
            repository_name,
            path,
            version,
            channel,
            url: String::default(),
            directory: None,
        }
    }
}
impl ToId for InstalledPackage {
    fn create_id(&self) -> (&String, &Revision, &ChannelType) {
        (&self.path, &self.version, &self.channel)
    }
}

impl Display for InstalledPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{},{}", self.path, self.version)
    }
}

#[derive(Debug)]
pub struct InstalledListErr {
    kind: InstalledListErrKind,
    file: Option<String>,
}

impl InstalledListErr {
    pub fn new(kind: InstalledListErrKind, file: Option<String>) -> Self {
        Self { kind, file }
    }
}

#[derive(Debug)]
pub enum InstalledListErrKind {
    /// A required key in toml is missing
    MissingKey(&'static str, usize),
    /// Failed converting a toml value to string
    ToStringErr(&'static str, usize),
    /// Failed to read license id entry as string
    LicenseIdStrError(&'static str, usize),
}

impl Display for InstalledListErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const UNKNOWN: &str = "[unknown]";
        match self.kind {
            InstalledListErrKind::MissingKey(key, position) => write!(
                f,
                "{}: Missing {} in table at position {}",
                self.file.as_ref().map_or(UNKNOWN, |p| p.as_str()),
                key,
                position
            ),
            InstalledListErrKind::ToStringErr(key, position) => write!(
                f,
                "{}: Failed to parse {} value as string on table at position {}",
                self.file.as_ref().map_or(UNKNOWN, |p| p.as_str()),
                key,
                position
            ),
            InstalledListErrKind::LicenseIdStrError(key, index) => write!(
                f,
                "{}: Failed to parse {} value as string at index {}",
                self.file.as_ref().map_or(UNKNOWN, |p| p.as_str()),
                key,
                index
            ),
        }
    }
}

impl std::error::Error for InstalledListErr {}

#[derive(Default, Debug)]
pub struct InstalledList {
    pub packages: Vec<InstalledPackage>,
    pub repositories: HashMap<String, RepositoryInfo>,
}

#[derive(Debug)]
pub struct RepositoryInfo {
    /// The repository url
    pub url: String,
    /// A list of licenses that the user pressed accept
    pub accepted_licenses: HashSet<String>,
    /// The repository directory. This is where all the repository data is stored and managed
    pub path: PathBuf,
}

impl InstalledList {
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            repositories: HashMap::new(),
        }
    }
    /// Reads file from disk and parses it into an installed list struct
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(InstalledList::default());
        }

        let mut file = File::open(path)?;
        let mut data = String::new();
        file.read_to_string(&mut data)
            .context(format!("Failed to read ({:?})", path.to_string_lossy()))?;

        data.parse()
    }
    /// Reads the installed.toml from the standard sdk path and returns the resulting parsed list
    pub fn parse_from_sdk() -> anyhow::Result<Self> {
        let mut sdk = get_sdk_path().context(SDK_PATH_ERR_STRING)?;
        sdk.push(INSTALLED_LIST);

        Self::from_file(&sdk)
    }
    pub fn get_hash_map(&self) -> HashMap<String, &InstalledPackage> {
        self.packages.iter().map(|p| (p.to_id(), p)).collect()
    }
    pub fn contains(&self, package: &InstalledPackage) -> bool {
        self.packages.contains(package)
    }
    /// Searches for a package with a given path id. Returns first match.
    pub fn contains_path(&self, path: &String) -> Option<&InstalledPackage> {
        self.packages.iter().find(|p| p.path.eq(path))
    }
    /// Searches for a packages with a given path id. Returns all matches.
    pub fn contains_paths(&self, path: &String) -> Vec<&InstalledPackage> {
        self.packages.iter().filter(|p| p.path.eq(path)).collect()
    }
    /// Searches for a first occurence of a package using `InstalledPackage::to_id`
    pub fn contains_id(&self, package: &InstalledPackage) -> Option<&InstalledPackage> {
        self.packages.iter().find(|p| p.to_id() == package.to_id())
    }
    /// Searches for a first occurence of a package using `InstalledPackage::to_id`. Returns a mutable reference.
    pub fn contains_id_mut(&mut self, package: &InstalledPackage) -> Option<&mut InstalledPackage> {
        self.packages
            .iter_mut()
            .find(|p| p.to_id() == package.to_id())
    }
    /// This will push a package to the end of the list without checking for its existence
    pub fn add_installed_package(&mut self, package: InstalledPackage) {
        self.packages.push(package);
    }
    /// This will try to find a package with same id and replace it with the new package or
    /// add it at the end of package list if missing.
    /// This function uses the package id (`to_id`) to search for matches.
    pub fn insert_installed_package(&mut self, package: InstalledPackage) {
        if let Some(p) = self.contains_id_mut(&package) {
            *p = package;
        } else {
            self.add_installed_package(package);
        }
    }
    /// Removes a package from the list of packages with matching id
    /// This function uses the package id (`to_id`) to search for matches.
    pub fn remove_installed_package(&mut self, package: &InstalledPackage) {
        if let Some((i, _)) = self
            .packages
            .iter()
            .enumerate()
            .find(|(_, p)| p.to_id() == package.to_id())
        {
            self.packages.remove(i);
        }
    }
    /// Checks if user has already accepted a license.
    /// This allows displaying of license for only one time
    pub fn has_accepted(&self, name: &str, license_id: &String) -> Option<bool> {
        self.repositories
            .get(name)
            .map(|repo| repo.accepted_licenses.contains(license_id))
    }
    /// Marks a license as accepted so we don't have to nag the user again to accept
    pub fn accept_license(&mut self, name: &str, license_id: String) {
        if let Some(repo) = self.repositories.get_mut(name) {
            repo.accepted_licenses.insert(license_id);
        }
    }
    pub fn save_to_file(&mut self) -> anyhow::Result<()> {
        let mut sdk = get_sdk_path().context(SDK_PATH_ERR_STRING)?;
        sdk.push(INSTALLED_LIST);

        let mut file = File::create(&sdk).context(format!(
            "Failed to open/create ({:?}) to write installed package list.",
            sdk
        ))?;

        file.write_all(self.to_string().as_bytes())?;

        Ok(())
    }
}

impl FromStr for InstalledList {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use toml_strings::*;
        let doc: Document = s
            .parse()
            .context(format!("Failed to parse {INSTALLED_LIST}"))?;

        let mut repositories: HashMap<String, RepositoryInfo> = HashMap::new();
        if doc.contains_array_of_tables(toml_strings::REPOSITORY) {
            if let Some(repos) = doc[toml_strings::REPOSITORY].as_array_of_tables() {
                for (i, repo_table) in repos.iter().enumerate() {
                    let name = if let Some(name) = repo_table.get(toml_strings::NAME) {
                        name.as_str()
                            .ok_or_else(|| {
                                InstalledListErr::new(
                                    InstalledListErrKind::ToStringErr(PATH, i),
                                    Some(INSTALLED_LIST.to_string()),
                                )
                            })?
                            .to_string()
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(PATH, i),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    };
                    let path: PathBuf = if let Some(path) = repo_table.get(toml_strings::PATH) {
                        PathBuf::from(
                            path.as_str()
                                .ok_or_else(|| {
                                    InstalledListErr::new(
                                        InstalledListErrKind::ToStringErr(PATH, i),
                                        Some(INSTALLED_LIST.to_string()),
                                    )
                                })?
                                .to_string(),
                        )
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(PATH, i),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    };
                    let url = if let Some(url) = repo_table.get(toml_strings::URL) {
                        url.as_str()
                            .ok_or_else(|| {
                                InstalledListErr::new(
                                    InstalledListErrKind::ToStringErr(PATH, i),
                                    Some(INSTALLED_LIST.to_string()),
                                )
                            })?
                            .to_string()
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(PATH, i),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    };

                    let mut accepted_licenses: HashSet<String> = HashSet::new();
                    if repo_table.contains_key(ACCEPTED_LICENSES) {
                        if let Some(list) = repo_table[ACCEPTED_LICENSES].as_array() {
                            for (i, value) in list.iter().enumerate() {
                                let id = value
                                    .as_str()
                                    .ok_or_else(|| {
                                        InstalledListErr::new(
                                            InstalledListErrKind::LicenseIdStrError(
                                                ACCEPTED_LICENSES,
                                                i,
                                            ),
                                            Some(INSTALLED_LIST.to_string()),
                                        )
                                    })?
                                    .to_string();
                                accepted_licenses.insert(id);
                            }
                        }
                    }

                    repositories.insert(
                        name,
                        RepositoryInfo {
                            url,
                            accepted_licenses,
                            path,
                        },
                    );
                }
            }
        }

        let mut package_list: Vec<InstalledPackage> = Vec::new();
        if doc.contains_array_of_tables(PACKAGE) {
            if let Some(packages) = doc[PACKAGE].as_array_of_tables() {
                for package in packages {
                    let mut p = InstalledPackage::default();
                    let position = package.position().unwrap_or(0);

                    // parse path
                    if let Some(path) = package.get(PATH) {
                        p.path = path
                            .as_str()
                            .ok_or_else(|| {
                                InstalledListErr::new(
                                    InstalledListErrKind::ToStringErr(PATH, position),
                                    Some(INSTALLED_LIST.to_string()),
                                )
                            })?
                            .to_string();
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(PATH, position),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    }

                    // the repository name
                    if let Some(name) = package.get(toml_strings::REPOSITORY_NAME) {
                        p.repository_name = name
                            .as_str()
                            .ok_or_else(|| {
                                InstalledListErr::new(
                                    InstalledListErrKind::ToStringErr(PATH, position),
                                    Some(INSTALLED_LIST.to_string()),
                                )
                            })?
                            .to_string()
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(PATH, position),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    }

                    // parse url
                    if let Some(url) = package.get(URL) {
                        p.url = url
                            .as_str()
                            .ok_or_else(|| {
                                InstalledListErr::new(
                                    InstalledListErrKind::ToStringErr(URL, position),
                                    Some(INSTALLED_LIST.to_string()),
                                )
                            })?
                            .to_string();
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(URL, position),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    }

                    // parse version
                    if let Some(version) = package.get(VERSION) {
                        p.version = version
                            .as_str()
                            .ok_or_else(|| {
                                InstalledListErr::new(
                                    InstalledListErrKind::ToStringErr(VERSION, position),
                                    Some(INSTALLED_LIST.to_string()),
                                )
                            })?
                            .parse()
                            .context("Failed to parse version string to revision")?;
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(VERSION, position),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    }

                    // parse channel
                    if let Some(channel) = package.get(CHANNEL) {
                        p.channel = channel
                            .as_str()
                            .ok_or_else(|| {
                                InstalledListErr::new(
                                    InstalledListErrKind::ToStringErr(CHANNEL, position),
                                    Some(INSTALLED_LIST.to_string()),
                                )
                            })?
                            .into();
                    } else {
                        bail!(InstalledListErr::new(
                            InstalledListErrKind::MissingKey(CHANNEL, position),
                            Some(INSTALLED_LIST.to_string()),
                        ));
                    }

                    // parse directory
                    if let Some(directory) = package.get(DIRECTORY) {
                        p.directory = Some(
                            directory
                                .as_str()
                                .ok_or_else(|| {
                                    InstalledListErr::new(
                                        InstalledListErrKind::ToStringErr(DIRECTORY, position),
                                        Some(INSTALLED_LIST.to_string()),
                                    )
                                })?
                                .into(),
                        );
                    }

                    package_list.push(p);
                }
            }
        }
        let installed = Self {
            packages: package_list,
            repositories,
        };
        Ok(installed)
    }
}

impl Display for InstalledList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut doc = toml_edit::Document::new();

        let mut packages = toml_edit::ArrayOfTables::new();

        let mut repository = toml_edit::ArrayOfTables::new();
        for (name, repo) in self.repositories.iter() {
            let mut table = toml_edit::Table::new();
            table.insert(toml_strings::NAME, toml_edit::value(name));

            table.insert(toml_strings::URL, toml_edit::value(&repo.url));

            table.insert(
                toml_strings::PATH,
                toml_edit::value(repo.path.to_string_lossy().to_string()),
            );

            let mut licenses: Vec<&String> = repo.accepted_licenses.iter().collect();
            licenses.sort_unstable();

            let mut accepted = toml_edit::Array::new();
            for id in licenses {
                accepted.push(id);
            }
            table.insert(ACCEPTED_LICENSES, toml_edit::value(accepted));
            repository.push(table);
        }
        doc.insert(
            toml_strings::REPOSITORY,
            toml_edit::Item::ArrayOfTables(repository),
        );

        for package in &self.packages {
            let mut table = toml_edit::Table::new();
            table.insert(
                toml_strings::REPOSITORY_NAME,
                value(&package.repository_name),
            );
            table.insert(toml_strings::PATH, value(&package.path));
            table.insert(toml_strings::VERSION, value(package.version.to_string()));
            table.insert(toml_strings::CHANNEL, value(package.channel.to_string()));
            if let Some(dir) = &package.directory {
                table.insert(
                    toml_strings::DIRECTORY,
                    value(dir.to_string_lossy().to_string()),
                );
            }
            table.insert(toml_strings::URL, value(&package.url));

            packages.push(table);
        }

        doc.insert(PACKAGE, toml_edit::Item::ArrayOfTables(packages));
        write!(f, "{}", doc)
    }
}

/// Writes the provided hashset to a installed.list file in sdk folder
/// Order is not guaranteed as it is a hashmap
pub fn write_installed_list(
    list: Vec<InstalledPackage>,
    writer: &mut dyn Write,
) -> anyhow::Result<()> {
    // let mut sdk = get_sdk_path().context(SDK_PATH_ERR_STRING)?;
    // sdk.push(INSTALLED_LIST);

    let mut doc = toml_edit::Document::new();

    // let mut file = File::create(&sdk).context(INSTALLED_LIST_OPEN_ERR)?;

    let mut packages = toml_edit::ArrayOfTables::new();

    for package in list {
        let mut table = toml_edit::Table::new();
        table.insert(toml_strings::PATH, value(&package.path));
        table.insert(toml_strings::VERSION, value(package.version.to_string()));
        table.insert(toml_strings::CHANNEL, value(package.channel.to_string()));
        if let Some(dir) = package.directory {
            table.insert(
                toml_strings::DIRECTORY,
                value(dir.to_string_lossy().to_string()),
            );
        }
        table.insert(toml_strings::URL, value(package.url));

        packages.push(table);
    }

    doc.insert(PACKAGE, toml_edit::Item::ArrayOfTables(packages));
    writer.write_all(doc.to_string().as_bytes())?;
    // .context(format!(
    //     "Failed to write installed sdk package list to {}",
    //     sdk.to_string_lossy()
    // ))?;

    Ok(())
}

/// Inserts or updates an installed package entry on installed list
pub fn update_installed_list(
    package: InstalledPackage,
) -> anyhow::Result<HashMap<String, InstalledPackage>> {
    let mut sdk = get_sdk_path().context(SDK_PATH_ERR_STRING)?;
    sdk.push(INSTALLED_LIST);

    let mut installed: HashMap<String, InstalledPackage> = HashMap::new();

    if !sdk.exists() {
        let mut file = File::create(&sdk).context(INSTALLED_LIST_OPEN_ERR)?;
        write_installed_list(vec![package.clone()], &mut file)?;
        installed.insert(package.to_id(), package);
        return Ok(installed);
    }

    let data = fs::read_to_string(&sdk).context(format!("Failed to read ({:?})", sdk))?;
    let mut doc: Document = data
        .parse()
        .context(format!("Failed to parse ({:?})", sdk))?;

    let mut array = doc[PACKAGE]
        .as_array_of_tables()
        .map_or(ArrayOfTables::default(), |t| t.to_owned());

    let mut table = Table::new();
    table.insert(toml_strings::PATH, value(&package.path));
    table.insert(toml_strings::VERSION, value(package.version.to_string()));
    table.insert(toml_strings::CHANNEL, value(package.channel.to_string()));
    table.insert(toml_strings::URL, value(&package.url));
    if let Some(dir) = &package.directory {
        table.insert(
            toml_strings::DIRECTORY,
            value(dir.to_string_lossy().to_string()),
        );
    }

    array.push(table);

    let mut table = Table::new();
    table.insert(toml_strings::PATH, value(&package.path));
    table.insert(toml_strings::VERSION, value(package.version.to_string()));
    table.insert(toml_strings::CHANNEL, value(package.channel.to_string()));
    table.insert(toml_strings::URL, value(&package.url));
    if let Some(dir) = package.directory {
        table.insert(
            toml_strings::DIRECTORY,
            value(dir.to_string_lossy().to_string()),
        );
    }

    array.push(table);
    doc.insert(PACKAGE, toml_edit::Item::ArrayOfTables(array));

    println!("{}", doc);

    Ok(installed)
}

#[cfg(test)]
mod installed_list_test {
    use std::collections::HashSet;

    use crate::{
        config::repository::{ChannelType, Revision},
        submodules::sdkmanager::{installed_list::RepositoryInfo, ToId},
    };
    use pretty_assertions::assert_eq;

    use super::{InstalledList, InstalledPackage};

    #[test]
    fn add_package() {
        let package_1: InstalledPackage = InstalledPackage {
            repository_name: "google".to_string(),
            path: "sdk:package1".to_string(),
            version: Revision::new(1),
            channel: ChannelType::Stable,
            url: "gitlab.com".to_string(),
            directory: None,
        };

        let mut list = InstalledList::new();
        list.add_installed_package(package_1.clone());

        assert!(list.packages.contains(&package_1));
    }

    #[test]
    fn insert_package() {
        let package_1: InstalledPackage = InstalledPackage {
            repository_name: "google".to_string(),
            path: "sdk:package1".to_string(),
            version: Revision::new(1),
            channel: ChannelType::Stable,
            url: "gitlab.com".to_string(),
            directory: None,
        };
        let package_2: InstalledPackage = InstalledPackage {
            repository_name: "google".to_string(),
            path: "sdk:package2".to_string(),
            version: Revision::new(1),
            channel: ChannelType::Stable,
            url: "gitlab.com".to_string(),
            directory: None,
        };

        let mut list = InstalledList::new();
        list.add_installed_package(package_1.clone());
        list.add_installed_package(package_2.clone());

        // insert a package not available in list
        let package_3: InstalledPackage = InstalledPackage {
            repository_name: "google".to_string(),
            path: "sdk:package3".to_string(),
            version: Revision::new(1),
            channel: ChannelType::Stable,
            url: "gitlab.com".to_string(),
            directory: None,
        };

        list.insert_installed_package(package_3);
        assert!(list.packages.len().eq(&3), "List length is not equal to 3");

        // try to re update package 1
        let package_1 = InstalledPackage {
            url: "example.com".to_string(),
            ..package_1
        };

        list.insert_installed_package(package_1.clone());
        assert!(list.packages.len().eq(&3), "List length is not equal to 3");

        assert_eq!(list.packages[0].url, package_1.url);
    }
    #[test]
    fn installed_package_list_from_str() {
        let toml = r#"
[[repository]]
name = "google"
url = "https://repo.google.com"
path = ".labt/sdk/google"
accepted_licenses = []

[[package]]
repository_name = "google"
path = "extras;google;auto"
version = "2.0.0.0"
channel = "stable"
url = "http://example.com"
"#;

        let result: InstalledList = toml.parse().unwrap();
        let mut iter = result.packages.iter();
        let value: &InstalledPackage = iter.next().unwrap();

        let package = InstalledPackage {
            repository_name: "google".to_string(),
            path: "extras;google;auto".to_string(),
            version: "2.0.0.0".parse().unwrap(),
            channel: ChannelType::Stable,
            url: "http://example.com".to_string(),
            directory: None,
        };

        assert_eq!(value.to_id(), package.to_id());
        assert_eq!(value, &package);

        let repo = RepositoryInfo {
            url: "https://repo.google.com".to_string(),
            accepted_licenses: HashSet::new(),
            path: ".labt/sdk/google".into(),
        };

        assert_eq!(result.repositories.len(), 1);
        let google_repo = result.repositories.get("google").unwrap();

        assert_eq!(google_repo.url, repo.url);
        assert_eq!(google_repo.path, repo.path);
        assert_eq!(google_repo.accepted_licenses, repo.accepted_licenses);
    }
    #[test]
    fn installed_package_list_to_toml() {
        let mut list = InstalledList::new();

        list.repositories.insert(
            "google".to_string(),
            RepositoryInfo {
                url: "https://repo.google.com".to_string(),
                accepted_licenses: HashSet::new(),
                path: ".labt/sdk/google".into(),
            },
        );

        let package = InstalledPackage {
            repository_name: "google".to_string(),
            path: "extras;google;auto".to_string(),
            version: "2.0.0.0".parse().unwrap(),
            channel: ChannelType::Stable,
            url: "http://example.com".to_string(),
            directory: None,
        };

        list.add_installed_package(package.clone());

        let toml = r#"
[[repository]]
name = "google"
url = "https://repo.google.com"
path = ".labt/sdk/google"
accepted_licenses = []

[[package]]
repository_name = "google"
path = "extras;google;auto"
version = "2.0.0.0"
channel = "stable"
url = "http://example.com"
"#;
        assert_eq!(list.to_string(), toml.trim_start());
    }
}
