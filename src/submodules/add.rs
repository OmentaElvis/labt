use crate::{config::add_dependency_to_config, pom::Project};

use super::{resolve::resolve, Submodule};
use anyhow::{Context, Result};
use clap::{Args, Command};
use regex::Regex;

#[derive(Clone, Args)]
pub struct AddArgs {
    ///dependency group id
    #[arg(short)]
    pub group_id: Option<String>,
    /// dependency name
    #[arg(short)]
    pub artifact_id: Option<String>,
    /// Version
    #[arg(short = 'V')]
    pub version: Option<String>,
    /// Dependency string in the form group_id:artifact_id:version
    /// e.g. com.example:project1:1.0.0
    pub dependency: Option<String>,
}

pub struct Add {
    pub args: AddArgs,
}

impl Add {
    pub fn new(args: &AddArgs) -> Add {
        Add { args: args.clone() }
    }
    fn parse_dependency(&mut self) -> clap::error::Result<(String, String, String)> {
        use clap::error::ContextKind;
        use clap::error::ContextValue;
        use clap::error::ErrorKind;
        use clap::Error;
        let cmd = AddArgs::augment_args(Command::new("add"));

        if let Some(dep) = &self.args.dependency {
            // if dependency positional argument was provided, try to parse it
            let re = Regex::new(r"^([\w\.]+):([\w-]+):([\w\.-]+)$")
                .context("Invalid regex")
                .unwrap();
            if let Some(group) = re.captures(dep) {
                let group_id = &group[1];
                let artifact_id = &group[2];
                let version = &group[3];

                return Ok((
                    group_id.to_string(),
                    artifact_id.to_string(),
                    version.to_string(),
                ));
            } else {
                let mut err = Error::new(ErrorKind::InvalidValue).with_cmd(&cmd);
                err.insert(ContextKind::InvalidArg, ContextValue::String("invalid dependency string format, allowed format is groupid:artifactid:version".to_string()));
                return Err(err);
            }
        }

        let artifact_id = self.args.artifact_id.clone().ok_or({
            let mut err = Error::new(ErrorKind::MissingRequiredArgument).with_cmd(&cmd);
            err.insert(ContextKind::Usage, ContextValue::String("-a".to_string()));
            err
        })?;
        let group_id = self.args.group_id.clone().ok_or({
            let mut err = Error::new(ErrorKind::MissingRequiredArgument).with_cmd(&cmd);
            err.insert(ContextKind::Usage, ContextValue::String("-g".to_string()));
            err
        })?;
        let version = self.args.version.clone().ok_or({
            let mut err = Error::new(ErrorKind::MissingRequiredArgument).with_cmd(&cmd);
            err.insert(ContextKind::Usage, ContextValue::String("-V".to_string()));
            err
        })?;

        Ok((group_id, artifact_id, version))
    }
}

impl Submodule for Add {
    fn run(&mut self) -> Result<()> {
        let res = self.parse_dependency();
        let (group_id, artifact_id, version) = match res {
            Ok(dep) => dep,
            Err(err) => {
                err.print()?;
                return Ok(());
            }
        };
        add_dependency_to_config(group_id.clone(), artifact_id.clone(), version.clone())?;
        let mut project = Project::new(group_id.as_str(), artifact_id.as_str(), version.as_str());
        resolve(&mut project)?;

        // println!("{:?}", project);

        Ok(())
    }
}
