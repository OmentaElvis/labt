use cliargs::parse_args;
pub mod cliargs;
pub mod config;
pub mod pom;
pub mod submodules;
pub mod templating;

fn main() -> anyhow::Result<()> {
    parse_args();
    Ok(())
}
