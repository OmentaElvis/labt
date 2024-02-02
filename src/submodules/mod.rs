use anyhow::Result;

pub trait Submodule {
    fn run(&mut self) -> Result<()> {
        Ok(())
    }
}

pub mod add;
pub mod init;
