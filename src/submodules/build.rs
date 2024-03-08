use std::cell::RefCell;

use anyhow::Context;
use clap::{Args, ValueEnum};

use crate::plugin::load_plugins;

use super::Submodule;

// temporary, will remove if a cleaner way of passing the current step
// to plugins is achieved
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

#[derive(Clone, Copy, ValueEnum, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
        // The order by which to run the plugin build step
        let order: Vec<Step> = if let Some(step) = self.args.step {
            // if the build step was added explicitly, then just run that one
            // particular step
            vec![step]
        } else {
            // TODO add a more intelligent filter to run only the
            // required steps instead of just running everything
            vec![
                Step::PRE,
                Step::AAPT,
                Step::COMPILE,
                Step::DEX,
                Step::BUNDLE,
                Step::POST,
            ]
        };

        let map = load_plugins()?;

        for step in order {
            // update build step if already provided
            BUILD_STEP.with(|s| {
                *s.borrow_mut() = step;
            });

            if let Some(plugins) = map.get(&step) {
                for plugin in plugins {
                    // loop through each plugin executing each
                    let exe = plugin.load().context(format!(
                        "Error loading plugin: {}:{} at build step {:?}",
                        plugin.name, plugin.version, plugin.step
                    ))?;

                    let chunk = exe.load().context(format!(
                        "Error loading lua code for {}:{} at build step {:?}",
                        plugin.name, plugin.version, plugin.step
                    ))?;

                    chunk.exec().context(format!(
                        "Failed to execute plugin code {:?} for plugin {}:{} at build step {:?}",
                        plugin.path, plugin.name, plugin.version, plugin.step
                    ))?;
                }
            }
        }

        Ok(())
    }
}
