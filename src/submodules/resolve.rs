use super::Submodule;
use anyhow::Result;
use clap::Args;

#[derive(Args, Clone)]
pub struct ResolveArgs {
    // TODO add arguments
}

pub struct Resolver {
    pub args: ResolveArgs,
}

impl Resolver {
    pub fn new(args: &ResolveArgs) -> Self {
        Resolver { args: args.clone() }
    }
}

impl Submodule for Resolver {
    fn run(&mut self) -> Result<()> {
        Ok(())
    }
}
