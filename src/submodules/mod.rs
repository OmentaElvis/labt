use anyhow::Result;

pub trait Submodule {
    fn run(&mut self) -> Result<()> {
        Ok(())
    }
}

pub mod add;
pub mod build;
pub mod init;
pub mod plugin;
pub mod resolve;
pub mod resolvers;
pub mod sdk;
pub mod sdkmanager;
