use std::collections::HashMap;
use std::fmt::Display;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context};
use toml_edit::{value, ArrayOfTables, Document, Table};

use crate::config::repository::{ChannelType, Revision};
use crate::submodules::sdk::{get_sdk_path, toml_strings};

const INSTALLED_LIST: &str = "installed.toml";
const INSTALLED_LIST_OPEN_ERR: &str = "Failed to open sdk installed.toml";
const PACKAGE: &str = "package";
pub const SDK_PATH_ERR_STRING: &str = "Failed to get android sdk path";

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub struct InstalledPackage {
    pub path: String,
    pub version: Revision,
    pub channel: ChannelType,
    pub url: String,
    pub directory: Option<PathBuf>,
}
impl InstalledPackage {
    pub fn new(path: String, version: Revision, channel: ChannelType) -> Self {
        Self {
            path,
            version,
            channel,
            url: String::default(),
            directory: None,
        }
    }
    pub fn to_id(&self) -> String {
        format!("{}:{}:{}", self.path, self.version, self.channel)
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
        }
    }
}

impl std::error::Error for InstalledListErr {}

#[derive(Default, Debug)]
pub struct InstalledList {
    pub packages: Vec<InstalledPackage>,
}

impl InstalledList {
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
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
        };
        Ok(installed)
    }
}

impl Display for InstalledList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut doc = toml_edit::Document::new();

        let mut packages = toml_edit::ArrayOfTables::new();

        for package in &self.packages {
            let mut table = toml_edit::Table::new();
            table.insert(toml_strings::PATH, value(&package.path));
            table.insert(toml_strings::VERSION, value(package.version.to_string()));
            table.insert(toml_strings::CHANNEL, value(package.channel.to_string()));
            if let Some(dir) = &package.directory {
                table.insert(
                    toml_strings::DIRECTORY,
                    value(&dir.to_string_lossy().to_string()),
                );
            }
            table.insert(toml_strings::URL, value(&package.url));

            packages.push(table);
        }

        doc.insert(PACKAGE, toml_edit::Item::ArrayOfTables(packages));
        write!(f, "{}", doc)
    }
}

/// Parses the provided toml string into Installed list
pub fn parse_installed_list(data: &str) -> anyhow::Result<HashMap<String, InstalledPackage>> {
    use toml_strings::*;
    let doc: Document = data
        .parse()
        .context(format!("Failed to parse {INSTALLED_LIST}"))?;

    let mut installed: HashMap<String, InstalledPackage> = HashMap::new();
    let missing_err = |key: &str, position: usize| -> anyhow::Result<()> {
        bail!(
            "{INSTALLED_LIST}: Missing {} in table at position {} ",
            key,
            position
        );
    };

    let as_str_err = |key: &str, position: usize| {
        anyhow::anyhow!(
          "{INSTALLED_LIST}: Failed to parse {key} value as string on table at position {position}"
      )
    };

    if doc.contains_array_of_tables(PACKAGE) {
        if let Some(packages) = doc[PACKAGE].as_array_of_tables() {
            for package in packages {
                let mut p = InstalledPackage::default();
                let position = package.position().unwrap_or(0);

                // parse path
                if let Some(path) = package.get(PATH) {
                    p.path = path
                        .as_str()
                        .ok_or_else(|| as_str_err(PATH, position))?
                        .to_string();
                } else {
                    missing_err(PATH, position)?;
                }

                // parse url
                if let Some(url) = package.get(URL) {
                    p.url = url
                        .as_str()
                        .ok_or_else(|| as_str_err(URL, position))?
                        .to_string();
                } else {
                    missing_err(URL, position)?;
                }

                // parse version
                if let Some(version) = package.get(VERSION) {
                    p.version = version
                        .as_str()
                        .ok_or_else(|| as_str_err(VERSION, position))?
                        .parse()
                        .context("Failed to parse version string to revision")?;
                } else {
                    missing_err(VERSION, position)?;
                }

                // parse channel
                if let Some(channel) = package.get(CHANNEL) {
                    p.channel = channel
                        .as_str()
                        .ok_or_else(|| as_str_err(CHANNEL, position))?
                        .into();
                } else {
                    missing_err(CHANNEL, position)?;
                }

                // parse directory
                if let Some(directory) = package.get(DIRECTORY) {
                    p.directory = Some(
                        directory
                            .as_str()
                            .ok_or_else(|| as_str_err(DIRECTORY, position))?
                            .into(),
                    );
                }

                installed.insert(p.to_id(), p);
            }
        }
    }
    Ok(installed)
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
                value(&dir.to_string_lossy().to_string()),
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
    use crate::config::repository::{ChannelType, Revision};

    use super::{InstalledList, InstalledPackage};

    #[test]
    fn add_package() {
        let package_1: InstalledPackage = InstalledPackage {
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
            path: "sdk:package1".to_string(),
            version: Revision::new(1),
            channel: ChannelType::Stable,
            url: "gitlab.com".to_string(),
            directory: None,
        };
        let package_2: InstalledPackage = InstalledPackage {
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
[[package]]
path = "extras;google;auto"
version = "2.0.0.0"
channel = "stable"
url = "http://example.com"
"#;

        let result: InstalledList = toml.parse().unwrap();
        let mut iter = result.packages.iter();
        let value: &InstalledPackage = iter.next().unwrap();

        let package = InstalledPackage {
            path: "extras;google;auto".to_string(),
            version: "2.0.0.0".parse().unwrap(),
            channel: ChannelType::Stable,
            url: "http://example.com".to_string(),
            directory: None,
        };

        assert_eq!(value.to_id(), package.to_id());
        assert_eq!(value, &package);
    }
    #[test]
    fn installed_package_list_to_toml() {
        let mut list = InstalledList::new();

        let package = InstalledPackage {
            path: "extras;google;auto".to_string(),
            version: "2.0.0.0".parse().unwrap(),
            channel: ChannelType::Stable,
            url: "http://example.com".to_string(),
            directory: None,
        };

        list.add_installed_package(package.clone());

        let toml = r#"
[[package]]
path = "extras;google;auto"
version = "2.0.0.0"
channel = "stable"
url = "http://example.com"
"#;
        assert_eq!(list.to_string(), toml.trim_start());
    }
}
