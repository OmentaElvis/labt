use std::cmp::Ordering;
use std::env::current_dir;
use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;

use crate::pom::{self, Project};

use super::Submodule;
use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use clap::Args;
use futures_util::TryStreamExt;
use pom::{parse_pom, parse_pom_async};
use reqwest::Client;
use tokio::io::BufReader;
use tokio_util::io::StreamReader;
use toml_edit::value;
use toml_edit::Array;
use toml_edit::ArrayOfTables;
use toml_edit::Document;
use toml_edit::Item;
use toml_edit::Table;

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
#[derive(Debug)]
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
trait BuildTree {
    fn build_tree(
        &mut self,
        resolved: &mut Vec<ProjectDep>,
        unresolved: &mut Vec<String>,
    ) -> anyhow::Result<()>;
    fn fetch(&mut self) -> anyhow::Result<()>;
}

impl BuildTree for Project {
    fn build_tree(
        &mut self,
        resolved: &mut Vec<ProjectDep>,
        unresolved: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        // push this project to unresolved
        unresolved.push(self.qualified_name());
        println!(
            "\x1b[94m[Fetch]\x1b[0m \x1b[90m{:?}\x1b[0m {}:{}:{}",
            self.get_scope(),
            self.get_group_id(),
            self.get_artifact_id(),
            self.get_version(),
        );
        // fetch the dependencies of this project
        if let Err(err) = self.fetch() {
            println!(
                "\x1b[91m[Error]\x1b[0m \x1b[90m{:?}\x1b[0m \x1b[90m{}\x1b[0m \n{:?}",
                self.get_scope(),
                self.qualified_name(),
                err
            );
        }
        let excludes = Rc::new(self.get_excludes().clone());
        self.get_dependencies_mut().retain(|dep| {
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

        for dep in &mut self.get_dependencies_mut().iter_mut() {
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
            dep.build_tree(resolved, unresolved)?;
        }

        // remove this project from unresolved
        unresolved.pop();

        // add this project to list of resolved
        resolved.push(ProjectDep {
            artifact_id: self.get_artifact_id(),
            group_id: self.get_group_id(),
            version: self.get_version(),
            dependencies: self
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
    fn fetch(&mut self) -> anyhow::Result<()> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("Labt/1.1")
            .build()?;
        let maven_url = format!(
            "https://repo1.maven.org/maven2/{0}/{1}/{2}/{1}-{2}.pom",
            self.get_group_id().replace('.', "/"),
            self.get_artifact_id(),
            self.get_version(),
        );
        let google_url = format!(
            "https://maven.google.com/{0}/{1}/{2}/{1}-{2}.pom",
            self.get_group_id().replace('.', "/"),
            self.get_artifact_id(),
            self.get_version(),
        );
        let cached = format!(
            "http://localhost:3000/{0}/{1}/{2}/{1}-{2}.pom",
            self.get_group_id(),
            self.get_artifact_id(),
            self.get_version()
        );
        let urls = [cached, maven_url, google_url];
        let mut i = 0;

        let response = loop {
            if i >= urls.len() {
                break None;
            }

            let res = client.get(&urls[i]).send()?;
            if res.status().is_success() {
                break Some(res);
            }

            i += 1;
        };

        if let Some(res) = response {
            let reader = io::BufReader::new(res);
            let p = parse_pom(reader, self.to_owned())?;
            self.get_dependencies_mut()
                .extend(p.get_dependencies().iter().map(|dep| dep.to_owned()));
        } else {
            return Err(anyhow!(format!(
                "Failed to resolve: {}:{}:{}",
                self.get_artifact_id(),
                self.get_group_id(),
                self.get_version()
            )));
        }

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

pub fn resolve(project: &mut Project) -> anyhow::Result<()> {
    let mut resolved = vec![];
    let mut unresolved = vec![];
    project.build_tree(&mut resolved, &mut unresolved)?;

    write_lock(resolved)?;
    Ok(())
}

pub fn dump(project: &Project) {
    println!(
        "{}:{}:{}",
        project.get_group_id(),
        project.get_artifact_id(),
        project.get_version()
    );
}

pub fn write_lock(resolved: Vec<ProjectDep>) -> anyhow::Result<()> {
    let mut path: PathBuf = current_dir().context("Unable to open current directory")?;
    path.push("Labt.lock");

    let mut file = File::options()
        .truncate(true)
        .write(true)
        .read(true)
        .create(true)
        .open(path)
        .context("Unable to open lock file")?;
    let mut lock = String::new();
    file.read_to_string(&mut lock)
        .context("Unable to read lock file contents")?;
    println!("{}", lock);

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
        table.insert("artifact_id", value(&dep.artifact_id));
        table.insert("group_id", value(&dep.group_id));
        table.insert("version", value(&dep.version));
        table.insert("dependencies", value(deps_array));
        table
    }));

    lock["project"] = Item::ArrayOfTables(tables_array);

    file.write_all(lock.to_string().as_bytes())
        .context("Error writing lock file")?;

    Ok(())
}
