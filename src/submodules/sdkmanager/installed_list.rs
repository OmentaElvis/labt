use std::collections::HashSet;
use std::fmt::Display;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

use anyhow::Context;

use crate::config::repository::Revision;
use crate::submodules::sdk::get_sdk_path;

const INSTALLED_LIST: &str = "installed.list";
const INSTALLED_LIST_OPEN_ERR: &str = "Failed to open sdk installed.list";
pub const SDK_PATH_ERR_STRING: &str = "Failed to get android sdk path";

#[derive(PartialEq, Eq, Hash)]
pub struct InstalledPackage {
    pub path: String,
    pub version: Revision,
}
impl InstalledPackage {
    pub fn new(path: String, version: Revision) -> Self {
        Self { path, version }
    }
}

impl TryFrom<&str> for InstalledPackage {
    type Error = anyhow::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut iter = value.splitn(2, ',');
        let path = iter.next().context("Missing path entry")?;
        let version = iter.next().context("Missing version entry")?;
        let revision: Revision = version
            .parse()
            .context(format!("Failed to parse revision from string {}", version))?;

        Ok(InstalledPackage {
            path: path.to_string(),
            version: revision,
        })
    }
}
impl Display for InstalledPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{},{}", self.path, self.version)
    }
}

/// Reads installed.list file from sdkfolder. If the file does not exist it returns
/// an empty hashset
pub fn read_installed_list() -> anyhow::Result<HashSet<InstalledPackage>> {
    let mut sdk = get_sdk_path().context(SDK_PATH_ERR_STRING)?;
    sdk.push(INSTALLED_LIST);

    if !sdk.exists() {
        return Ok(HashSet::new());
    }

    let file = File::open(sdk).context(INSTALLED_LIST_OPEN_ERR)?;
    let mut reader = BufReader::new(file);
    let mut installed: HashSet<InstalledPackage> = HashSet::new();

    let mut line_number: usize = 0;
    // parse the lines
    loop {
        let mut line = String::new();
        let count = reader
            .read_line(&mut line)
            .context("Failed to read line from file")?;
        if count == 0 {
            break;
        }
        line_number = line_number.saturating_add(1);

        let package: InstalledPackage = line.as_str().try_into().context(format!(
            "Failed to parse installed package on line {}",
            line_number
        ))?;

        installed.insert(package);
    }

    Ok(installed)
}
/// Writes the provided hashset to a installed.list file in sdk folder
/// Order is not guaranteed as it is a hashmap
pub fn write_installed_list(list: HashSet<InstalledPackage>) -> anyhow::Result<()> {
    let mut sdk = get_sdk_path().context(SDK_PATH_ERR_STRING)?;
    sdk.push(INSTALLED_LIST);

    let file = File::create(sdk).context(INSTALLED_LIST_OPEN_ERR)?;
    let mut writer = BufWriter::new(file);

    for package in list {
        writer
            .write_all(package.to_string().as_bytes())
            .context(format!("Failed to write line to {INSTALLED_LIST}"))?;
    }

    Ok(())
}
