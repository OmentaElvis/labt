use std::{
    error::Error as StdError,
    fmt::{write, Display},
};

pub mod fs;
pub mod labt;
pub mod log;
pub mod sys;
pub mod zip;

/// Wraps anyhow Error so as to allow useful anyhow error chain to be
/// passed back into the lua executer for tracing
#[derive(Debug)]
pub struct MluaAnyhowWrapper(anyhow::Error);

impl StdError for MluaAnyhowWrapper {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
    fn description(&self) -> &str {
        "An error ocurred"
    }
    fn cause(&self) -> Option<&dyn StdError> {
        None
    }
}

impl Display for MluaAnyhowWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write(f, format_args!("{:?}", self.0))
    }
}

impl From<anyhow::Error> for MluaAnyhowWrapper {
    fn from(value: anyhow::Error) -> Self {
        MluaAnyhowWrapper(value)
    }
}
impl From<MluaAnyhowWrapper> for mlua::Error {
    fn from(val: MluaAnyhowWrapper) -> Self {
        mlua::Error::external(val)
    }
}

impl MluaAnyhowWrapper {
    /// converts a anyhow error into mlua Error
    pub fn external(err: anyhow::Error) -> mlua::Error {
        mlua::Error::external(MluaAnyhowWrapper(err))
    }
}
