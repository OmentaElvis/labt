use std::io;

pub trait Submodule {
    fn run(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub mod add;
pub mod init;
