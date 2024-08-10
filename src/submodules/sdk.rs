use core::panic;
use std::{
    collections::HashMap,
    env,
    fs::{self, create_dir, create_dir_all, remove_dir_all, remove_file, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process,
    sync::Arc,
};

use anyhow::{anyhow, bail, Context};
use clap::{Args, Subcommand};
use console::style;
use crossterm::style::Stylize;
use futures_util::StreamExt;
use indicatif::{HumanBytes, ProgressBar, ProgressStyle};
use log::{info, warn};
use reqwest::Url;
use sha1::{Digest, Sha1};
use toml_edit::{value, Document};
use zip::ZipArchive;

use crate::{
    config::repository::{
        parse_repository_xml, Archive, BitSizeType, ChannelType, RemotePackage, RepositoryXml,
        Revision,
    },
    get_home,
    submodules::sdkmanager::ToId,
    tui::{
        self,
        sdkmanager::{PendingAccepts, PendingAction, PendingActions, SdkManager},
        Tui,
    },
    MULTI_PROGRESS_BAR, USER_AGENT,
};

// consts
const DEFAULT_URL: &str = "https://dl.google.com/android/repository/";
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
    /// The base url to download this package from. The target archive file path is appended to the end of this.
    #[arg(long)]
    url: Option<Url>,
    /// The host platform to select. Format: <Os[;bit]> e.g. linux;64. Defaults to native os. This flag is used during install to select correct packages to download.
    #[arg(long)]
    host_os: Option<String>,
    /// Disables progressbars and trace logs
    #[arg(long, action)]
    quiet: bool,
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
    /// The base url to download this package from. The target archive file path is appended to the end of this.
    #[arg(long)]
    url: Option<Url>,
    /// Disables progressbars and trace logs
    #[arg(long, action)]
    quiet: bool,
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
    /// Polls for the lock file at intervals until it is released.
    /// Does not return util lock is available
    pub fn obtain_wait(path: &Path, pid: u32) -> io::Result<Self> {
        create_dir_all(path)?;

        let lock_file = path.join(LOCK_FILE);
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
    pub fn start_tui<'a>(
        packages: &'a mut FilteredPackages<'a, 'a>,
    ) -> io::Result<(PendingActions, PendingAccepts)> {
        let mut terminal: Tui = tui::init()?;
        terminal.clear()?;
        let (actions, accepts) = SdkManager::new(packages).run(&mut terminal)?;
        tui::restore()?;
        for (key, action) in actions.iter() {
            match action {
                tui::sdkmanager::PendingAction::Install => println!(
                    "{} {} {} [v{}]",
                    "+".green(),
                    key.get_display_name().clone().green(),
                    key.get_path(),
                    key.get_revision()
                ),
                tui::sdkmanager::PendingAction::Uninstall => println!(
                    "{} {} {} [v{}]",
                    "-".red(),
                    key.get_display_name().clone().red(),
                    key.get_path(),
                    key.get_revision()
                ),
                _ => {}
            }
        }
        for license in &accepts {
            println!("Accepted license: {}", license);
        }

        Ok((actions, accepts))
    }
    /// Lists the available and installed packages
    pub fn list_packages(
        &self,
        args: &ListArgs,
        repo: &RepositoryXml,
        installed: &mut InstalledList,
    ) -> anyhow::Result<()> {
        // let installed = Rc::new(installed);

        let mut filtered = FilteredPackages::new(repo, installed);
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

        if args.no_interactive {
            let pipe = style("|").dim();
            for package in filtered.get_packages() {
                println!(
                    "{}{pipe}{}{pipe}{}",
                    style(package.get_path()).blue(),
                    package.get_revision(),
                    package.get_display_name(),
                );
            }
            return Ok(());
        }

        let (actions, accepts) = Self::start_tui(&mut filtered)?;
        for license in accepts {
            installed.accept_license(license);
        }
        installed
            .save_to_file()
            .context("Failed to update accepted licenses to installed list config.")?;

        if actions.is_empty() {
            // nothing to do
            return Ok(());
        }

        let url = if let Some(url) = &args.url {
            url.clone()
        } else {
            Url::parse(DEFAULT_URL)?
        };
        // self contain errors comming from installers
        if let Err(err) =
            self.perform_actions(actions, repo, installed, url, &args.host_os, args.quiet)
        {
            log::error!(target: SDKMANAGER_TARGET, "{:?}", err);
        }
        Ok(())
    }
    /// performs all the pending actions
    pub fn perform_actions(
        &self,
        mut actions: HashMap<RemotePackage, PendingAction>,
        _repo: &RepositoryXml,
        installed_list: &mut InstalledList,
        url: Url,
        host_os: &Option<String>,
        quiet: bool,
    ) -> anyhow::Result<()> {
        let mut uninstaller = Uninstaller::new(quiet);
        let (host_os, bits) = self.get_host_os_and_bits(host_os.to_owned())?;
        let mut installer = Installer::new(url, bits, host_os, quiet);

        for (package, action) in actions.drain() {
            match action {
                PendingAction::Install => installer.add_package(package)?,
                PendingAction::Uninstall
                | PendingAction::Upgrade(_)
                | PendingAction::Downgrade(_)
                | PendingAction::Channel(_) => {
                    if let Some(p) = installed_list.contains_id(&InstalledPackage::new(
                        package.get_path().to_owned(),
                        package.get_revision().to_owned(),
                        package.get_channel().to_owned(),
                    )) {
                        uninstaller.add_uninstall_package(p.to_owned());
                    }
                }
                _ => {}
            }
        }
        // do uninstalls first before installs to have clean slate
        let removed_packages = uninstaller
            .uninstall()
            .context("Failed to uninstall packages")?;
        for package in removed_packages {
            let dir = &package.directory.clone().unwrap_or(PathBuf::default());
            info!(target: SDKMANAGER_TARGET, "Removed package {} at ({:?})", package.path, dir);
            installed_list.remove_installed_package(&package);
        }
        installed_list.save_to_file()?; // save after uninstall since next install process may fail leaving phantom packages

        installer.install()?;
        if !installer.install_targets.is_empty() {
            log::info!(target: SDKMANAGER_TARGET, "Installed [{} of {}] packages", installer.complete_tasks.len(), installer.install_targets.len());
        }
        for complete in installer.complete_tasks {
            installed_list.add_installed_package(complete);
        }
        installed_list.save_to_file()?;

        Ok(())
    }
    /// Returns the appropriate os and pointer width size (64 or 32bit)
    /// If os is None it returns the defaults of the current host os running labt
    fn get_host_os_and_bits(&self, os: Option<String>) -> anyhow::Result<(String, BitSizeType)> {
        // if you are debugging the sdkmanager, please check this section as it may be a source of bugs
        let mut bits = if cfg!(target_pointer_width = "64") {
            BitSizeType::Bit64
        } else {
            BitSizeType::Bit32
        };
        // I think android repo only supports linux, macos, windows
        let host_os = if let Some(host) = os {
            if let Some((os, bit)) = host.split_once(';') {
                bits = bit
                    .parse()
                    .context("Invalid platform bit width. Supported are 32bit and 64bit")?;
                os.to_string()
            } else {
                host
            }
        } else {
            match env::consts::FAMILY {
                "unix" if env::consts::OS.eq("macos") => "macos",
                "unix" => "linux",
                _ => "windows",
            }
            .to_string()
        };

        Ok((host_os, bits))
    }
    /// Tries to install the package provided
    pub fn install_package(
        &self,
        args: &InstallArgs,
        repo: RepositoryXml,
        installed: InstalledList,
    ) -> anyhow::Result<InstalledPackage> {
        let mut installed = installed;

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

            if let Some(channel) = &args.channel {
                if channel != p.get_channel() {
                    return false;
                }
            }

            true
        });

        let package = if let Some(p) = package {
            info!(target: SDKMANAGER_TARGET, "Found sdk package: {}, {} v{}-{}",p.get_display_name(), p.get_path(), p.get_revision(), p.get_channel());

            if p.is_obsolete() {
                // is obsolete
                warn!(target: SDKMANAGER_TARGET, "Package {} is obsolete", p.get_display_name());
            }
            p
        } else {
            let err = if let Some(channel) = &args.channel {
                format!(
                    "Package {} v{}-{} not found",
                    args.path, args.version, channel
                )
            } else {
                format!("Package {} v{} not found", args.path, args.version)
            };
            warn!(target: SDKMANAGER_TARGET, "{}", err);
            return Err(anyhow!(io::Error::new(io::ErrorKind::NotFound, err)));
        };
        let (host_os, bits) = self.get_host_os_and_bits(args.host_os.clone())?;

        let url = if let Some(url) = &args.url {
            url.to_owned()
        } else {
            Url::parse(DEFAULT_URL).context("Failed to parse default URL")?
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

        let result = install_package(package, host_os, bits, &target, &url);

        if let Ok(package) = &result {
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

        let mut list =
            InstalledList::parse_from_sdk().context("Failed reading installed packages list")?;

        match &self.args.subcommands {
            Some(SdkSubcommands::Install(args)) => {
                self.install_package(args, repo, list)
                    .context("Failed to install package")?;
            }
            Some(SdkSubcommands::List(args)) => {
                self.list_packages(args, &repo, &mut list)
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
        table.insert(CHANNEL, value(package.get_channel().to_string()));
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
                    package.set_channel(channel.as_str().unwrap().into())
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

    Ok(repo)
}

/// Starts the installation process
pub fn install_package(
    package: &RemotePackage,
    host_os: String,
    bits: BitSizeType,
    target_path: &Path,
    download_url: &Url,
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

    let url = download_url.join(archive.get_url())?;

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
    let prog = indicatif::ProgressBar::new(archive.len() as u64)
        .with_style(
            ProgressStyle::with_template(
                "{spinner} {msg} [{percent}%] {bar:40} {pos}/{len} {duration}",
            )
            .unwrap(),
        )
        .with_message("Extracting");
    extract_with_progress(&mut archive, target_path, &prog).context(format!(
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
        channel: package.get_channel().to_owned(),
    })
}

pub fn extract_with_progress<P: AsRef<Path>>(
    archive: &mut ZipArchive<File>,
    directory: P,
    prog: &indicatif::ProgressBar,
) -> anyhow::Result<()> {
    prog.set_length(archive.len() as u64);
    prog.reset();
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
            make_writable_dir_all(&p)
                .context(format!("Failed to make output path ({:?}) writable", p))?;
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

/// Obtains a lock on the target path and deletes the package path
struct Uninstaller {
    packages: Vec<InstalledPackage>,
    quiet: bool,
}

impl Uninstaller {
    pub fn new(quiet: bool) -> Self {
        Self {
            packages: Vec::new(),
            quiet,
        }
    }

    pub fn add_uninstall_package(&mut self, package: InstalledPackage) {
        self.packages.push(package);
    }
    /// Scans to check if a package exists in the sdk folder and if its removal
    /// left an empty parent package path.
    fn cleanup_sdk_dir(package: &mut InstalledPackage, mut dir: PathBuf) -> anyhow::Result<()> {
        // pop the first entry as it was removed before this function call
        if !dir.pop() {
            return Ok(());
        }
        let sdk = get_sdk_path().context(super::sdkmanager::installed_list::SDK_PATH_ERR_STRING)?;
        if !dir.starts_with(sdk) {
            // aint touching that, not ours
            return Ok(());
        }
        // skip the first as it was deleted by previous remove
        let segments = package.path.split(';').rev().skip(1);
        for segment in segments {
            if let Some(p) = dir.file_name() {
                if segment.eq(p) {
                    // short circuit if path is not empty
                    if dir.is_dir() {
                        let entries = fs::read_dir(&dir)
                            .context(format!("Failed to read directory contents of ({:?}).", dir))?
                            .count();
                        if entries > 0 {
                            break;
                        }
                        #[cfg(test)]
                        {
                            info!("Removing {:?}", dir);
                        }
                        #[cfg(not(test))]
                        {
                            fs::remove_dir(&dir)
                                .context(format!("Failed to remove directory ({:?})", dir))?;
                        }
                        dir.pop();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(())
    }
    fn remove_package(package: &mut InstalledPackage, quiet: bool) -> anyhow::Result<()> {
        // check for lock file on target dir
        let dir = if let Some(dir) = &package.directory {
            dir.clone()
        } else {
            let path: PathBuf = package.path.split(';').collect();
            let sdk = get_sdk_path()?;
            sdk.join(path)
        };
        let lock = dir.join(LOCK_FILE);
        if lock.exists() {
            let pid = process::id();
            let other_pid = fs::read_to_string(&lock)?;
            if !pid.to_string().eq(&other_pid) {
                bail!("Unable to obtain lock at {}. This may be caused by a previous installation attempt that crashed or terminated unexpectedly, or another LABt process is currently operating on the directory and is locking it to prevent corruption. Try removing the lock file or waiting for the other process ({:?}) to finish.", lock.to_string_lossy(), pid);
            }
        }
        let prog = if !quiet {
            let prog = MULTI_PROGRESS_BAR.with(|m| m.borrow().add(ProgressBar::new_spinner()));
            prog.set_message(format!("Removing {} at ({:?}).", package.path, dir));
            Some(prog)
        } else {
            None
        };
        remove_dir_all(&dir).context(format!(
            "Failed to clear package directory at ({:?}). An error occurred while removing all contents from this directory.",
            dir
        ))?;
        package.directory = Some(dir.clone());
        Self::cleanup_sdk_dir(package, dir).context(format!(
            "Failed to cleanup sdk directory for package {}",
            package.path
        ))?;
        if let Some(prog) = prog {
            prog.finish_and_clear();
        }
        Ok(())
    }
    /// Loops through all packages marked for uninstall removing them from disk and install list
    pub fn uninstall(mut self) -> anyhow::Result<Vec<InstalledPackage>> {
        for package in &mut self.packages {
            Self::remove_package(package, self.quiet)?;
        }
        Ok(self.packages)
    }
}

// /// How the installer should behave
// enum InstallerMode {
//     /// Turn on a tokio runtime to install everything concurrently. May avoid
//     /// using tokio if it is just a single package.
//     Parallel,
//     /// Install the packages sequentially
//     Sequential,
//     /// Changes install mode according to current state
//     Default,
// }

/// Manages the installation on packages
struct Installer {
    /// The installer mode
    // pub mode: InstallerMode,
    install_targets: Vec<InstallerTarget>,
    complete_tasks: Vec<InstalledPackage>,

    default_url: Arc<Url>,
    /// The current os architecture bits, ie 64 or 32. This sets the preferred bits. If an archive is platform independent, it will be downloaded instead.
    bits: BitSizeType,
    /// Target os
    host_os: String,
    /// If to show progressbars
    quiet: bool,
}

#[derive(Clone)]
struct InstallerTarget {
    bits: BitSizeType,
    host_os: String,
    target_path: PathBuf,
    download_url: Arc<Url>,
    package: RemotePackage,
}
fn checksum_err(path: String, archive: &Archive, checksum: String) -> anyhow::Error {
    // error messages in reverse so anyhow can do its thing
    anyhow!("Common reasons for this error include network connectivity issues, file corruption, or malicious tampering.")
        .context(format!("Calculated checksum: {} ", checksum))
        .context(format!("Expected checksum: {}", archive.get_checksum()))
        .context(format!("Checksum mismatch: The downloaded file's SHA-1 checksum for {} does not match the expected value. Refusing to install.", path))
}

impl Installer {
    pub fn new(download_from: Url, bits: BitSizeType, host_os: String, quiet: bool) -> Self {
        Self {
            install_targets: Vec::new(),
            complete_tasks: Vec::new(),
            default_url: Arc::new(download_from),
            bits,
            host_os,
            quiet,
        }
    }

    pub fn add_target(&mut self, target: InstallerTarget) {
        self.install_targets.push(target);
    }

    pub fn add_package(&mut self, package: RemotePackage) -> anyhow::Result<()> {
        let path: PathBuf = package.get_path().split(';').collect();
        let sdk = get_sdk_path()?;
        let target = InstallerTarget {
            bits: self.bits,
            host_os: self.host_os.clone(),
            target_path: sdk.join(path),
            package,
            download_url: Arc::clone(&self.default_url),
        };

        self.add_target(target);

        Ok(())
    }

    fn select_archive<'a>(
        archives: &'a [Archive],
        host_os: &String,
        bits: &BitSizeType,
    ) -> anyhow::Result<&'a Archive> {
        let archives: Vec<&Archive> = archives
            .iter()
            .filter(|p| {
                if p.get_host_os().is_empty() {
                    // os not set so include this
                    true
                } else {
                    p.get_host_os().eq(host_os)
                }
            })
            .filter(|p| {
                let b = p.get_host_bits();
                if b == BitSizeType::Unset {
                    true
                } else {
                    b == *bits
                }
            })
            .collect();
        // select the first archive and install it
        if let Some(archive) = archives.first() {
            Ok(archive)
        } else {
            bail!(
                "Failed to get an appropriate archive to download for platform: {}, {} bit",
                host_os,
                bits
            );
        }
    }
    pub fn calculate_checksum(path: &Path, prog: Option<ProgressBar>) -> anyhow::Result<String> {
        let file = File::open(path).context(format!(
            "Failed to open file at ({:?}) to compute checksum.",
            path
        ))?;
        let mut reader = BufReader::new(file);
        let mut sha = Sha1::new();
        let mut buf = [0; 4 * 1024];

        if let Some(prog) = &prog {
            prog.reset();
            prog.set_message(format!("Calculating sha1 checksum for ({:?})", path));
        }

        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            sha.update(&buf[..n]);
            if let Some(prog) = &prog {
                prog.inc(n as u64);
            }
        }
        let digest = sha.finalize();
        Ok(format!("{:x}", digest))
    }
    fn download_package_blocking(
        &self,
        client: &reqwest::blocking::Client,
        target: &InstallerTarget,
    ) -> anyhow::Result<InstalledPackage> {
        // get the target archive to download
        let archive =
            Self::select_archive(target.package.get_archives(), &target.host_os, &target.bits)?;
        let archive_url = archive.get_url();
        let url =
                    // if archive url is a full url use it otherwise treat the url like a file name
                    if archive_url.starts_with("http://") || archive_url.starts_with("https://") {
                        Url::parse(archive_url).context("Invalid archive url encountered")?
                    } else {
                        target.download_url.join(archive_url).context(format!("Failed to join url {} with {}", target.download_url, archive_url))?
                    };
        let req = client.get(url.clone());
        let res = req
            .send()
            .context(format!(
                "Failed to complete request to {url} for {}",
                target.package.get_path()
            ))?
            .error_for_status()
            .context(format!(
                "Server responded with an error while trying to fetch {}",
                target.package.get_path()
            ))?;

        let prog = if !self.quiet {
            Some(indicatif::ProgressBar::new(archive.get_size() as u64).with_style(
                ProgressStyle::with_template(
                    "{spinner}[{percent}%] {bar:40} {binary_bytes_per_sec} {duration} {wide_msg}",
                )
                .unwrap(),
            ))
        } else {
            None
        };

        let target_path = &target.target_path;
        // create a lock file to protect directory
        let pid = process::id();
        // lock will be released if it goes out of scope
        let _lock = SdkLock::obtain(target_path, pid)?;
        let mut output = target_path.clone();
        output.push("package.tmp");

        let file = File::create(&output).context("Failed to create download tmp file")?;
        let mut writer = BufWriter::new(file);

        let mut reader = BufReader::new(res);

        if let Some(prog) = &prog {
            prog.set_message(format!("Downloading {}", target.package.get_path()));
            // progressbar is enabled, so possibly waste some extra cpu cycles accomodating for it
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
            writer.flush().context(format!(
                "An error occured while trying to flush remaining bytes to disk at ({:?}) at {}",
                &output,
                target.package.get_path()
            ))?;
            drop(writer);
            drop(reader);
        } else {
            // pipe input to output
            io::copy(&mut reader, &mut writer)?;
            writer.flush().context(format!(
                "An error occured while trying to flush remaining bytes to disk at ({:?}) at {}",
                &output,
                target.package.get_path()
            ))?;
            drop(writer);
            drop(reader);
        }
        // calculate checksum
        let checksum = Self::calculate_checksum(&output, prog.clone()).context(format!(
            "Failed to compute sha1 checksum for ({:?})",
            &output
        ))?;

        if !checksum.eq(archive.get_checksum()) {
            bail!(checksum_err(target.package.to_id(), archive, checksum));
        }

        // unzip
        let file = File::open(&output).context("Failed to open download tmp file")?;
        let mut archive = zip::ZipArchive::new(file)?;
        if let Some(prog) = &prog {
            prog.set_message(format!("Extracting {}", target.package.get_path()));
            extract_with_progress(&mut archive, target_path, prog).context(format!(
                "Failed to unzip package archive to ({:?})",
                target_path
            ))?;
        } else {
            archive.extract(target_path)?;
        }
        info!(target: SDKMANAGER_TARGET, "Extracted {} entries to ({:?}).", archive.len(), target_path);

        log::trace!(target: SDKMANAGER_TARGET, "Removing download temp file ({:?})", output);
        remove_file(&output).context(format!(
            "Failed to remove download temp file at ({:?})",
            output
        ))?;

        let package = &target.package;

        Ok(InstalledPackage {
            path: package.get_path().to_owned(),
            version: package.get_revision().to_owned(),
            url: String::new(),
            directory: Some(target_path.to_path_buf()),
            channel: package.get_channel().to_owned(),
        })
    }

    async fn download_package_async(
        client: reqwest::Client,
        target: InstallerTarget,
        prog: Option<ProgressBar>,
    ) -> anyhow::Result<InstalledPackage> {
        use tokio::io::AsyncWriteExt;
        let archive =
            Self::select_archive(target.package.get_archives(), &target.host_os, &target.bits)?;
        let archive_url = archive.get_url();
        let url =
                // if archive url is a full url use it otherwise treat the url like a file name
                if archive_url.starts_with("http://") || archive_url.starts_with("https://") {
                    Url::parse(archive_url).context("Invalid archive url encountered")?
                } else {
                    target.download_url.join(archive_url).context(format!("Failed to join url {} with {}", target.download_url, archive_url))?
                };
        let req = client.get(url.clone());
        let res = req
            .send()
            .await
            .context(format!(
                "Failed to complete request to {url} for {}",
                target.package.get_path()
            ))?
            .error_for_status()
            .context(format!(
                "Server responded with an error while trying to fetch {}",
                target.package.get_path()
            ))?;
        if let Some(prog) = &prog {
            prog.set_length(archive.get_size() as u64);
            prog.set_message(format!("Downloading {}", target.package.get_path()));
        }
        let target_path = &target.target_path;
        // create a lock file to protect directory
        let pid = process::id();
        // lock will be released if it goes out of scope
        let _lock = SdkLock::obtain(target_path, pid)?;

        let mut output = target_path.clone();
        output.push("package.tmp");

        let file = tokio::fs::File::create(&output)
            .await
            .context("Failed to create download tmp file")?;
        let mut writer = tokio::io::BufWriter::new(file);

        let mut stream = res.bytes_stream();
        while let Some(item) = stream.next().await {
            let bytes = item?;
            let written = writer.write(&bytes[0..]).await?;
            if written != bytes.len() {
                return Err(anyhow!("Failed to copy all bytes from the network stream to a local file: read {}, written: {}", bytes.len(), written));
            }

            if let Some(prog) = &prog {
                prog.inc(bytes.len() as u64);
            }
        }
        writer.flush().await.context(format!(
            "An error occured while trying to flush remaining bytes to disk at ({:?}) at {}",
            &output,
            target.package.get_path()
        ))?;
        drop(writer);
        let extract_path = target_path.clone();
        let package_path_name = target.package.get_path().to_owned();
        let package_path_id = target.package.to_id();
        let output_file = output.to_owned();
        let archive = archive.clone();

        // unzip
        tokio::task::spawn_blocking(move || {
            let prog = prog;
            // calculate checksum
            let checksum = Self::calculate_checksum(&output_file, prog.clone()).context(format!(
                "Failed to compute sha1 checksum for ({:?})",
                &output_file
            ))?;

            if !checksum.eq(archive.get_checksum()) {
                bail!(checksum_err(package_path_id, &archive, checksum));
            }

            // unzip file
            let file = File::open(&output_file).context("Failed to open download tmp file")?;
            let mut archive = zip::ZipArchive::new(file).context(format!(
                "Failed to open downloaded zip archive ({:?}) for {}",
                &output_file, package_path_name
            ))?;
            if let Some(prog) = &prog {
                prog.reset();
                prog.set_message(format!("Extracting {}", &package_path_name));
                extract_with_progress(&mut archive, &extract_path, prog).context(format!(
                    "Failed to unzip package archive to ({:?})",
                    extract_path
                ))?;
            } else {
                archive.extract(&extract_path).context(format!(
                    "Failed to open downloaded zip archive ({:?}) for {}",
                    &output_file, package_path_name
                ))?;
            }
            if let Some(prog) = &prog {
                prog.finish_and_clear();
            }
            info!(target: SDKMANAGER_TARGET, "Extracted {} entries to ({:?}).", archive.len(), extract_path);
            Ok::<_, anyhow::Error>(())
        }).await??;

        log::trace!(target: SDKMANAGER_TARGET, "Removing download temp file ({:?})", output);
        remove_file(&output).context(format!(
            "Failed to remove download temp file at ({:?})",
            output
        ))?;

        let package = &target.package;

        Ok(InstalledPackage {
            path: package.get_path().to_owned(),
            version: package.get_revision().to_owned(),
            url: url.to_string(),
            directory: Some(target_path.to_path_buf()),
            channel: package.get_channel().to_owned(),
        })
    }
    /// spawns a new tokio instance to do all the installs
    fn install_async(&mut self) -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("package_installer")
            .enable_all()
            .build()?;

        let client = reqwest::ClientBuilder::new()
            .user_agent(USER_AGENT)
            .build()?;
        let quiet = self.quiet;

        let results = runtime.block_on(async {
            let mut tasks = Vec::new();

            for target in &self.install_targets {
                let prog = if !quiet {
                    let prog = indicatif::ProgressBar::new(0).with_style(
                        ProgressStyle::with_template(
                            "{spinner}[{percent}%] {bar:40} {binary_bytes_per_sec} {duration} {wide_msg}",
                        )
                        .unwrap(),
                    ).with_message("Downloading");
                    Some(MULTI_PROGRESS_BAR.with(|multi| multi.borrow().add(prog)))
                } else {
                    None
                };
                tasks.push((target.package.to_id(), tokio::spawn(Self::download_package_async(
                    client.clone(),
                    target.clone(),
                    prog,
                ))));
            }
            let mut result: Vec<anyhow::Result<InstalledPackage>> = Vec::new();
            for (target, task) in tasks {
                result.push(task.await?.context(format!("Failed to install package: {}", target)));
            }

            Ok::<Vec<anyhow::Result<InstalledPackage>>, anyhow::Error>(result)
        })?;

        for result in results {
            match result {
                Ok(package) => self.complete_tasks.push(package),
                Err(err) => {
                    log::error!(target: SDKMANAGER_TARGET, "{:?}", err);
                }
            }
        }

        Ok(())
    }
    fn install_sync(&mut self) -> anyhow::Result<()> {
        let client = reqwest::blocking::ClientBuilder::new()
            .user_agent(USER_AGENT)
            .build()?;
        for target in &self.install_targets {
            let installed_package =
                self.download_package_blocking(&client, target)
                    .context(format!(
                        "Failed to install package: {}",
                        target.package.to_id()
                    ))?;
            self.complete_tasks.push(installed_package);
        }

        Ok(())
    }

    /// Starts the installation process
    pub fn install(&mut self) -> anyhow::Result<()> {
        if self.install_targets.len() > 1 {
            self.install_async()?;
        } else {
            self.install_sync()?;
        }

        Ok(())
    }
}
