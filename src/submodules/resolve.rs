use std::cell::RefCell;
use std::cmp::Ordering;
use std::env::current_dir;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::caching::save_dependencies;
use crate::config::get_config;
use crate::config::lock::load_lock_dependencies;
use crate::config::lock::strings::LOCK_FILE;
use crate::config::lock::write_lock;
use crate::pom::Scope;
use crate::pom::{self, Project};
use crate::MULTI_PRPGRESS_BAR;

use super::resolvers::CacheResolver;
use super::resolvers::NetResolver;
use super::resolvers::Resolver;
use super::resolvers::ResolverErrorKind;
use super::Submodule;
use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use clap::Args;
use futures_util::TryStreamExt;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use log::info;
use pom::parse_pom_async;
use reqwest::Client;
use serde::Serialize;
use tokio::io::BufReader;
use tokio_util::io::StreamReader;

#[derive(Args, Clone)]
pub struct ResolveArgs {
    // TODO add arguments
}

pub struct Resolve {
    pub args: ResolveArgs,
}

impl Resolve {
    pub fn new(args: &ResolveArgs) -> Self {
        Resolve { args: args.clone() }
    }
}

impl Submodule for Resolve {
    fn run(&mut self) -> Result<()> {
        // try reading toml file
        let config = get_config()?;
        if let Some(deps) = config.dependencies {
            let dependencies: Vec<Project> = deps
                .iter()
                .map(|(artifact_id, table)| {
                    Project::new(&table.group_id, artifact_id, &table.version)
                })
                .collect();

            resolve(dependencies)?;
        }
        Ok(())
    }
}
#[derive(Debug, Default, Serialize)]
pub struct ProjectDep {
    pub artifact_id: String,
    pub group_id: String,
    pub version: String,
    pub scope: Scope,
    pub dependencies: Vec<String>,
    pub url: String,
    pub packaging: String,
    pub cache_hit: bool,
}

impl PartialEq for ProjectDep {
    fn eq(&self, other: &Self) -> bool {
        if self.group_id != other.group_id {
            return false;
        }

        if self.artifact_id != other.artifact_id {
            return false;
        }

        if self.version != other.version {
            return false;
        }

        true
    }
}
impl PartialOrd for ProjectDep {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.group_id != other.group_id {
            return None;
        }
        if self.artifact_id != other.artifact_id {
            return None;
        }
        match version_compare::compare(&self.version, &other.version)
            .unwrap_or(version_compare::Cmp::Ne)
        {
            version_compare::Cmp::Lt => Some(Ordering::Less),
            version_compare::Cmp::Eq => Some(Ordering::Equal),
            version_compare::Cmp::Gt => Some(Ordering::Greater),
            _ => None,
        }
    }
}

impl From<&Project> for ProjectDep {
    fn from(project: &Project) -> Self {
        ProjectDep {
            artifact_id: project.get_artifact_id(),
            group_id: project.get_group_id(),
            version: project.get_version(),
            scope: project.get_scope(),
            packaging: project.get_packaging(),
            dependencies: project
                .get_dependencies()
                .iter()
                .map(|dep| {
                    format!(
                        "{}:{}:{}",
                        dep.get_group_id(),
                        dep.get_artifact_id(),
                        dep.get_version()
                    )
                })
                .collect(),
            ..Default::default()
        }
    }
}

pub struct ProjectWrapper {
    project: Project,
    resolvers: Rc<RefCell<Vec<Box<dyn Resolver>>>>,
    progress: Option<Rc<RefCell<ProgressBar>>>,
}

impl ProjectWrapper {
    pub fn new(project: Project, resolvers: Rc<RefCell<Vec<Box<dyn Resolver>>>>) -> Self {
        ProjectWrapper {
            project,
            resolvers,
            progress: None,
        }
    }
    pub fn set_progress_bar(&mut self, progress: Option<Rc<RefCell<ProgressBar>>>) {
        self.progress = progress;
    }
    #[allow(unused)]
    pub fn add_resolver(&mut self, resolver: Box<dyn Resolver>) {
        self.resolvers.borrow_mut().push(resolver);
    }
    fn fetch(&mut self) -> anyhow::Result<(String, bool)> {
        let mut found = false;
        let mut url = String::new();
        let mut cache_hit = false;

        for resolver in self.resolvers.borrow_mut().iter() {
            match resolver.fetch(&mut self.project) {
                Err(err) => match err.kind() {
                    ResolverErrorKind::NotFound => continue,
                    _ => {
                        return Err(anyhow!(err).context(format!(
                            "Error while trying to resolve dependency on {}",
                            resolver.get_name()
                        )));
                    }
                },
                Ok(base_url) => {
                    url = base_url;
                    found = true;
                    cache_hit = resolver.get_name() == *"cache";
                    break;
                }
            }
        }

        // we failed to fetch dependency across all configured resolvers
        if !found {
            bail!(
                "Dependency \"{}\" not found on all configured resolvers",
                self.project.qualified_name()
            );
        }

        Ok((url, cache_hit))
    }
}

pub trait BuildTree {
    fn build_tree(
        &mut self,
        resolved: &mut Vec<ProjectDep>,
        unresolved: &mut Vec<String>,
    ) -> anyhow::Result<()>;
    // fn fetch(&mut self) -> anyhow::Result<()>;
}

impl BuildTree for ProjectWrapper {
    fn build_tree(
        &mut self,
        resolved: &mut Vec<ProjectDep>,
        unresolved: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        // push this project to unresolved
        unresolved.push(self.project.qualified_name());
        if let Some(prog) = &self.progress {
            let prog = prog.borrow();
            prog.set_message(format!(" {} ", self.project.qualified_name()));
            prog.set_prefix("Fetching");
        }
        info!(target: "fetch", "{}:{}:{} scope {:?}",
            self.project.get_group_id(),
            self.project.get_artifact_id(),
            self.project.get_version(),
            self.project.get_scope(),
        );
        // before we even proceed to do this "expensive" fetch just confirm this isn't a
        // potential version conflict and return instead
        if let Some((index, res)) = resolved.iter_mut().enumerate().find(|(_, res)| {
            res.group_id == self.project.get_group_id()
                && res.artifact_id == self.project.get_artifact_id()
        }) {
            // now check version for possible conflicts
            match version_compare::compare(&res.version, self.project.get_version()) {
                Ok(v) => match v {
                    version_compare::Cmp::Eq => {
                        // the versions are same, so skip resolving
                        return Ok(());
                    }
                    version_compare::Cmp::Ne => {
                        // TODO not really sure of what to do with this
                    }
                    version_compare::Cmp::Gt | version_compare::Cmp::Ge => {
                        // dependency conflict, so use the latest version which happens to be already resolved
                        return Ok(());
                    }
                    version_compare::Cmp::Lt | version_compare::Cmp::Le => {
                        // dependency version conflict, so replace the already resolved version with the latesr
                        // version and proceed to resolve for this version
                        resolved[index].version = self.project.get_version();
                    }
                },
                Err(_) => {
                    return Err(anyhow!(format!(
                        "Invalid versions string. Either {} or {} is invalid",
                        res.version,
                        self.project.get_version()
                    )));
                }
            }
        }
        // fetch the dependencies of this project
        let (url, cache_hit) = self.fetch().context(format!(
            "Error fetching {} scope {:?}",
            self.project.qualified_name(),
            self.project.get_scope(),
        ))?;

        let excludes = Rc::new(self.project.get_excludes().clone());
        self.project.get_dependencies_mut().retain(|dep| {
            if dep.get_scope().ne(&pom::Scope::COMPILE) {
                return false;
            }

            // filter all dependencies that match an exclude
            // Return true - to include a dependency
            //        false - to exclude a dependency
            let dep_group_id = dep.get_group_id();
            let dep_artifact_id = dep.get_artifact_id();

            for exclude in Rc::clone(&excludes).iter() {
                // exclude all transitive dependencies
                // Exclude: *:artifact or *:*
                if exclude.group_id == "*" {
                    return false; // exclude
                }

                // this dependency doesn't match the group id, so good to go
                // Exclude: com.example:* or com.example:artifact
                // Dep: org.example:artifact or something
                if dep_group_id != exclude.group_id {
                    continue; // maybe something will match later
                }

                // exclude all artifacts from this group
                // Exclude: com.example:*
                // Dep: com.example:artifact1 or com.example:artifact2
                if dep_group_id == exclude.group_id && exclude.artifact_id == "*" {
                    return false; // exclude
                }

                // implicit exclusion
                // Exclude: com.example:artifact
                // Dep: com.example:artifact
                if dep_group_id == exclude.group_id && dep_artifact_id == exclude.artifact_id {
                    return false;
                }
            }
            true // this particular guy survived, such a waster of clock cycles, good for it
        });

        for dep in self.project.get_dependencies() {
            // TODO remove this since it is redundant, but for some reason it breaks everything
            if let Some((index, res)) = resolved.iter_mut().enumerate().find(|(_, res)| {
                res.group_id == dep.get_group_id() && res.artifact_id == dep.get_artifact_id()
            }) {
                // now check version for possible conflicts
                match version_compare::compare(&res.version, dep.get_version()) {
                    Ok(v) => match v {
                        version_compare::Cmp::Eq => {
                            // the versions are same, so skip resolving
                            continue;
                        }
                        version_compare::Cmp::Ne => {
                            // TODO not really sure of what to do with this
                        }
                        version_compare::Cmp::Gt | version_compare::Cmp::Ge => {
                            // dependency conflict, so use the latest version which happens to be already resolved
                            continue;
                        }
                        version_compare::Cmp::Lt | version_compare::Cmp::Le => {
                            // dependency version conflict, so replace the already resolved version with the latesr
                            // version and proceed to resolve for this version
                            resolved[index].version = dep.get_version();
                        }
                    },
                    Err(_) => {
                        return Err(anyhow!(format!(
                            "Invalid versions string. Either {} or {} is invalid",
                            res.version,
                            dep.get_version()
                        )));
                    }
                }
            }

            if unresolved.contains(&dep.qualified_name()) {
                // Circular dep, if encountered,
                // TODO check config for ignore, warn, or Error
                return Ok(());
            }
            let mut wrapper = ProjectWrapper::new(dep.clone(), self.resolvers.clone());
            if let Some(progress) = &self.progress {
                wrapper.set_progress_bar(Some(progress.clone()));
            }
            wrapper.build_tree(resolved, unresolved)?;
        }

        // remove this project from unresolved
        unresolved.pop();

        // add this project to list of resolved
        let mut project = ProjectDep::from(&self.project);
        project.url = url;
        project.cache_hit = cache_hit;
        resolved.push(project);
        Ok(())
    }
}

#[allow(unused)]
async fn fetch_async(project: Project) -> anyhow::Result<Project> {
    let client = Client::builder().user_agent("Labt/1.1").build()?;
    let maven_url = format!(
        "https://repo1.maven.org/maven2/{0}/{1}/{2}/{1}-{2}.pom",
        project.get_group_id().replace('.', "/"),
        project.get_artifact_id(),
        project.get_version(),
    );
    let _google_url = format!(
        "https://maven.google.com/{0}/{1}/{2}/{1}-{2}.pom",
        project.get_group_id().replace('.', "/"),
        project.get_artifact_id(),
        project.get_version(),
    );

    let response = client.get(maven_url).send().await?;

    if response.status().is_success() {
        let stream = response
            .bytes_stream()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err));
        let reader = BufReader::new(StreamReader::new(stream));
        let p = parse_pom_async(reader, project).await?;
        Ok(p)
    } else {
        Err(anyhow!(format!(
            "{} Failed to resolve: {}:{}:{}",
            response.status(),
            project.get_artifact_id(),
            project.get_group_id(),
            project.get_version()
        )))
    }
}

/// Starts the resolution algorithm. Reads any existing Labt.lock and it includes
/// its resolution in the algorithm. After complete resolution it writes the result to
/// Labt.lock
///
/// # Panics
/// if we fail to initialize template for spinner progress bar, should not happen at runtime
///
/// # Errors
///
/// This function will return an error if one of the underlying IO errors or parse error occurs
/// on config and pom files
pub fn resolve(dependencies: Vec<Project>) -> anyhow::Result<Vec<Project>> {
    // load labt.lock file directory
    let mut path: PathBuf = current_dir().context("Unable to open current directory")?;
    path.push(LOCK_FILE);

    //initialize resolvers
    // TODO move these to a config for more flexibility
    // cache resolver
    let cache: Box<dyn Resolver> = Box::<CacheResolver>::default();
    // maven resolver
    let maven_url: Box<dyn Resolver> = Box::new(NetResolver::init(
        "central",
        "https://repo1.maven.org/maven2",
    )?);
    // google resolver
    let google_url: Box<dyn Resolver> =
        Box::new(NetResolver::init("google", "https://maven.google.com")?);

    // list of resolvers by their order of priority
    let resolvers = Rc::new(RefCell::new(vec![cache, google_url, maven_url]));

    // load resolved dependencies from lock file
    let mut resolved: Vec<ProjectDep> = if path.exists() {
        load_lock_dependencies()?
    } else {
        vec![]
    };
    let mut unresolved = vec![];

    // start a new spinner progress bar and add it to the global multi progress bar
    let spinner = Rc::new(RefCell::new(
        MULTI_PRPGRESS_BAR.with(|multi| multi.borrow().add(ProgressBar::new_spinner())),
    ));
    spinner
        .borrow()
        .enable_steady_tick(Duration::from_millis(100));
    spinner
        .borrow()
        .set_style(ProgressStyle::with_template("\n{spinner} {prefix:.blue} {wide_msg}").unwrap());

    let mut resolved_projects: Vec<Project> = Vec::new();

    for project in dependencies {
        // create a new project wrapper for dependency resolution
        let mut wrapper = ProjectWrapper::new(project.clone(), Rc::clone(&resolvers));
        wrapper.set_progress_bar(Some(spinner.clone()));

        // walk the dependency tree
        wrapper.build_tree(&mut resolved, &mut unresolved)?;
        resolved_projects.push(wrapper.project);
    }
    // clear progressbar
    spinner.borrow().finish_and_clear();

    let mut file = File::options()
        .write(true)
        .read(true)
        .create(true)
        .open(path)
        .context("Unable to open lock file")?;
    write_lock(&mut file, &resolved)?;
    save_dependencies(&resolved).context("Failed downloading saved dependencies")?;
    Ok(resolved_projects)
}
