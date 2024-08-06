use std::{
    cell::RefCell,
    env::current_dir,
    ffi::OsStr,
    fs::{create_dir, create_dir_all},
    io::Write,
    path::PathBuf,
    sync::OnceLock,
};

use anyhow::bail;
use cliargs::parse_args;
use console::style;
use env_logger::Env;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use log::warn;

use crate::envs::HOME;
use crate::envs::LOCALAPPDATA;
pub mod caching;
pub mod cliargs;
pub mod config;
pub mod plugin;
pub mod pom;
pub mod submodules;
pub mod templating;
pub mod tui;

thread_local! {
    pub static MULTI_PRPGRESS_BAR: RefCell<MultiProgress> = RefCell::new(MultiProgress::new());
}
/// Initialized by get_project_root. It caches the the
/// result of the function to prevent extra system calls
/// DO NOT use directly
static PROJECT_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Cached value if labt home. Initialized by get_home function
static LABT_HOME_PATH: OnceLock<PathBuf> = OnceLock::new();

pub const LABT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const USER_AGENT: &str = concat!("Labt/", env!("CARGO_PKG_VERSION"));
pub const TARGET: &str = env!("TARGET");

pub mod envs {
    pub const LABT_HOME: &str = "LABT_HOME";
    pub const HOME: &str = "HOME";
    pub const LOCALAPPDATA: &str = "LOCALAPPDATA";
}

/// Returns the location of Labt home, this is where Labt stores its
/// configurations files, plugins and cache. It first checks if LABT_HOME
/// was set. If not set, falls back to $HOME/.labt on linux, or %LOCALAPPDATA%/.labt on
/// windows.
///
/// # Errors
///
/// This function will return an error if no suitable path is found for labt home.
pub fn get_home() -> anyhow::Result<PathBuf> {
    get_home_ref().cloned()
}

/// Returns the location of Labt home, this is where Labt stores its
/// configurations files, plugins and cache. It first checks if LABT_HOME
/// was set. If not set, falls back to $HOME/.labt on linux, or %LOCALAPPDATA%/.labt on
/// windows.
///
/// # Errors
///
/// This function will return an error if no suitable path is found for labt home.
#[cfg(not(target_os = "windows"))]
pub fn get_home_ref() -> anyhow::Result<&'static PathBuf> {
    // check for cached static variable
    if let Some(path) = LABT_HOME_PATH.get() {
        return Ok(path);
    }

    if let Ok(path) = std::env::var(envs::LABT_HOME) {
        return Ok(LABT_HOME_PATH.get_or_init(|| PathBuf::from(path)));
    }

    if let Ok(path) = std::env::var(envs::HOME) {
        let mut path = PathBuf::from(path);
        path.push(".labt");
        if path.exists() {
            return Ok(LABT_HOME_PATH.get_or_init(|| path));
        } else {
            bail!(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                ".labt folder does not exist on $HOME",
            ));
        }
    }

    bail!("No apropriate Labt home directory detected!");
}

/// Returns the location of Labt home, this is where Labt stores its
/// configurations files, plugins and cache. It first checks if LABT_HOME
/// was set. If not set, falls back to $HOME/.labt on linux, or %LOCALAPPDATA%/.labt on
/// windows.
///
/// # Errors
///
/// This function will return an error if no suitable path is found for labt home.
#[cfg(target_os = "windows")]
pub fn get_home_ref() -> anyhow::Result<&'static PathBuf> {
    // check for cached static variable
    if let Some(path) = LABT_HOME_PATH.get() {
        return Ok(path);
    }

    if let Ok(path) = std::env::var(envs::LABT_HOME) {
        return Ok(LABT_HOME_PATH.get_or_init(|| PathBuf::from(path)));
    }

    if let Ok(path) = std::env::var(envs::LOCALAPPDATA) {
        let mut path = PathBuf::from(path);
        path.push(".labt");
        if path.exists() {
            return Ok(LABT_HOME_PATH.get_or_init(|| path));
        } else {
            bail!(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                ".labt folder does not exist on %LOCALAPPDATA%",
            ));
        }
    }

    bail!("No apropriate Labt home directory detected!");
}

/// Recursively searches for project root folder by checking if
/// Labt.toml exist from the current working directory going up
/// the directory tree
/// uses the current working directory as the start point
/// Returns an error if listing directory contents fails or Labt.toml
// is never found
pub fn get_project_root<'a>() -> std::io::Result<&'a PathBuf> {
    if let Some(path) = PROJECT_ROOT.get() {
        return Ok(path);
    }
    let cwd = current_dir()?;
    let path = get_project_root_recursive(cwd)?;
    Ok(PROJECT_ROOT.get_or_init(|| path))
}

/// Recursively searches for project root folder by checking if
/// Labt.toml exist from the current working directory going up
/// the directory tree
/// Returns an error if listing directory contents fails or Labt.toml
// is never found
fn get_project_root_recursive(current_dir: PathBuf) -> std::io::Result<PathBuf> {
    for entry in (current_dir.read_dir()?).flatten() {
        let file = entry.path();
        if file.is_file() && file.file_name() == Some(OsStr::new("Labt.toml")) {
            // found!
            return Ok(current_dir);
        }
    }
    // didn't find the file, go up the tree
    if let Some(path) = current_dir.parent() {
        get_project_root_recursive(PathBuf::from(path))
    } else {
        // no more upsies
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Failed to get project root",
        ))
    }
}

/// Should be executed on labt first run.
/// it is assumed to be a first run if labt home does not exist
fn first_run(path: &mut PathBuf) -> anyhow::Result<()> {
    path.push(".labt");
    // create .labt folder
    create_dir_all(&path)?;

    // create cache dir
    path.push("cache");
    create_dir(&path)?;

    // create plugins
    path.pop();
    path.push("plugins");
    create_dir(&path)?;

    Ok(())
}

fn main() -> anyhow::Result<()> {
    // create home dir
    if get_home().is_err() {
        if cfg!(windows) {
            // windows initialize at LOCALAPPDATA
            if let Ok(home) = std::env::var(LOCALAPPDATA) {
                println!("Initializing LABt configs on home directory at {}.", home);
                // try creating on the home path
                let mut path = PathBuf::from(home);
                first_run(&mut path)?;
            }
        } else if let Ok(home) = std::env::var(HOME) {
            println!("Initializing LABt configs on home directory.");
            // try creating on the home path
            let mut path = PathBuf::from(home);
            first_run(&mut path)?;
        } else {
            warn!(target: "labt", "Failed to initialize labt home, please set LABT_HOME environmental variable pointing to where you want LABt to store its files.");
        }
    };

    let logger = env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .format(|buf, record| {
            let level = match record.level() {
                log::Level::Error => style("ERROR").red().bold(),
                log::Level::Warn => style(" WARN").yellow().bold(),
                log::Level::Info => style(" INFO").green().bold(),
                log::Level::Debug => style("DEBUG").blue().bold(),
                log::Level::Trace => style("TRACE").blue().bold(),
            };

            writeln!(
                buf,
                "{}{} {}{} {}",
                style("[").dim(),
                level,
                style(record.target()).dim(),
                style("]").dim(),
                record.args()
            )
        })
        .build();

    let multi = MULTI_PRPGRESS_BAR.with(|multi| multi.borrow().clone());
    LogWrapper::new(multi.clone(), logger).try_init()?;

    parse_args();

    Ok(())
}
