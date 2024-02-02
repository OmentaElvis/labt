use super::Submodule;
use anyhow::Result;
use clap::Args;

#[derive(Clone, Args)]
pub struct AddArgs {
    /// dependency name
    pub name: String,
    /// Version
    pub version: String,
}

pub struct Add {
    args: AddArgs,
}

impl Add {
    pub fn new(args: &AddArgs) -> Add {
        return Add { args: args.clone() };
    }
}

impl Submodule for Add {
    fn run(&mut self) -> Result<()> {
        Ok(())
    }
}
