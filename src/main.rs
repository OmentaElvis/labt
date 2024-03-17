use std::{
    cell::RefCell,
    fs::{create_dir, create_dir_all},
    io::Write,
    path::PathBuf,
};

use anyhow::bail;
use cliargs::parse_args;
use console::style;
use env_logger::Env;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;

use crate::envs::HOME;
pub mod caching;
pub mod cliargs;
pub mod config;
pub mod plugin;
pub mod pom;
pub mod submodules;
pub mod templating;

thread_local! {
    pub static MULTI_PRPGRESS_BAR: RefCell<MultiProgress> = RefCell::new(MultiProgress::new());
}

pub const LABT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod envs {
    pub const LABT_HOME: &str = "LABT_HOME";
    pub const HOME: &str = "HOME";
}

pub fn get_home() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var(envs::LABT_HOME) {
        return Ok(PathBuf::from(path));
    }

    if let Ok(path) = std::env::var(envs::HOME) {
        let mut path = PathBuf::from(path);
        path.push(".labt");
        if path.exists() {
            return Ok(path);
        } else {
            bail!(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                ".labt folder does not exist on $HOME",
            ));
        }
    }

    bail!("No apropriate Labt home directory detected!");
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
        if let Ok(home) = std::env::var(HOME) {
            println!("Initializing LABt configs on home directory.");
            // try creating on the home path
            let mut path = PathBuf::from(home);
            first_run(&mut path)?;
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
