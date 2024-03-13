use std::cell::RefCell;
use std::cmp::Ordering;
use std::env::current_dir;
use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::config::get_config;
use crate::config::lock::strings::ARTIFACT_ID;
use crate::config::lock::strings::DEPENDENCIES;
use crate::config::lock::strings::GROUP_ID;
use crate::config::lock::strings::LOCK_FILE;
use crate::config::lock::strings::PROJECT;
use crate::config::lock::strings::VERSION;
use crate::pom::{self, Project};
use crate::MULTI_PRPGRESS_BAR;

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
use log::error;
use log::info;
use pom::parse_pom_async;
use reqwest::Client;
use serde::Serialize;
use tokio::io::BufReader;
use tokio_util::io::StreamReader;
use toml_edit::value;
use toml_edit::Array;
use toml_edit::ArrayOfTables;
use toml_edit::Document;
use toml_edit::Formatted;
use toml_edit::Item;
use toml_edit::Table;

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
            for (artifact_id, table) in deps {
                let project = Project::new(&table.group_id, &artifact_id, &table.version);
                resolve(project)?;
            }
        }
        Ok(())
    }
}
#[derive(Debug, Default, Serialize)]
pub struct ProjectDep {
    pub artifact_id: String,
    pub group_id: String,
    pub version: String,
    pub dependencies: Vec<String>,
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

struct ProjectWrapper {
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
    fn fetch(&mut self) -> anyhow::Result<()> {
        let mut found = false;
        for resolver in self.resolvers.borrow_mut().iter() {
            if let Err(err) = resolver.fetch(&mut self.project) {
                match err.kind() {
                    ResolverErrorKind::NotFound => continue,
                    _ => {
                        return Err(anyhow!(err).context(format!(
                            "Error while trying to resolve dependency on {}",
                            resolver.get_name()
                        )));
                    }
                }
            } else {
                found = true;
                break;
            }
        }

        // we failed to fetch dependency across all configured resolvers
        if !found {
            bail!(
                "Dependency \"{}\" not found on all configured resolvers",
                self.project.qualified_name()
            );
        }

        Ok(())
    }
}

trait BuildTree {
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
        // fetch the dependencies of this project
        if let Err(err) = self.fetch() {
            error!(target: "fetch", "{} scope {:?} \n{:?}",
                self.project.qualified_name(),
                self.project.get_scope(),
                err
            );
        }
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
            // TODO some tests need to be done on this block, if feels "hacky"
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
        resolved.push(ProjectDep {
            artifact_id: self.project.get_artifact_id(),
            group_id: self.project.get_group_id(),
            version: self.project.get_version(),
            dependencies: self
                .project
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
        });
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

pub fn resolve(project: Project) -> anyhow::Result<Project> {
    let mut path: PathBuf = current_dir().context("Unable to open current directory")?;
    path.push(LOCK_FILE);

    //initialize resolvers
    let maven_url: Box<dyn Resolver> = Box::new(NetResolver::init(
        "central",
        "https://repo1.maven.org/maven2",
    )?);
    let google_url: Box<dyn Resolver> =
        Box::new(NetResolver::init("google", "https://maven.google.com")?);
    let local: Box<dyn Resolver> = Box::new(NetResolver::init("local", "http://localhost:3000")?);

    let resolvers = RefCell::new(vec![local, google_url, maven_url]);

    let mut file = File::options()
        .write(true)
        .read(true)
        .create(true)
        .open(path)
        .context("Unable to open lock file")?;

    let mut resolved: Vec<ProjectDep> = load_lock_dependencies_with(&mut file)?;
    let mut unresolved = vec![];
    let spinner = Rc::new(RefCell::new(
        MULTI_PRPGRESS_BAR.with(|multi| multi.borrow().add(ProgressBar::new_spinner())),
    ));
    spinner
        .borrow()
        .enable_steady_tick(Duration::from_millis(100));
    spinner
        .borrow()
        .set_style(ProgressStyle::with_template("\n{spinner} {prefix:.blue} {wide_msg}").unwrap());

    let mut wrapper = ProjectWrapper::new(project.clone(), Rc::new(resolvers));
    wrapper.set_progress_bar(Some(spinner.clone()));

    wrapper.build_tree(&mut resolved, &mut unresolved)?;
    spinner.borrow().finish_and_clear();
    write_lock(&mut file, resolved)?;
    Ok(wrapper.project)
}

pub fn load_lock_dependencies() -> anyhow::Result<Vec<ProjectDep>> {
    let mut path: PathBuf = current_dir().context("Unable to open current directory")?;
    path.push(LOCK_FILE);

    let mut file = File::open(path).context("Unable to open lock file")?;

    let resolved: Vec<ProjectDep> = load_lock_dependencies_with(&mut file)?;

    Ok(resolved)
}

pub fn load_lock_dependencies_with(file: &mut File) -> anyhow::Result<Vec<ProjectDep>> {
    let mut resolved: Vec<ProjectDep> = vec![];

    let mut lock = String::new();
    file.read_to_string(&mut lock)
        .context("Unable to read lock file contents")?;

    let lock = lock
        .parse::<Document>()
        .context("Unable to parse lock file")?;

    if lock.contains_array_of_tables(PROJECT) {
        if let Some(table_arrays) = lock[PROJECT].as_array_of_tables() {
            let missing_err = |key: &str, position: usize| -> anyhow::Result<()> {
                bail!("Missing {} in table at position {} ", key, position);
            };

            for dep in table_arrays.iter() {
                let mut project = ProjectDep::default();
                let position = dep.position().unwrap_or(0);

                // check for artifact_id
                if let Some(artifact_id) = dep.get(ARTIFACT_ID) {
                    project.artifact_id = artifact_id
                        .as_value()
                        .unwrap_or(&toml_edit::Value::String(Formatted::new(String::new())))
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                } else {
                    missing_err(ARTIFACT_ID, position)?;
                }

                // check for group_id
                if let Some(group_id) = dep.get(GROUP_ID) {
                    project.group_id = group_id
                        .as_value()
                        .unwrap_or(&toml_edit::Value::String(Formatted::new(String::new())))
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                } else {
                    missing_err(GROUP_ID, position)?;
                }

                // check for version
                if let Some(version) = dep.get(VERSION) {
                    project.version = version
                        .as_value()
                        .unwrap_or(&toml_edit::Value::String(Formatted::new(String::new())))
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                } else {
                    missing_err(VERSION, position)?;
                }

                if let Some(dependencies) = dep.get(DEPENDENCIES) {
                    if let Some(array) = dependencies.as_array() {
                        let mut deps = Vec::new();
                        deps.extend(array.iter().map(|d| d.as_str().unwrap_or("").to_string()));
                        project.dependencies = deps;
                    }
                }

                resolved.push(project);
            }
        }
    }
    Ok(resolved)
}

pub fn write_lock(file: &mut File, resolved: Vec<ProjectDep>) -> anyhow::Result<()> {
    let mut lock = String::new();
    file.read_to_string(&mut lock)
        .context("Unable to read lock file contents")?;

    let mut lock = lock
        .parse::<Document>()
        .context("Unable to parse lock file")?;

    // map dependencies ProjectTable to Tables and extend
    // the ArrayOfTables with the resulting iterator
    let mut tables_array = ArrayOfTables::new();
    tables_array.extend(resolved.iter().map(|dep| {
        let mut deps_array = Array::new();
        deps_array.decor_mut().set_suffix("\n");
        deps_array.extend(dep.dependencies.iter());

        let mut table = Table::new();
        table.insert(ARTIFACT_ID, value(&dep.artifact_id));
        table.insert(GROUP_ID, value(&dep.group_id));
        table.insert(VERSION, value(&dep.version));
        table.insert(DEPENDENCIES, value(deps_array));
        table
    }));

    lock["project"] = Item::ArrayOfTables(tables_array);

    file.seek(io::SeekFrom::Start(0))?;
    file.write_all(lock.to_string().as_bytes())
        .context("Error writing lock file")?;

    Ok(())
}
