use std::cell::RefCell;

use clap::{Args, ValueEnum};

use super::Submodule;

thread_local! {
    pub static BUILD_STEP: RefCell<Step> = RefCell::new(Step::PRE);
}

#[derive(Clone, Args)]
pub struct BuildArgs {
    pub step: Option<Step>,
}

pub struct Build {
    pub args: BuildArgs,
}

#[derive(Clone, ValueEnum, Debug)]
pub enum Step {
    /// PRE compilation step which indicates that should
    /// run code generators, dependency injection, fetching dynamic
    /// modules etc.
    PRE,
    /// Android asset packaging
    AAPT,
    /// Application source files compilation, e.g. compile java, kotlin etc.
    COMPILE,
    /// Dexing application step to generate app classes.dex from jar files
    DEX,
    /// Bundles compiled resources into apk file and aligning, signing etc.
    BUNDLE,
    /// POST compilation step. Run, create a release file, return results to
    /// CI/CD pipeline etc.
    POST,
}

impl Build {
    pub fn new(args: &BuildArgs) -> Self {
        Self { args: args.clone() }
    }
}

impl Submodule for Build {
    fn run(&mut self) -> anyhow::Result<()> {
        if let Some(step) = self.args.step.clone() {
            // update build step if already provided
            BUILD_STEP.with(|s| {
                *s.borrow_mut() = step;
            });
        }

        BUILD_STEP.with(|step| {
            println!("{:#?}", *step.borrow());
        });

        Ok(())
    }
}
