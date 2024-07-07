use std::collections::HashMap;
use std::fmt::Display;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context};
use toml_edit::{value, Document};

use crate::config::repository::{ChannelType, Revision};
use crate::submodules::sdk::{get_sdk_path, toml_strings};

const INSTALLED_LIST: &str = "installed.toml";
const INSTALLED_LIST_OPEN_ERR: &str = "Failed to open sdk installed.toml";
const PACKAGE: &str = "package";
pub const SDK_PATH_ERR_STRING: &str = "Failed to get android sdk path";

#[derive(Debug, Default, PartialEq, Eq, Hash)]
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

/// Reads installed.list file from sdkfolder. If the file does not exist it returns
/// an empty hashset
pub fn read_installed_list() -> anyhow::Result<HashMap<String, InstalledPackage>> {
    let mut sdk = get_sdk_path().context(SDK_PATH_ERR_STRING)?;
    sdk.push(INSTALLED_LIST);

    if !sdk.exists() {
        return Ok(HashMap::new());
    }

    let mut file = File::open(&sdk).context(INSTALLED_LIST_OPEN_ERR)?;
    let mut data = String::new();
    file.read_to_string(&mut data)
        .context(format!("Failed to read {}", sdk.to_string_lossy()))?;

    parse_installed_list(&data)
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

#[test]
fn installed_package_toml_from_str() {
    let toml = r#"
[[package]]
path = "extras;google;auto"
version = "2.0.0.0"
channel = "stable"
url = "http://example.com"
"#;

    let result = parse_installed_list(toml).unwrap();
    let mut iter = result.iter();

    let (key, value) = iter.next().unwrap();

    let package = InstalledPackage {
        path: "extras;google;auto".to_string(),
        version: "2.0.0.0".parse().unwrap(),
        channel: ChannelType::Stable,
        url: "http://example.com".to_string(),
        directory: None,
    };

    assert_eq!(key, &package.to_id());
    assert_eq!(value, &package);
}

#[test]
fn installed_package_list_to_toml() {
    let packages = vec![InstalledPackage {
        path: "extras;google;auto".to_string(),
        version: "2.0.0.0".parse().unwrap(),
        channel: ChannelType::Stable,
        url: "http://example.com".to_string(),
        directory: None,
    }];

    let mut data = Vec::new();
    write_installed_list(packages, &mut data).unwrap();

    let toml = r#"
[[package]]
path = "extras;google;auto"
version = "2.0.0.0"
channel = "stable"
url = "http://example.com"
"#;
    assert_eq!(&String::from_utf8(data).unwrap(), toml.trim_start());
}
