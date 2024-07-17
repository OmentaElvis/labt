use core::panic;
use std::{
    env,
    fs::{self, create_dir, create_dir_all, remove_file, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process,
    rc::Rc,
};

use anyhow::{anyhow, bail, Context};
use clap::{Args, Subcommand};
use console::style;
use indicatif::{HumanBytes, ProgressStyle};
use log::{info, warn};
use reqwest::Url;
use toml_edit::{value, Document};
use zip::ZipArchive;

use crate::{
    config::repository::{
        parse_repository_xml, Archive, BitSizeType, ChannelType, RemotePackage, RepositoryXml,
        Revision,
    },
    get_home,
    tui::{self, sdkmanager::SdkManager, Tui},
};

// consts
const DEFAULT_URL: &str = "https://dl.google.com/android/repository/";
const TEST_URL: &str = "http://localhost:8080/";
const DEFAULT_RESOURCES_URL: &str = "https://dl.google.com/android/repository/repository2-1.xml";
const SDKMANAGER_TARGET: &str = "sdkmanager";
const LOCK_FILE: &str = ".lock";

use super::sdkmanager::filters::FilteredPackages;
use super::sdkmanager::installed_list::InstalledList;
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

    #[command(subcommand)]
    subcommands: Option<SdkSubcommands>,
}

#[derive(Subcommand, Clone)]
pub enum SdkSubcommands {
    /// Install a package
    Install(InstallArgs),
    /// List packages
    List(ListArgs),
}

#[derive(Clone, Args)]
pub struct ListArgs {
    /// Show only installed packages
    #[arg(long, action)]
    installed: bool,
    /// Include obsolete packages on package list
    #[arg(long, action)]
    show_obsolete: bool,
    /// Do not show interactive Terminal user interface
    #[arg(long, action)]
    no_interactive: bool,
    /// Filter by channel name e.g. stable, beta, dev, canary etc.
    #[arg(long)]
    channel: Option<ChannelType>,
}

#[derive(Clone, Args)]
pub struct InstallArgs {
    /// The package path name to install
    #[arg(long)]
    path: String,
    /// The package version to install
    #[arg(long)]
    version: Revision,
    /// The package channel to install
    #[arg(long)]
    channel: Option<ChannelType>,
    /// The display name. Use this only to further disambiguate packages with same path and version.
    #[arg(long)]
    display_name: Option<String>,
    /// Accept license if available
    #[arg(long, action)]
    accept: bool,
    #[arg(long)]
    /// The host platform to select. Format: <Os[;bit]> e.g. linux;64. Defaults to native os.
    host_os: Option<String>,
}

pub struct Sdk {
    url: String,
    update: bool,
    args: SdkArgs,
}

/// Locks the target directory so that other LABt processes do not interfere with it
/// The lock is released once it goes out of scope and dropped
/// Please note that this is a heavy drop since it involves reading and deleating of lock files
pub struct SdkLock {
    path: PathBuf,
    /// How should we handle release error behaviour
    release_err_behaviour: SdkLockReleaseErrorBehaviour,
    /// Current process id
    pid: u32,
}
#[derive(Default)]
pub enum SdkLockReleaseErrorBehaviour {
    /// Log and ignore
    Log,
    /// Panic
    Panic,
    /// Log and panic.
    #[default]
    LogPanic,
    /// Silently ignore lock release errors
    Ignore,
}

impl SdkLock {
    pub fn obtain(path: &Path, pid: u32) -> io::Result<Self> {
        create_dir_all(path)?;

        let lock_file = path.join(LOCK_FILE);

        if lock_file.exists() {
            let other_pid = fs::read_to_string(&lock_file)?;
            if !pid.to_string().eq(&other_pid) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unable to obtain lock at {}. This may be caused by a previous installation attempt that crashed or terminated unexpectedly, or another LABt process is currently operating on the directory and is locking it to prevent corruption. Try removing the lock file or waiting for the other process ({}) to finish.", lock_file.to_string_lossy(), pid)));
            }
        } else {
            fs::write(&lock_file, pid.to_string().as_bytes())?;
        }

        Ok(Self {
            path: lock_file,
            pid,
            release_err_behaviour: SdkLockReleaseErrorBehaviour::LogPanic,
        })
    }
    /// Looks for a lock file on target directory and tries to delete it if its process id matches the current process.
    /// Takes ownership to prevent double releases
    pub fn release(self) {
        // self.released = true;
        drop(self);
    }
    /// Meant to be called by drop();
    fn internal_release(&self) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }

        // /// Setting force to true will disregard if process id matches and deletes the lock file anyway.
        // if force {
        //     remove_file(&self.path)
        //         .context(format!("Failed to remove lock file at {:?}", self.path))?;
        //     return Ok(());
        // }

        let pid = fs::read_to_string(&self.path).context(format!(
            "Failed reading pid from lock file ({:?})",
            self.path
        ))?;

        if !self.pid.to_string().eq(&pid) {
            return Err(anyhow!("Mismatched PID on lock file. lock has {} and current PID is {}. This lock file at ({:?}) may not be owned by current process.", pid, self.pid, self.path));
        }

        remove_file(&self.path)
            .context(format!("Failed to remove lock file at {:?}", self.path))?;

        Ok(())
    }
}

impl Drop for SdkLock {
    fn drop(&mut self) {
        let result = self.internal_release();
        if let Err(err) = &result {
            match self.release_err_behaviour {
                SdkLockReleaseErrorBehaviour::Log => {
                    log::error!(target: SDKMANAGER_TARGET, "{}", err)
                }
                SdkLockReleaseErrorBehaviour::Panic => result.unwrap(),
                SdkLockReleaseErrorBehaviour::Ignore => {} //no op
                SdkLockReleaseErrorBehaviour::LogPanic => {
                    log::error!(target: SDKMANAGER_TARGET, "{}", err);
                    panic!("Failed to release lock! Please delete lock file manually.");
                }
            }
        }
    }
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
            args: args.clone(),
        }
    }
    pub fn start_tui(&self, packages: FilteredPackages) -> io::Result<()> {
        let mut terminal: Tui = tui::init()?;
        terminal.clear()?;
        SdkManager::new(packages).run(&mut terminal)?;
        tui::restore()?;

        Ok(())
    }
    /// Lists the available and installed packages
    pub fn list_packages(
        &self,
        args: &ListArgs,
        repo: RepositoryXml,
        installed: InstalledList,
    ) -> anyhow::Result<()> {
        let mut filtered = FilteredPackages::new(Rc::new(repo), installed);
        if args.installed {
            filtered.insert_singleton_filter(super::sdkmanager::filters::SdkFilters::Installed);
        }
        // if show obsolete is not set, add a default flag to filter all obsolete packages
        if !args.show_obsolete {
            filtered
                .insert_singleton_filter(super::sdkmanager::filters::SdkFilters::Obsolete(false));
        }
        filtered.set_channel(args.channel.clone());
        filtered.apply();
        if !args.no_interactive {
            self.start_tui(filtered)?;
        } else {
            // println!(
            //     "{}|{}|{}",
            //     style("Path").underlined(),
            //     style("Version").underlined(),
            //     style("Description").underlined()
            // );
            let pipe = style("|").dim();
            for package in filtered.get_packages() {
                println!(
                    "{}{pipe}{}{pipe}{}",
                    style(package.get_path()).blue(),
                    package.get_revision(),
                    package.get_display_name(),
                );
            }
        }
        Ok(())
    }
    /// Tries to install the package provided
    pub fn install_package(
        &self,
        args: &InstallArgs,
        repo: RepositoryXml,
        installed: InstalledList,
    ) -> anyhow::Result<InstalledPackage> {
        let mut installed = installed;
        let channel = if let Some(channel) = &args.channel {
            repo.get_channels().iter().find(|p| p.1 == channel)
        } else {
            None
        };

        let package = repo.get_remote_packages().iter().find(|p| {
            if !&args.path.eq(p.get_path()) {
                return false;
            }

            if !args.version.eq(p.get_revision()) {
                return false;
            }

            if let Some(name) = &args.display_name {
                if !name.eq(p.get_display_name()) {
                    return false;
                }
            }

            if let Some((ref_id, _)) = channel {
                if !ref_id.eq(p.get_channel_ref()) {
                    return false;
                }
            } else {
                //the channel was not found
                if args.channel.is_some() {
                    return false;
                }
            }

            true
        });

        let package = if let Some(p) = package {
            info!(target: SDKMANAGER_TARGET, "Found sdk package: {}, {} v{}-{}",p.get_display_name(), p.get_path(), p.get_revision(), repo.get_channels().get(p.get_channel_ref()).unwrap_or(&ChannelType::Unknown("unknown".to_string())));

            if p.is_obsolete() {
                // is obsolete
                warn!(target: SDKMANAGER_TARGET, "Package {} is obsolete", p.get_display_name());
            }
            p
        } else {
            let err = format!(
                "Package {} v{}-{} not found",
                args.path,
                args.version,
                channel.map_or("unknown".to_string(), |c| c.1.to_string())
            );
            warn!(target: SDKMANAGER_TARGET, "{}", err);
            return Err(anyhow!(io::Error::new(io::ErrorKind::NotFound, err)));
        };
        // if you are debugging the sdkmanager, please check this section as it may be a source of bugs
        let mut bits = if cfg!(target_pointer_width = "64") {
            BitSizeType::Bit64
        } else {
            BitSizeType::Bit32
        };
        // I think android repo only supports linux, macos, windows
        let host_os = if let Some(host) = &args.host_os {
            if let Some((os, bit)) = host.split_once(';') {
                bits = bit
                    .parse()
                    .context("Failed to parse bits from --host-os arg")?;
                os.to_string()
            } else {
                host.clone()
            }
        } else {
            match env::consts::FAMILY {
                "unix" if env::consts::OS.eq("macos") => "macos",
                "unix" => "linux",
                _ => "windows",
            }
            .to_string()
        };

        // obtain the installation directory

        let path = package.get_path();
        let dir: PathBuf = path.split(';').collect();
        let sdk = get_sdk_path().context(super::sdkmanager::installed_list::SDK_PATH_ERR_STRING)?;
        let target = sdk.join(dir);

        // create a lock file to protect directory
        let pid = process::id();
        // lock will be released if it goes out of scope
        let _lock = SdkLock::obtain(&target, pid)?;
        // self.create_lock_file(&target, &pid)?;

        let result = install_package(package, host_os, bits, &target);

        if let Ok(package) = &result {
            let mut package = package.to_owned();
            if let ChannelType::Ref(reference) = &package.channel {
                package.channel = repo
                    .get_channels()
                    .get(reference)
                    .unwrap_or(&ChannelType::Unset)
                    .clone();
            }

            installed.insert_installed_package(package.to_owned());
            installed.save_to_file()?;
        }

        result
    }
    pub fn get_url(&self) -> &String {
        &self.url
    }
}

pub mod toml_strings {
    pub const PATH: &str = "path";
    pub const VERSION: &str = "version";
    pub const DISPLAY_NAME: &str = "display_name";
    pub const LICENSE: &str = "license";
    pub const CHANNEL: &str = "channel";
    pub const CHANNELS: &str = "channels";
    pub const URL: &str = "url";
    pub const CHECKSUM: &str = "checksum";
    pub const SIZE: &str = "size";
    pub const OS: &str = "os";
    pub const BITS: &str = "bits";
    pub const ARCHIVE: &str = "archive";
    pub const OBSOLETE: &str = "obsolete";
    pub const REMOTE_PACKAGE: &str = "remote_package";
    pub const CONFIG_FILE: &str = "repository.toml";
    pub const DIRECTORY: &str = "directory";
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

        let list =
            InstalledList::parse_from_sdk().context("Failed reading installed packages list")?;

        match &self.args.subcommands {
            Some(SdkSubcommands::Install(args)) => {
                self.install_package(args, repo, list)
                    .context("Failed to install package")?;
            }
            Some(SdkSubcommands::List(args)) => {
                self.list_packages(args, repo, list)
                    .context("Failed to list packages")?;
            }
            None => {}
        }

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
    let sdk = get_sdk_path().context(super::sdkmanager::installed_list::SDK_PATH_ERR_STRING)?;

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
        if package.is_obsolete() {
            table.insert(OBSOLETE, value(true));
        }
        remotes.push(table);
    }
    doc[REMOTE_PACKAGE] = toml_edit::Item::ArrayOfTables(remotes);

    // write channels ref
    let mut channels = toml_edit::Table::new();
    for channel in repo.get_channels() {
        channels.insert(channel.0, value(channel.1.to_string()));
    }
    doc[CHANNELS] = toml_edit::Item::Table(channels);

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

                // parse obsolete
                if let Some(obsolete) = p.get(OBSOLETE) {
                    package.set_obsolete(obsolete.as_bool().unwrap());
                }
                repo.add_remote_package(package);
            }
        }
    }
    if toml.contains_table(CHANNELS) {
        if let Some(channels) = toml[CHANNELS].as_table() {
            for c in channels {
                let channel: ChannelType = c.1.as_str().unwrap().into();
                repo.add_channel(c.0, channel);
            }
        }
    }

    Ok(repo)
}

/// Starts the installation process
pub fn install_package(
    package: &RemotePackage,
    host_os: String,
    bits: BitSizeType,
    target_path: &Path,
) -> anyhow::Result<InstalledPackage> {
    const NO_TARGET_ERR: &str = "No target to install";
    // select the appropriate archive
    let archives = package.get_archives();
    if archives.is_empty() {
        bail!(NO_TARGET_ERR);
    }

    let archives: Vec<&Archive> = archives
        .iter()
        .filter(|p| {
            if p.get_host_os().is_empty() {
                // os not set so include this
                true
            } else {
                p.get_host_os().eq(&host_os)
            }
        })
        .filter(|p| {
            let b = p.get_host_bits();
            if b == BitSizeType::Unset {
                true
            } else {
                b == bits
            }
        })
        .collect();

    let archive = archives.first().context(NO_TARGET_ERR)?;
    info!(target: SDKMANAGER_TARGET, "Downloading {} from {} with size {}", archive.get_url(), DEFAULT_URL, HumanBytes(archive.get_size() as u64));

    let client = reqwest::blocking::ClientBuilder::new()
        .user_agent(crate::USER_AGENT)
        .build()?;

    let url = Url::parse(TEST_URL).context("Failed to parse download url.")?;
    let url = url.join(archive.get_url())?;

    let req = client.get(url.clone());
    let res = req.send().context("Failed to complete request")?;

    let mut output = target_path.to_path_buf();
    output.push("package.tmp");
    let file = File::create(&output).context("Failed to create download tmp file")?;
    let mut writer = BufWriter::new(file);

    let mut reader = BufReader::new(res);

    let prog = indicatif::ProgressBar::new(archive.get_size() as u64).with_style(
        ProgressStyle::with_template(
            "{spinner}[{percent}%] {bar:40} {binary_bytes_per_sec} {duration}",
        )
        .unwrap(),
    );
    const BUFFER_LENGTH: usize = 8 * 1024;
    let mut buf: [u8; BUFFER_LENGTH] = [0; BUFFER_LENGTH];
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }

        let written = writer.write(&buf[0..read])?;
        if written != read {
            return Err(anyhow!("Failed to copy all bytes from the network stream to a local file: read {}, written: {}", read, written));
        }

        prog.inc(read as u64);
    }

    prog.finish_and_clear();
    drop(writer);
    drop(reader);

    // unzip
    let file = File::open(&output).context("Failed to open download tmp file")?;
    let mut archive = zip::ZipArchive::new(file)?;
    extract_with_progress(&mut archive, target_path).context(format!(
        "Failed to unzip package archive to ({:?})",
        target_path
    ))?;
    info!(target: SDKMANAGER_TARGET, "Extracted {} entries to ({:?}).", archive.len(), target_path);

    log::trace!(target: SDKMANAGER_TARGET, "Removing download temp file ({:?})", output);
    remove_file(&output).context(format!(
        "Failed to remove download temp file at ({:?})",
        output
    ))?;

    Ok(InstalledPackage {
        path: package.get_path().to_owned(),
        version: package.get_revision().to_owned(),
        url: url.to_string(),
        directory: Some(target_path.to_path_buf()),
        channel: ChannelType::Ref(package.get_channel_ref().to_owned()),
    })
}

pub fn extract_with_progress<P: AsRef<Path>>(
    archive: &mut ZipArchive<File>,
    directory: P,
) -> anyhow::Result<()> {
    let prog = indicatif::ProgressBar::new(archive.len() as u64)
        .with_style(
            ProgressStyle::with_template(
                "{spinner} {msg} [{percent}%] {bar:40} {pos}/{len} {duration}",
            )
            .unwrap(),
        )
        .with_message("Extracting");

    let make_writable_dir_all = |outpath: &dyn AsRef<Path>| -> Result<(), zip::result::ZipError> {
        create_dir_all(outpath.as_ref())?;
        #[cfg(unix)]
        {
            // Dirs must be writable until all normal files are extracted
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                outpath.as_ref(),
                std::fs::Permissions::from_mode(
                    0o700 | std::fs::metadata(outpath.as_ref())?.permissions().mode(),
                ),
            )?;
        }
        Ok(())
    };

    // Patched from ZipArchive::extract function
    // The MIT License (MIT)
    // Copyright (C) 2014 Mathijs van de Nes
    use std::fs;
    #[cfg(unix)]
    let mut files_by_unix_mode = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        prog.inc(1);
        let filepath = file
            .enclosed_name()
            .ok_or(zip::result::ZipError::InvalidArchive("Invalid file path"))?;

        let outpath = directory.as_ref().join(filepath);

        if file.is_dir() {
            make_writable_dir_all(&outpath)?;
            continue;
        }
        let symlink_target = if file.is_symlink() && (cfg!(unix) || cfg!(windows)) {
            let mut target = Vec::with_capacity(file.size() as usize);
            file.read_exact(&mut target)?;
            Some(target)
        } else {
            None
        };
        drop(file);
        if let Some(p) = outpath.parent() {
            make_writable_dir_all(&p)?;
        }
        if let Some(target) = symlink_target {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                let target = std::ffi::OsString::from_vec(target);
                let target_path = directory.as_ref().join(target);
                std::os::unix::fs::symlink(target_path, outpath.as_path())?;
            }
            #[cfg(windows)]
            {
                let Ok(target) = String::from_utf8(target) else {
                    return Err(ZipError::InvalidArchive("Invalid UTF-8 as symlink target"));
                };
                let target = target.into_boxed_str();
                let target_is_dir_from_archive =
                    archive.shared.files.contains_key(&target) && is_dir(&target);
                let target_path = directory.as_ref().join(OsString::from(target.to_string()));
                let target_is_dir = if target_is_dir_from_archive {
                    true
                } else if let Ok(meta) = std::fs::metadata(&target_path) {
                    meta.is_dir()
                } else {
                    false
                };
                if target_is_dir {
                    std::os::windows::fs::symlink_dir(target_path, outpath.as_path())?;
                } else {
                    std::os::windows::fs::symlink_file(target_path, outpath.as_path())?;
                }
            }
            continue;
        }
        let mut file = archive.by_index(i)?;
        let mut outfile = fs::File::create(&outpath)?;
        io::copy(&mut file, &mut outfile)?;
        #[cfg(unix)]
        {
            // Check for real permissions, which we'll set in a second pass
            if let Some(mode) = file.unix_mode() {
                files_by_unix_mode.push((outpath.clone(), mode));
            }
        }
    }
    #[cfg(unix)]
    {
        use std::cmp::Reverse;
        use std::os::unix::fs::PermissionsExt;

        if files_by_unix_mode.len() > 1 {
            // Ensure we update children's permissions before making a parent unwritable
            files_by_unix_mode.sort_by_key(|(path, _)| Reverse(path.clone()));
        }
        for (path, mode) in files_by_unix_mode.into_iter() {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
        }
    }
    prog.finish_and_clear();
    Ok(())
}
