use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt::Display;
use std::fs::File;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::caching::save_dependencies;
use crate::config::lock::strings::LOCK_FILE;
use crate::config::lock::write_lock;
use crate::config::lock::{load_labt_lock, LabtLock};
use crate::config::{get_config, get_resolvers_from_config};
use crate::pom::{self, Project, VersionRange};
use crate::pom::{Scope, VersionRequirement};
use crate::{get_project_root, MULTI_PROGRESS_BAR};

use super::resolvers::ResolverErrorKind;
use super::resolvers::{Resolver, CACHE_REPO_STR};
use super::Submodule;
use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use clap::Args;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use log::info;

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
// =================
// Entry point
// =================
impl Submodule for Resolve {
    fn run(&mut self) -> Result<()> {
        // try reading toml file
        let config = get_config()?;
        if let Some(deps) = &config.dependencies {
            let dependencies: Vec<Project> = deps
                .iter()
                .map(|(artifact_id, table)| {
                    let mut p = Project::new(&table.group_id, artifact_id, &table.version);
                    p.set_selected_version(Some(table.version.clone()));
                    p
                })
                .collect();
            let resolvers =
                get_resolvers_from_config(&config).context("Failed to get resolvers")?;

            resolve(dependencies, resolvers)?;
        }
        Ok(())
    }
}
#[derive(Debug, Default, Clone)]
pub struct ProjectDep {
    pub artifact_id: String,
    pub group_id: String,
    pub version: String,
    pub scope: Scope,
    pub dependencies: Vec<String>,
    pub base_url: String,
    pub packaging: String,
    pub cache_hit: bool,
    pub constraints: Option<Constraint>,
}

/// This is a summary of all dependency constraints that we need to
/// prevent conflicts and other crazy stuff
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Constraint {
    /// The minimum allowed version
    pub min: Option<(bool, String)>,
    /// The minimum allowed version
    pub max: Option<(bool, String)>,
    /// An exact version version
    pub exact: Option<String>,
    /// Exclusions
    pub exclusions: Vec<(VersionRange, VersionRange)>,
}

impl Display for Constraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some((inclusive, min)) = &self.min {
            if *inclusive {
                write!(f, "{min}>=")?;
            } else {
                write!(f, "{min}>")?;
            }
        }
        write!(f, "v")?;
        if let Some((inclusive, max)) = &self.max {
            if *inclusive {
                write!(f, "<={max}")?;
            } else {
                write!(f, "<{max}")?;
            }
        }
        if let Some(exact) = &self.exact {
            write!(f, "={exact}")?;
        }
        if !self.exclusions.is_empty() {
            write!(f, " with exclusions.")?;
            for (start, end) in &self.exclusions {
                write!(f, "({start}, {end}), ")?;
            }
        }
        Ok(())
    }
}

impl From<&Constraint> for Constraint {
    fn from(value: &Constraint) -> Self {
        Constraint {
            min: value.min.clone(),
            max: value.max.clone(),
            exact: value.exact.clone(),
            exclusions: value.exclusions.clone(),
        }
    }
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

impl TryFrom<&Project> for ProjectDep {
    type Error = anyhow::Error;
    fn try_from(project: &Project) -> std::prelude::v1::Result<Self, Self::Error> {
        let mut deps = Vec::new();
        for p in project.get_dependencies() {
            deps.push(p.qualified_name().context(format!(
                "Version not resolved for package {}:{} on dependency {}:{}",
                project.get_group_id(),
                p.get_artifact_id(),
                p.get_group_id(),
                p.get_artifact_id()
            ))?);
        }
        let c = Constraint::default().contain(project.get_version())?;
        Ok(ProjectDep {
            artifact_id: project.get_artifact_id(),
            group_id: project.get_group_id(),
            version: project
                .get_selected_version()
                .clone()
                .context("Version not set for package")?,
            scope: project.get_scope(),
            packaging: project.get_packaging(),
            constraints: Some(c),
            dependencies: deps,
            ..Default::default()
        })
    }
}

impl ProjectDep {
    /// Gets the root url for this dependency
    /// e.g. https://maven.example.com/maven2/groupId/artifactId/version/
    /// This is just ready to append a required file type from the repo
    pub fn get_root_url(&self) -> String {
        // check if base url ends with foward slash
        if self.base_url.ends_with('/') {
            format!(
                "{}{}/{}/{}/",
                self.base_url,
                self.group_id.replace('.', "/"),
                self.artifact_id,
                self.version
            )
        } else {
            format!(
                "{}/{}/{}/{}/",
                self.base_url,
                self.group_id.replace('.', "/"),
                self.artifact_id,
                self.version
            )
        }
    }
    /// Tries to obtain base url from root url
    /// e.g. https://maven.example.com/maven2/groupId/artifactId/version/
    /// resolves https://maven.example.com/maven2/
    /// likely very unstablesince it uses string replace internally
    pub fn set_base_url_from_root(&mut self, url: String) {
        let path = if url.ends_with('/') {
            // include the trailing slash
            format!(
                "{}/{}/{}/",
                self.group_id.replace('.', "/"),
                self.artifact_id,
                self.version
            )
        } else {
            format!(
                "{}/{}/{}",
                self.group_id.replace('.', "/"),
                self.artifact_id,
                self.version
            )
        };

        self.base_url = url.replace(path.as_str(), "");
    }
}

impl Constraint {
    /// checks if a version requirement falls in this current constraint
    pub fn within<'version>(&self, versions: &'version VersionRequirement) -> anyhow::Result<bool> {
        let version_parse_error = |a: &'version String, b| {
            anyhow!("Failed to compare versions \"{}\" and \"{}\". Unable to parse one of the versions.", a, b)
        };
        match versions {
            // version was not set, so it falls within
            VersionRequirement::Unset => Ok(true),
            // Soft version
            VersionRequirement::Soft(version) => {
                // if an exact version is specified, lock on it
                if let Some(exact) = &self.exact {
                    return version_compare::compare_to(version, exact, version_compare::Cmp::Eq)
                        .map_err(|_| version_parse_error(version, exact));
                }
                // Min
                if let Some((inclusive, min)) = &self.min {
                    // reject any version below this number
                    if *inclusive {
                        if !version_compare::compare_to(version, min, version_compare::Cmp::Ge)
                            .map_err(|_| version_parse_error(version, min))?
                        {
                            return Ok(false);
                        }
                    } else if !version_compare::compare_to(version, min, version_compare::Cmp::Gt)
                        .map_err(|_| version_parse_error(version, min))?
                    {
                        return Ok(false);
                    }
                }
                // Max
                if let Some((inclusive, max)) = &self.max {
                    // reject any versions above this version
                    if *inclusive {
                        if !version_compare::compare_to(version, max, version_compare::Cmp::Le)
                            .map_err(|_| version_parse_error(version, max))?
                        {
                            return Ok(false);
                        }
                    } else if !version_compare::compare_to(version, max, version_compare::Cmp::Lt)
                        .map_err(|_| version_parse_error(version, max))?
                    {
                        return Ok(false);
                    }
                }

                // Exclusions
                for (start, end) in &self.exclusions {
                    if Self::within_range(version, start, end) {
                        return Ok(false);
                    }
                }

                Ok(true)
            }
            VersionRequirement::Hard(hard_constraints) => {
                // short circuit of first musmatch
                for c in hard_constraints {
                    match c {
                        // check if these fall below the maximum version.
                        pom::VersionRange::Ge(version) => {
                            // if an exact version is specified, lock on it
                            if let Some(exact) = &self.exact {
                                return version_compare::compare_to(
                                    version,
                                    exact,
                                    version_compare::Cmp::Eq,
                                )
                                .map_err(|_| version_parse_error(version, exact));
                            }
                            if let Some((inclusive, max)) = &self.max {
                                if *inclusive {
                                    if !version_compare::compare_to(
                                        version,
                                        max,
                                        version_compare::Cmp::Le,
                                    )
                                    .map_err(|_| version_parse_error(version, max))?
                                    {
                                        return Ok(false);
                                    }
                                } else if !version_compare::compare_to(
                                    version,
                                    max,
                                    version_compare::Cmp::Lt,
                                )
                                .map_err(|_| version_parse_error(version, max))?
                                {
                                    return Ok(false);
                                }
                            }
                        }
                        pom::VersionRange::Gt(version) => {
                            // if an exact version is specified, lock on it
                            if let Some(exact) = &self.exact {
                                return version_compare::compare_to(
                                    version,
                                    exact,
                                    version_compare::Cmp::Eq,
                                )
                                .map_err(|_| version_parse_error(version, exact));
                            }
                            if let Some((_, max)) = &self.max {
                                if !version_compare::compare_to(
                                    version,
                                    max,
                                    version_compare::Cmp::Lt,
                                )
                                .map_err(|_| version_parse_error(version, max))?
                                {
                                    return Ok(false);
                                }
                            }
                        }
                        // check if these fall above the minimum version.
                        pom::VersionRange::Lt(version) => {
                            // if an exact version is specified, lock on it
                            if let Some(exact) = &self.exact {
                                return version_compare::compare_to(
                                    version,
                                    exact,
                                    version_compare::Cmp::Eq,
                                )
                                .map_err(|_| version_parse_error(version, exact));
                            }
                            if let Some((_, min)) = &self.min {
                                if !version_compare::compare_to(
                                    version,
                                    min,
                                    version_compare::Cmp::Gt,
                                )
                                .map_err(|_| version_parse_error(version, min))?
                                {
                                    return Ok(false);
                                }
                            }
                        }
                        pom::VersionRange::Le(version) => {
                            // if an exact version is specified, lock on it
                            if let Some(exact) = &self.exact {
                                return version_compare::compare_to(
                                    version,
                                    exact,
                                    version_compare::Cmp::Eq,
                                )
                                .map_err(|_| version_parse_error(version, exact));
                            }
                            if let Some((inclusive, min)) = &self.min {
                                if *inclusive {
                                    if !version_compare::compare_to(
                                        version,
                                        min,
                                        version_compare::Cmp::Ge,
                                    )
                                    .map_err(|_| version_parse_error(version, min))?
                                    {
                                        return Ok(false);
                                    }
                                } else if !version_compare::compare_to(
                                    version,
                                    min,
                                    version_compare::Cmp::Gt,
                                )
                                .map_err(|_| version_parse_error(version, min))?
                                {
                                    return Ok(false);
                                }
                            }
                        }
                        // Make sure the version matches exact if set and lies within the min max
                        pom::VersionRange::Eq(version) => {
                            // if an exact version is specified, lock on it
                            if let Some(exact) = &self.exact {
                                return version_compare::compare_to(
                                    version,
                                    exact,
                                    version_compare::Cmp::Eq,
                                )
                                .map_err(|_| version_parse_error(version, exact));
                            }
                            // just to confirm this lies within boundaries
                            // Min
                            if let Some((inclusive, min)) = &self.min {
                                // reject any version below this number
                                if *inclusive {
                                    if !version_compare::compare_to(
                                        version,
                                        min,
                                        version_compare::Cmp::Ge,
                                    )
                                    .map_err(|_| version_parse_error(version, min))?
                                    {
                                        return Ok(false);
                                    }
                                } else if !version_compare::compare_to(
                                    version,
                                    min,
                                    version_compare::Cmp::Gt,
                                )
                                .map_err(|_| version_parse_error(version, min))?
                                {
                                    return Ok(false);
                                }
                            }
                            // Max
                            if let Some((inclusive, max)) = &self.max {
                                // reject any versions above this version
                                if *inclusive {
                                    if !version_compare::compare_to(
                                        version,
                                        max,
                                        version_compare::Cmp::Le,
                                    )
                                    .map_err(|_| version_parse_error(version, max))?
                                    {
                                        return Ok(false);
                                    }
                                } else if !version_compare::compare_to(
                                    version,
                                    max,
                                    version_compare::Cmp::Lt,
                                )
                                .map_err(|_| version_parse_error(version, max))?
                                {
                                    return Ok(false);
                                }
                            }
                            // Exclusions
                            for (start, end) in &self.exclusions {
                                if Self::within_range(version, start, end) {
                                    return Ok(false);
                                }
                            }

                            return Ok(true);
                        }
                    }
                }

                Ok(true)
            }
        }
    }
    pub fn within_range(target: &String, start: &VersionRange, end: &VersionRange) -> bool {
        // if a version is on the left of the start, then it is out of range
        // if version is on right of the end, then it is out of range
        //
        match start {
            VersionRange::Gt(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Gt).unwrap() {
                    return false;
                }
            }
            VersionRange::Ge(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Ge).unwrap() {
                    return false;
                }
            }
            // The value is within this v and -ve Infinity
            VersionRange::Lt(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Lt).unwrap() {
                    return false;
                }
            }
            VersionRange::Le(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Le).unwrap() {
                    return false;
                }
            }
            VersionRange::Eq(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Eq).unwrap() {
                    return false;
                }
            }
        }

        match end {
            VersionRange::Lt(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Lt).unwrap() {
                    return false;
                }
            }
            VersionRange::Le(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Le).unwrap() {
                    return false;
                }
            }
            // The value is within this v and +ve infinity
            VersionRange::Gt(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Gt).unwrap() {
                    return false;
                }
            }
            VersionRange::Ge(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Ge).unwrap() {
                    return false;
                }
            }
            VersionRange::Eq(v) => {
                if !version_compare::compare_to(target, v, version_compare::Cmp::Eq).unwrap() {
                    return false;
                }
            }
        }

        true
    }
    pub fn contain(&self, versions: &VersionRequirement) -> anyhow::Result<Constraint> {
        let mut c = self.clone();
        c.contain_mut(versions)?;
        Ok(c)
    }
    /// Will try to contain the extremes of incoming version constraints.
    /// If it cant fit then that is a version conflict. An error is thrown.
    pub fn contain_mut(&mut self, versions: &VersionRequirement) -> anyhow::Result<()> {
        match versions {
            VersionRequirement::Soft(_) => {
                // doesnt really matter since its a suggestion
                Ok(())
            }
            VersionRequirement::Unset => {
                // Looks good since we did not choose a version then the constraint does not need tampering
                Ok(())
            }
            VersionRequirement::Hard(hard_constraints) => {
                // The critical stuff we care about.
                let mut number_line = Vec::new(); // a virtual number line for detection of exclusions or breaks.

                // if we have a min and max add them to the number line to serve as a guardrails
                if let Some((inclusive, min)) = &self.min {
                    if *inclusive {
                        number_line.push(VersionRange::Ge(min.to_string()));
                    } else {
                        number_line.push(VersionRange::Gt(min.to_string()));
                    }
                }

                if let Some((inclusive, max)) = &self.max {
                    if *inclusive {
                        number_line.push(VersionRange::Le(max.to_string()));
                    } else {
                        number_line.push(VersionRange::Lt(max.to_string()));
                    }
                }

                // add the rest of the constraints and sort them
                number_line.extend(hard_constraints.iter().cloned());
                // sort
                number_line.sort_by(|a, b| a.partial_cmp(b).unwrap());
                // let mut version_eq = |v| {
                //     if let Some(exact) = &self.exact {
                //         // This is an error. We are conflicting very hard
                //         bail!("Conflicting exact versions set with existing ={exact} and incomming ={v}");
                //     } else {
                //         self.exact = Some(v);
                //     }
                //     Ok(())
                // };
                #[derive(Debug)]
                enum Edges {
                    Infinity,
                    Bound(VersionRange),
                }

                // holds the encountered Ge & Gt
                let mut stack: Vec<VersionRange> = Vec::new();
                // Holds  >/>= , </<= pairs
                let mut pairs: Vec<(Edges, Edges)> = Vec::with_capacity(number_line.len() / 2);
                // Holds encountered '=' which will be used to check if falls within min/max and lock it.
                let mut exacts: Vec<String> = Vec::new();

                // Holds a new constraint defination that we are building
                // let mut constraint = Constraint::default();

                let old_min = match &self.min {
                    Some((inclusive, min)) => {
                        if *inclusive {
                            Some(VersionRange::Ge(min.clone()))
                        } else {
                            Some(VersionRange::Gt(min.clone()))
                        }
                    }
                    None => None,
                };
                let old_max = match &self.max {
                    Some((inclusive, max)) => {
                        if *inclusive {
                            Some(VersionRange::Le(max.clone()))
                        } else {
                            Some(VersionRange::Lt(max.clone()))
                        }
                    }
                    None => None,
                };

                *self = Constraint {
                    exact: self.exact.clone(),
                    exclusions: self.exclusions.clone(),
                    ..Default::default()
                };

                // collect all the pairs of inequalities
                for c in number_line {
                    match c {
                        VersionRange::Ge(_) | VersionRange::Gt(_) => {
                            stack.push(c);
                        }
                        VersionRange::Le(_) | VersionRange::Lt(_) => {
                            if let Some(open) = stack.pop() {
                                // We have a matching pair
                                pairs.push((Edges::Bound(open), Edges::Bound(c)));
                            } else {
                                // Missing a partner pair Mark it as Negative Infinity
                                pairs.push((Edges::Infinity, Edges::Bound(c)));
                            }
                        }
                        VersionRange::Eq(v) => {
                            // pile up all exacts and check for boundaries when done
                            exacts.push(v);
                        }
                    }
                }

                // check for struglers
                if !stack.is_empty() {
                    // if no pairs was collected then the last element is the minimum bound
                    if pairs.is_empty() {
                        // we need the first occurrence of LT/Le as max and last occurence of GT/GE as min
                        let mut min = None;
                        let mut max = None;

                        for s in stack {
                            match s {
                                VersionRange::Gt(v) => {
                                    min = Some((false, v.clone()));
                                }
                                VersionRange::Ge(v) => {
                                    min = Some((true, v.clone()));
                                }
                                // Only set once
                                VersionRange::Lt(v) => {
                                    if max.is_none() {
                                        max = Some((false, v.clone()));
                                    }
                                }
                                VersionRange::Le(v) => {
                                    if max.is_none() {
                                        max = Some((true, v.clone()));
                                    }
                                }
                                _ => {
                                    unreachable!();
                                }
                            }
                        }
                        self.min = min;
                        self.max = max;
                    } else {
                        // our pair list is not empty.
                        // This means we had extra ranges whose bound is infinity

                        // Loop through each remaining bound adding the lower and upper threshold
                        // For Lt/Le take the first occurence and limit it with infinity on the left
                        let mut max_pair = None;
                        // For Gt/Ge take the last occurrence and bind it with infinity to the right
                        let mut min_pair = None;

                        for s in stack {
                            match s {
                                VersionRange::Gt(v) => {
                                    min_pair = Some((
                                        Edges::Bound(VersionRange::Gt(v.to_string())),
                                        Edges::Infinity,
                                    ));
                                }
                                VersionRange::Ge(v) => {
                                    min_pair = Some((
                                        Edges::Bound(VersionRange::Ge(v.to_string())),
                                        Edges::Infinity,
                                    ));
                                }
                                VersionRange::Lt(v) => {
                                    if max_pair.is_none() {
                                        max_pair = Some((
                                            Edges::Infinity,
                                            Edges::Bound(VersionRange::Lt(v.to_string())),
                                        ));
                                    }
                                }
                                VersionRange::Le(v) => {
                                    if max_pair.is_none() {
                                        max_pair = Some((
                                            Edges::Infinity,
                                            Edges::Bound(VersionRange::Le(v.to_string())),
                                        ));
                                    }
                                }
                                _ => {
                                    unreachable!();
                                }
                            }
                        }
                        if let Some(min) = min_pair {
                            pairs.push(min);
                        }

                        if let Some(max) = max_pair {
                            pairs.push(max);
                        }
                    }
                }

                let mut min = None;
                let mut max = None;
                // loop through the pair updating the min/max
                pairs.into_iter().for_each(|(start, end)| {
                    // if start
                    match start {
                        Edges::Infinity => {
                            // Our bound is -ve infinity
                            if min.is_none() {
                                min = Some(Edges::Infinity);
                            }
                        }
                        Edges::Bound(c) => {
                            // check our lower limit if set
                            match &min {
                                // Someone explicitly specified our lower limit earlier
                                Some(bound) => {
                                    match bound {
                                        Edges::Infinity => {
                                            // We have a lower bound that is -ve infinity
                                            // This is a very specific rare scenario for things like:
                                            // |------------|-------|-------------|
                                            // -ve(inf)     <5.9    >5.9          +ve(inf)
                                            //                      ^ we are here
                                            // min         max     start         end

                                            // we want to detect an exclusion
                                            // There is no min bound value to compare with c so it is automatic less than
                                            if let Some(Edges::Bound(range)) = &max {
                                                // flip the symbols
                                                // The exclusion end is this pair start value while exclusion start is the previous max value
                                                let exclusion_end = match c {
                                                    VersionRange::Gt(v) => VersionRange::Le(v),
                                                    VersionRange::Ge(v) => VersionRange::Lt(v),
                                                    _ => {
                                                        unreachable!();
                                                    }
                                                };

                                                let exclusion_start = match range {
                                                    VersionRange::Lt(v) => {
                                                        VersionRange::Ge(v.to_string())
                                                    }
                                                    VersionRange::Le(v) => {
                                                        VersionRange::Gt(v.to_string())
                                                    }
                                                    _ => {
                                                        unreachable!();
                                                    }
                                                };
                                                self.exclusions
                                                    .push((exclusion_start, exclusion_end));
                                                // update the new max version
                                                max = Some(end);
                                                return;
                                            }
                                        }
                                        Edges::Bound(min_bound) => {
                                            // We have a lower bound set to an absolute value
                                            if &c > min_bound {
                                                // This particular bound is within the right of the number line
                                                //so might be an exclusion
                                                // |------------->
                                                // c             min_bound

                                                // check for a max
                                                match &max {
                                                    Some(max_bound) => {
                                                        match max_bound {
                                                            Edges::Infinity => {
                                                                // not wanted but can still happen
                                                            }
                                                            Edges::Bound(range) => {
                                                                // flip the symbols
                                                                // The exclusion end is this pair start value while exclusion start is the previous max value
                                                                let exclusion_end = match c {
                                                                    VersionRange::Gt(v) => {
                                                                        VersionRange::Le(v)
                                                                    }
                                                                    VersionRange::Ge(v) => {
                                                                        VersionRange::Lt(v)
                                                                    }
                                                                    _ => {
                                                                        unreachable!();
                                                                    }
                                                                };

                                                                let exclusion_start = match range {
                                                                    VersionRange::Lt(v) => {
                                                                        VersionRange::Ge(
                                                                            v.to_string(),
                                                                        )
                                                                    }
                                                                    VersionRange::Le(v) => {
                                                                        VersionRange::Gt(
                                                                            v.to_string(),
                                                                        )
                                                                    }
                                                                    _ => {
                                                                        unreachable!();
                                                                    }
                                                                };
                                                                self.exclusions.push((
                                                                    exclusion_start,
                                                                    exclusion_end,
                                                                ));
                                                                // update the new max version
                                                                max = Some(end);
                                                                return;
                                                            }
                                                        }
                                                    }
                                                    None => {
                                                        // undesired
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                None => {
                                    // No bound set, so it is ok to set this as a lower bound
                                    min = Some(Edges::Bound(c));
                                }
                            }
                        }
                    }

                    match end {
                        Edges::Infinity => {
                            // Our bound is -ve infinity
                            if max.is_none() {
                                max = Some(Edges::Infinity);
                            }
                        }
                        Edges::Bound(c) => {
                            if max.is_none() {
                                // max was not set, use this
                                max = Some(Edges::Bound(c));
                            }
                        }
                    }
                });

                // set the min max to self
                // we also have an old min and max, we need to do a sanity check to ensure no one just pushed the boundaries
                match min {
                    Some(Edges::Bound(range)) => {
                        // range should be on right of old_min
                        if let Some(old_min) = old_min {
                            if range >= old_min {
                                match range {
                                    VersionRange::Gt(v) => {
                                        self.min = Some((false, v));
                                    }
                                    VersionRange::Ge(v) => {
                                        self.min = Some((true, v));
                                    }
                                    _ => {
                                        unreachable!();
                                    }
                                }
                            } else {
                                // This is incorrect version computation
                                bail!("The minimum allowed version inequality ({}) was moved out of range of the previous constraint minimum ({}).", range, old_min);
                            }
                        } else {
                            match range {
                                VersionRange::Gt(v) => {
                                    self.min = Some((false, v));
                                }
                                VersionRange::Ge(v) => {
                                    self.min = Some((true, v));
                                }
                                _ => {
                                    unreachable!();
                                }
                            }
                        }
                    }
                    _ => {
                        // dont care
                    }
                }

                match max {
                    Some(Edges::Bound(range)) => {
                        if let Some(old_max) = old_max {
                            if old_max >= range {
                                match range {
                                    VersionRange::Lt(v) => {
                                        self.max = Some((false, v));
                                    }
                                    VersionRange::Le(v) => {
                                        self.max = Some((true, v));
                                    }
                                    _ => {
                                        unreachable!();
                                    }
                                }
                            } else {
                                // This is incorrect version computation
                                bail!("The maximum allowed version inequality ({}) was moved out of range of the previous constraint maximum ({}).", range, old_max);
                            }
                        } else {
                            match range {
                                VersionRange::Lt(v) => {
                                    self.max = Some((false, v));
                                }
                                VersionRange::Le(v) => {
                                    self.max = Some((true, v));
                                }
                                _ => {
                                    unreachable!();
                                }
                            }
                        }
                    }
                    _ => {
                        // dont care
                    }
                }

                self.exact = self.exact.clone();

                // Now compute the exacts. If we have more than one exact, that is just an error
                for c in exacts {
                    // check if we already have an exact set.
                    if let Some(exact) = &self.exact {
                        // We have an exact, they must be equal before we proceed
                        if !version_compare::compare_to(&c, exact, version_compare::Cmp::Eq)
                            .unwrap()
                        {
                            // This is an error. We are conflicting very hard
                            bail!("Conflicting exact versions set with existing ={exact} and incomming ={c}");
                        }
                    } else {
                        // no exact was set. so set it
                        // but before doind so, confirm it is in range
                        if !self.within(&VersionRequirement::Hard(vec![VersionRange::Eq(c.clone())])).context(format!("Failed to check if version ={} is within allowed min and max range.", c))? {
                            bail!("A set hard version is out of allowed range of {} and {}.", self.min.clone().map_or("-Inf>=".to_string(), |(inclusive, v)|{
                                if inclusive {
                                    format!("{v}>=")
                                }else{
                                    format!("{v}>")
                                }
                            }), self.max.clone().map_or("<=+Inf".to_string(), |(inclusive, v)|{
                                if inclusive {
                                    format!("<={v}")
                                }else{
                                    format!("<{v}")
                                }
                            }));
                        }

                        self.exact = Some(c);
                    }
                }

                // Go through our excludes one last time to see if everything fits in perfectly
                for (start, end) in &self.exclusions {
                    // check exact
                    if let Some(exact) = &self.exact {
                        if Self::within_range(exact, start, end) {
                            bail!("An exact version was specified which was later explicitly excluded");
                        }
                    }
                    // Confirm that min and max are not inside the exclusion zone
                    if let (Some((_inclusive_min, min)), Some((_inclusive_max, max))) =
                        (&self.min, &self.max)
                    {
                        if Self::within_range(min, start, end)
                            && Self::within_range(max, start, end)
                        {
                            // Falls within the excluded region so this is a conflict.
                            bail!("The min and max of this constraint falls between an exclusion region.");
                        }
                    }
                }

                Ok(())
            }
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
                    cache_hit = resolver.get_name() == CACHE_REPO_STR;
                    break;
                }
            }
        }

        // we failed to fetch dependency across all configured resolvers
        if !found {
            bail!(
                "Dependency \"{}\" not found on all configured resolvers",
                self.project.qualified_name()?
            );
        }

        Ok((url, cache_hit))
    }

    fn compute_version(
        resolvers: Rc<RefCell<Vec<Box<dyn Resolver>>>>,
        dep: &Project,
    ) -> anyhow::Result<String> {
        let mut found = false;
        let mut version = String::new();

        for resolver in resolvers.borrow_mut().iter() {
            match resolver.calculate_version(dep) {
                Err(err) => match err.kind() {
                    ResolverErrorKind::NotFound => continue,
                    ResolverErrorKind::NoSelectedVersion => {
                        // metadata was found but no correct version was found
                        if resolver.get_name() == CACHE_REPO_STR {
                            // Maybe the cache is stale, ignore this and continue to net resolvers
                            continue;
                        } else {
                            // now this is an error
                            return Err(anyhow!(err).context(format!(
                                "Failed to calculate suitable version from {} resolver.",
                                resolver.get_name(),
                            )));
                        }
                    }
                    _ => {
                        return Err(anyhow!(err).context(format!(
                            "Error while trying to compute dependency version on {} resolver",
                            resolver.get_name()
                        )));
                    }
                },
                Ok(m_version) => {
                    found = true;
                    if resolver.get_name() == CACHE_REPO_STR {
                        log::trace!(target: "fetch", "Version for {}:{} resolved from cache as {m_version}. ", dep.get_group_id(), dep.get_artifact_id());
                    }
                    version = m_version;
                    break;
                }
            }
        }
        // we failed to fetch dependency across all configured resolvers
        if !found {
            bail!(
                "No correct version could be selected for \"{}:{}\" on all configured resolvers",
                dep.get_group_id(),
                dep.get_artifact_id()
            );
        }

        Ok(version)
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
        let selected_version_err = |group_id, artifact_id| {
            anyhow!(
                "No selected version set for package {}:{}",
                group_id,
                artifact_id
            )
        };
        let qualified_name = self.project.qualified_name().context(selected_version_err(
            self.project.get_group_id(),
            self.project.get_artifact_id(),
        ))?;
        let version = self
            .project
            .get_selected_version()
            .clone()
            .context(selected_version_err(
                self.project.get_group_id(),
                self.project.get_artifact_id(),
            ))?;

        // push this project to unresolved
        // Nearest Defination Wins
        // So we only compare group_id and artifact_id.
        // Whatever package is the parent of this circular dependency continues with its mess
        unresolved.push(format!(
            "{}:{}",
            self.project.get_group_id(),
            self.project.get_artifact_id()
        ));

        // Version was resolved earlier and this is just a version conflict
        let mut resolved_earlier = false;

        if let Some(prog) = &self.progress {
            let prog = prog.borrow();
            prog.set_message(format!(" {} ", qualified_name));
            prog.set_prefix("Fetching");
        }
        info!(target: "fetch", "{} scope {:?}",
            qualified_name,
            self.project.get_scope(),
        );
        // before we even proceed to do this "expensive" fetch just confirm this isn't a
        // potential version conflict and return instead
        if let Some((index, res)) = resolved.iter_mut().enumerate().find(|(_, res)| {
            res.group_id == self.project.get_group_id()
                && res.artifact_id == self.project.get_artifact_id()
        }) {
            // We have already seen this package with same group and artifact id.
            // but are the versions the same?

            // now check version for possible conflicts
            match version_compare::compare(&res.version, &version) {
                Ok(v) => match v {
                    version_compare::Cmp::Eq => {
                        // the versions are same, so skip resolving as it has already been resolved
                        unresolved.pop();
                        return Ok(());
                    }
                    version_compare::Cmp::Ne => {
                        // TODO not really sure of what to do with this
                    }
                    _ => {
                        // we have encountered two different versions.
                        // so try to go back in time and see what sort of constraints were set earlier.
                        if let Some(constraints) = &res.constraints {
                            // This will be used to see if it is safe to proceed or this is irrecoverable.
                            let c = self.project.get_version();
                            match c {
                                VersionRequirement::Soft(v) => {
                                    // A soft version has been suggested. We can override it at will.
                                    // always prefer a more later version
                                    if constraints.within(c).unwrap() {
                                        if version_compare::compare_to(
                                            &version,
                                            &res.version,
                                            version_compare::Cmp::Gt,
                                        )
                                        .unwrap()
                                        {
                                            resolved[index].version = v.clone();
                                            // shrink the constraint to fit
                                            if let Some(con) = &mut resolved[index].constraints {
                                                con.contain_mut(c)?;
                                            }
                                            resolved_earlier = true;
                                        } else {
                                            // we don't need this tree direction, its older
                                            unresolved.pop();
                                            return Ok(());
                                        }
                                    } else {
                                        // It is a softie and out of range; we dont need you
                                        unresolved.pop();
                                        return Ok(());
                                    }
                                }
                                VersionRequirement::Unset => {
                                    // Someone did not bother choosing a version, so any can do. In this case just use the already resolved one.
                                    unresolved.pop();
                                    return Ok(());
                                }
                                VersionRequirement::Hard(v) => {
                                    // These are commandments. If we cannot fit within, then this is not recoverable at all.
                                    if constraints.within(c).context(format!(
                                        "Failed to contain {:?} within {constraints}",
                                        v,
                                    ))? {
                                        // we can fit within, so make the version more strict than ever
                                        let containment = constraints.contain(c).unwrap();
                                        // If containment has an exact value set, update it and resolve
                                        if let Some(exact) = &containment.exact {
                                            resolved[index].version = exact.to_string();
                                        } else {
                                            // No exact value set, so we will prefer the latest
                                            // we need to recalculate our selected versions based on what will fit in versions available in metadata.xml
                                            // This is also why caching of the metadata is important because it is going to be constantly indexed.
                                            // also update self.project with the selected version so as to capture this calculated version

                                            // step 1: convert Constraints to versionrequirement
                                            let vr = VersionRequirement::from(&containment);

                                            // step 2: Calculate the suitable version
                                            let version = Self::compute_version(
                                                Rc::clone(&self.resolvers),
                                                Project::new(&res.group_id, &res.artifact_id, "")
                                                    .set_version(vr),
                                            )
                                            .context(format!("Failed to calculate a version for dependency {}:{}.", res.group_id, res.artifact_id))
                                            .context(format!("No appropriate version could be selected that could sastify {} on this version conflict.", containment))?;

                                            // step 3: use computed version
                                            resolved[index].version = version;
                                        }
                                        // update contained constraints
                                        resolved[index].constraints = Some(containment);
                                        resolved_earlier = true;
                                    } else {
                                        // the constraint cannot fit in this. This is fatal.
                                        bail!(
                                            "Dependency version conflict. {}:{} has a hard set version requirements as {} which does not fit within previously set constraint of {constraints}. Canceling the resolution.",
                                            self.project.get_group_id(),
                                            self.project.get_artifact_id(),
                                            v.iter().map(|k| k.to_string()).collect::<Vec<String>>().join(", ")
                                        );
                                    }
                                }
                            }
                        } else {
                            // constraint was not set back in time.
                            // ideally we should not be here, all projectdep should include a set constraint
                            // maybe leave this for backward compatibility
                            let c = self.project.get_version();

                            match c {
                                VersionRequirement::Unset => {
                                    // No version set, so just use already resolved
                                    unresolved.pop();
                                    return Ok(());
                                }
                                VersionRequirement::Soft(v) => {
                                    // two softies, prefer the latest
                                    if let Ok(cmp) = version_compare::compare(v, &res.version) {
                                        match cmp {
                                            version_compare::Cmp::Ge | version_compare::Cmp::Gt => {
                                                // resolve this, it is bigger
                                                resolved[index].version = v.clone();
                                                resolved_earlier = true;
                                                // This is a soft range no need to add a new one
                                            }
                                            version_compare::Cmp::Lt
                                            | version_compare::Cmp::Le
                                            | version_compare::Cmp::Eq => {
                                                unresolved.pop();
                                                return Ok(());
                                            }
                                            version_compare::Cmp::Ne => {
                                                unreachable!();
                                            }
                                        }
                                    }
                                }
                                VersionRequirement::Hard(_) => {
                                    // no constraint was specified on the already resolved version.
                                    // so just override everything
                                    let mut new_constraint = Constraint::default();
                                    new_constraint.contain_mut(c)?;
                                    resolved[index].version = version.clone();
                                    resolved[index].constraints = Some(new_constraint);
                                    resolved_earlier = true;
                                }
                            }
                        }
                    }
                },
                Err(_) => {
                    return Err(anyhow!(format!(
                        "Invalid versions string. Either {} or {} is invalid",
                        res.version, version
                    )));
                }
            }
        }
        // fetch the dependencies of this project
        let (url, cache_hit) = self.fetch().context(format!(
            "Error fetching {} scope {:?}",
            qualified_name,
            self.project.get_scope(),
        ))?;

        if let Some(parent) = &self.project.parent {
            // if we are given a parent, try to fetch the parent common dependencies
            // obtain the returned dependencies and merge it with our chain.

            // this should just bubble up the parent tree
            let mut wrapper = ProjectWrapper::new(
                Project::new(&parent.group_id, &parent.artifact_id, &parent.version),
                self.resolvers.clone(),
            );
            if let Some(progress) = &self.progress {
                wrapper.set_progress_bar(Some(progress.clone()));
            }
            log::trace!(target: "fetch", "Fetching parent {}:{}:{} for {}:{}", 
                parent.group_id,
                parent.artifact_id,
                parent.version,
                self.project.get_group_id(),
                self.project.get_artifact_id());

            wrapper.build_tree(resolved, unresolved)?;
            let management = wrapper.project.get_dependency_management();
            for dep in self.project.get_dependencies_mut() {
                let key = format!("{}:{}", dep.get_group_id(), dep.get_artifact_id());
                if let Some(parent_dep) = management.get(&key) {
                    dep.copy_parent(parent_dep);
                }
            }
            let deps = wrapper.project.get_dependencies_owned();
            self.project.get_dependencies_mut().extend(deps);
        }

        let excludes = Rc::new(self.project.get_excludes().clone());
        self.project.get_dependencies_mut().retain(|dep| {
            if dep.get_scope().ne(&pom::Scope::COMPILE) {
                return false;
            }

            if dep.is_optional() {
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
        for dep in self.project.get_dependencies_mut() {
            // use version resolvers to compute the version of this dependency if needed
            let version =
                Self::compute_version(Rc::clone(&self.resolvers), dep).context(format!(
                    "Failed to calculate a version for dependency {}:{}.", // the artifact might even not exist
                    dep.get_group_id(),
                    dep.get_artifact_id()
                ))?;
            // from here now on we have a version for even the recursive calls, therefore there should be no complaints
            dep.set_selected_version(Some(version.clone()));

            if unresolved.contains(&format!("{}:{}", dep.get_group_id(), dep.get_artifact_id())) {
                // Circular dep, if encountered,
                // TODO check config for ignore, warn, or Error
                log::trace!(target: "fetch", "Circular dependency detected for {}:{}, Using \"Nearest defination wins\". ", dep.get_group_id(), dep.get_artifact_id());
                continue;
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
        let mut project = ProjectDep::try_from(&self.project).context(selected_version_err(
            self.project.get_group_id(),
            self.project.get_artifact_id(),
        ))?;
        project.base_url = url;
        project.cache_hit = cache_hit;

        if !resolved_earlier && project.packaging != "pom" {
            resolved.push(project);
        }
        Ok(())
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
pub fn resolve(
    dependencies: Vec<Project>,
    resolvers: Vec<Box<dyn Resolver>>,
) -> anyhow::Result<Vec<Project>> {
    // load labt.lock file directory
    let mut path: PathBuf = get_project_root()
        .context("Failed to get project root directory")?
        .clone();
    path.push(LOCK_FILE);

    // list of resolvers by their order of priority
    let resolvers = Rc::new(RefCell::new(resolvers));

    // load resolved dependencies from lock file
    let mut lock: LabtLock = if path.exists() {
        load_labt_lock()?
    } else {
        LabtLock::default()
    };
    let mut unresolved = vec![];

    // start a new spinner progress bar and add it to the global multi progress bar
    let spinner = Rc::new(RefCell::new(
        MULTI_PROGRESS_BAR.add(ProgressBar::new_spinner()),
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
        wrapper.build_tree(&mut lock.resolved, &mut unresolved)?;
        resolved_projects.push(wrapper.project);
    }
    // clear progressbar
    spinner.borrow().finish_and_clear();

    let mut file = File::create(path).context("Unable to open lock file")?;
    write_lock(&mut file, &lock)?;
    save_dependencies(&lock.resolved).context("Failed downloading saved dependencies")?;
    Ok(resolved_projects)
}
#[cfg(test)]
use pretty_assertions::assert_eq;

#[test]
fn check_base_url_conversion() {
    let base = String::from("https://maven.example.com/maven2/");
    let mut dep = ProjectDep {
        artifact_id: "labt".to_string(),
        group_id: "com.gitlab.labtool".to_string(),
        version: "6.9.0".to_string(),
        base_url: base.clone(),
        ..Default::default()
    };

    let expected = String::from("https://maven.example.com/maven2/com/gitlab/labtool/labt/6.9.0/");

    assert_eq!(expected, dep.get_root_url());

    dep.base_url = String::new();
    dep.set_base_url_from_root(expected);

    assert_eq!(dep.base_url, base);
}

#[test]
fn constraint_check_version_ranges() {
    let constraint = Constraint {
        min: Some((true, String::from("1.5.0"))),
        max: Some((true, String::from("5.8.0"))),
        exact: None,
        exclusions: Vec::new(),
    };

    assert!(constraint.within(&VersionRequirement::Unset).unwrap());

    assert!(constraint
        .within(&VersionRequirement::Soft(String::from("4.0")))
        .unwrap());

    assert!(!constraint
        .within(&VersionRequirement::Soft(String::from("1.0")))
        .unwrap());

    assert!(constraint
        .within(&"[1.5]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"[1.0]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"[1.5,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"(1.5,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"[1.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"(1.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,1.5)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"(,1.5]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"(,5.8)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"(,5.8]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"(,6.8]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"(1.5,6.8]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,1.5),(1.5,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,5.8),(5.8,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    // makes sense from logical standpoint where v5 is within limits
    assert!(constraint
        .within(&"(,5.0),(5.0)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    let constraint = Constraint {
        min: Some((false, String::from("1.5.0"))),
        max: Some((false, String::from("5.8.0"))),
        exact: Some(String::from("5.5")), // anything that is not this should fail
        exclusions: Vec::new(),
    };
    assert!(constraint.within(&VersionRequirement::Unset).unwrap());

    assert!(!constraint
        .within(&VersionRequirement::Soft(String::from("4.0")))
        .unwrap());

    assert!(constraint
        .within(&VersionRequirement::Soft(String::from("5.5")))
        .unwrap());

    assert!(!constraint
        .within(&VersionRequirement::Soft(String::from("1.0")))
        .unwrap());

    assert!(!constraint
        .within(&"[1.5]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(constraint
        .within(&"[5.5]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"[1.0]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"[1.5,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(1.5,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"[1.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(1.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,1.5)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,1.5]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,5.8)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,5.8]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,6.8]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(1.5,6.8]".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,1.5),(1.5,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"(,5.8),(5.8,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    // makes sense from logical standpoint where v5 is within limits
    assert!(!constraint
        .within(&"(,5.0),(5.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap());

    // Exclusions
    let mut c = Constraint::from(&constraint);
    c.exclusions.push((
        VersionRange::Gt(String::from("2.0")),
        VersionRange::Lt(String::from("3.0")),
    )); // 2.0>x<3.0

    assert!(!constraint
        .within(&"2.1".parse::<VersionRequirement>().unwrap())
        .unwrap());

    assert!(!constraint
        .within(&"[2.1]".parse::<VersionRequirement>().unwrap())
        .unwrap());
}

#[test]
fn constraint_contain_version_ranges() {
    let constraint = Constraint {
        min: Some((true, String::from("1.5.0"))),
        max: Some((true, String::from("5.8.0"))),
        exact: None,
        exclusions: Vec::new(),
    };

    // min exclusive
    let constraint2 = Constraint {
        min: Some((false, String::from("1.5.0"))),
        max: Some((true, String::from("5.8.0"))),
        exact: None,
        exclusions: Vec::new(),
    };
    // max exclusive
    let constraint3 = Constraint {
        min: Some((true, String::from("1.5.0"))),
        max: Some((false, String::from("5.8.0"))),
        exact: None,
        exclusions: Vec::new(),
    };

    // soft
    assert_eq!(
        constraint
            .contain(&"1.2".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        constraint
    );
    // unset
    assert_eq!(
        constraint
            .contain(&"".parse::<VersionRequirement>().unwrap())
            .unwrap(),
        constraint
    );

    // exact version
    let c = constraint
        .contain(&"[2.0]".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.exact, Some("2.0".to_string()));

    // exact version that is out of bounds
    let c = constraint.contain(&"[5.9]".parse::<VersionRequirement>().unwrap());
    assert!(c.is_err());

    let c = constraint.contain(&"[1.5]".parse::<VersionRequirement>().unwrap());
    assert!(c.is_ok());

    let c = constraint2.contain(&"[1.5]".parse::<VersionRequirement>().unwrap());
    assert!(c.is_err());

    let c = constraint3.contain(&"[5.8]".parse::<VersionRequirement>().unwrap());
    assert!(c.is_err());

    let c = constraint.contain(&"[5.5],[1.5]".parse::<VersionRequirement>().unwrap()); // an incorrect conflicting input
    assert!(c.is_err());

    // Exact was already set
    let mut c = Constraint::from(&constraint);
    c.exact = Some(String::from("4.0"));
    assert!(c
        .contain(&"[5.8]".parse::<VersionRequirement>().unwrap())
        .is_err());

    // Shrink the min max boundary
    let c = constraint
        .contain(&"[2.0, 3.0]".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("2.0"))));
    assert_eq!(c.max, Some((true, String::from("3.0"))));

    // Shrink the min max boundary
    let c = constraint
        .contain(&"(2.0, 3.0)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((false, String::from("2.0"))));
    assert_eq!(c.max, Some((false, String::from("3.0"))));

    let c = constraint
        .contain(&"[2.0, 6.0]".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("2.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));

    let c = constraint
        .contain(&"(2.0, 6.0)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((false, String::from("2.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));

    let c = constraint
        .contain(&"(,6.0]".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.5.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));

    let c = constraint
        .contain(&"(,6.0)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.5.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));
    // double move
    let c = constraint
        .contain(&"(,6.0),(,5.5.0)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.5.0"))));
    assert_eq!(c.max, Some((false, String::from("5.5.0"))));

    // moves the max bound by making them exclusive
    let c = constraint
        .contain(&"(,5.8)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.5.0"))));
    assert_eq!(c.max, Some((false, String::from("5.8"))));

    // Push the min up
    let c = constraint
        .contain(&"[2.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("2.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));

    let c = constraint
        .contain(&"(3.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((false, String::from("3.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));

    // moves the max bound by making them exclusive
    let c = constraint
        .contain(&"(1.5,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((false, String::from("1.5"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));
    // Double move
    let c = constraint
        .contain(&"(1.5,),(1.8,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((false, String::from("1.8"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));

    // Excludes
    let c = constraint
        .contain(&"(,2.0),(2.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.5.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));
    assert_eq!(
        c.exclusions,
        vec![(
            VersionRange::Ge(String::from("2.0")),
            VersionRange::Le(String::from("2.0"))
        )]
    );
    let c = constraint
        .contain(&"(,2.0],[2.0,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.5.0"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));
    assert_eq!(
        c.exclusions,
        vec![(
            VersionRange::Gt(String::from("2.0")),
            VersionRange::Lt(String::from("2.0"))
        )]
    );

    let c = constraint
        .contain(
            &"[1.8, 3.0],(,2.0),(2.0,)"
                .parse::<VersionRequirement>()
                .unwrap(),
        )
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.8"))));
    assert_eq!(c.max, Some((true, String::from("3.0"))));
    assert_eq!(
        c.exclusions,
        vec![(
            VersionRange::Ge(String::from("2.0")),
            VersionRange::Le(String::from("2.0"))
        )]
    );

    let c = constraint
        .contain(
            &"[1.8,),(,2.0),(2.0,)"
                .parse::<VersionRequirement>()
                .unwrap(),
        )
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.8"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));
    assert_eq!(
        c.exclusions,
        vec![(
            VersionRange::Ge(String::from("2.0")),
            VersionRange::Le(String::from("2.0"))
        )]
    );
    let c = constraint
        .contain(
            &"[1.8,),(,2.0),(2.0,)(,4.0),(5.0,)"
                .parse::<VersionRequirement>()
                .unwrap(),
        )
        .unwrap();

    assert_eq!(c.min, Some((true, String::from("1.8"))));
    assert_eq!(c.max, Some((true, String::from("5.8.0"))));
    assert_eq!(
        c.exclusions,
        vec![
            (
                VersionRange::Ge(String::from("2.0")),
                VersionRange::Le(String::from("2.0"))
            ),
            (
                VersionRange::Ge(String::from("4.0")),
                VersionRange::Le(String::from("5.0"))
            )
        ]
    );
    assert!(constraint
        .contain(&"[2.0],(,2.0),(2.0,)".parse::<VersionRequirement>().unwrap(),)
        .is_err());

    assert!(constraint
        .contain(&"[2.1],(,2.0),(3.0,)".parse::<VersionRequirement>().unwrap(),)
        .is_err());

    // set new min or max
    let constraint = Constraint::default();

    let c = constraint
        .contain(&"(5.9,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(
        c,
        Constraint {
            min: Some((false, "5.9".to_string())),
            ..Default::default()
        }
    );
    let c = constraint
        .contain(&"[5.9,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(
        c,
        Constraint {
            min: Some((true, "5.9".to_string())),
            ..Default::default()
        }
    );
    let c = constraint
        .contain(&"(,5.9)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(
        c,
        Constraint {
            max: Some((false, "5.9".to_string())),
            ..Default::default()
        }
    );
    let c = constraint
        .contain(
            &"(,5.9),(,5.6),(,5.1)"
                .parse::<VersionRequirement>()
                .unwrap(),
        )
        .unwrap();

    assert_eq!(
        c,
        Constraint {
            max: Some((false, "5.1".to_string())),
            ..Default::default()
        }
    );
    let c = constraint
        .contain(&"(,5.9]".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(
        c,
        Constraint {
            max: Some((true, "5.9".to_string())),
            ..Default::default()
        }
    );
    let c = constraint
        .contain(&"(,5.9),(5.9,)".parse::<VersionRequirement>().unwrap())
        .unwrap();

    assert_eq!(
        c,
        Constraint {
            exclusions: vec![(
                VersionRange::Ge("5.9".to_string()),
                VersionRange::Le("5.9".to_string())
            )],
            ..Default::default()
        }
    );
}

#[test]
fn constraint_within_range() {
    assert!(Constraint::within_range(
        &String::from("2.0"),
        &VersionRange::Ge(String::from("2.0")),
        &VersionRange::Le(String::from("2.0"))
    ));
    assert!(!Constraint::within_range(
        &String::from("2.0"),
        &VersionRange::Gt(String::from("2.0")),
        &VersionRange::Le(String::from("2.0"))
    ));
    assert!(!Constraint::within_range(
        &String::from("2.0"),
        &VersionRange::Ge(String::from("2.0")),
        &VersionRange::Lt(String::from("2.0"))
    ));
    assert!(!Constraint::within_range(
        &String::from("2.0"),
        &VersionRange::Gt(String::from("2.0")),
        &VersionRange::Lt(String::from("2.0"))
    ));

    assert!(!Constraint::within_range(
        &String::from("3.0"),
        &VersionRange::Ge(String::from("2.0")),
        &VersionRange::Le(String::from("2.0"))
    ));

    assert!(Constraint::within_range(
        &String::from("1.0"),
        &VersionRange::Lt(String::from("2.0")),
        &VersionRange::Le(String::from("3.0"))
    ));
    assert!(!Constraint::within_range(
        &String::from("1.0"),
        &VersionRange::Lt(String::from("2.0")),
        &VersionRange::Ge(String::from("3.0"))
    ));
    assert!(Constraint::within_range(
        &String::from("7.0"),
        &VersionRange::Gt(String::from("2.0")),
        &VersionRange::Ge(String::from("3.0"))
    ));
    assert!(!Constraint::within_range(
        &String::from("7.0"),
        &VersionRange::Lt(String::from("2.0")),
        &VersionRange::Ge(String::from("3.0"))
    ));
    assert!(!Constraint::within_range(
        &String::from("1.0"),
        &VersionRange::Gt(String::from("2.0")),
        &VersionRange::Ge(String::from("3.0"))
    ));
}

#[cfg(test)]
mod pom_faker {
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, BufWriter, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex},
        thread,
    };

    use regex::Regex;

    use crate::pom::Scope;

    struct MetadataEntry<'a> {
        latest: &'a str,
        release: &'a str,
        versions: Vec<&'a str>,
    }

    #[derive(Default, Debug, Clone)]
    pub struct ParentEntry {
        pub artifact_id: String,
        pub group_id: String,
        pub version: String,
    }

    #[derive(Default, Debug, Clone)]
    pub struct ProjectEntry {
        artifact_id: String,
        group_id: String,
        version: String,
        dependencies: Vec<ProjectEntry>,
        dependency_management: Vec<ProjectEntry>,
        scope: Scope,
        exclusions: Vec<(&'static str, &'static str)>,
        optional: bool,
        parent: Option<ParentEntry>,
    }

    impl ProjectEntry {
        pub fn new(group_id: &str, artifact_id: &str, version: &str) -> Self {
            Self {
                group_id: group_id.to_string(),
                artifact_id: artifact_id.to_string(),
                version: version.to_string(),
                optional: false,
                ..Default::default()
            }
        }
        pub fn get_id(&self) -> String {
            format!("{}:{}:{}", self.group_id, self.artifact_id, self.version)
        }
        pub fn add_dependency(mut self, dep: ProjectEntry) -> Self {
            self.dependencies.push(dep);
            self
        }
        pub fn add_dependency_management(mut self, dep: ProjectEntry) -> Self {
            self.dependency_management.push(dep);
            self
        }
        pub fn add_exclusion(mut self, group_id: &'static str, artifact_id: &'static str) -> Self {
            self.exclusions.push((group_id, artifact_id));
            self
        }
        pub fn set_parent(mut self, parent: ParentEntry) -> Self {
            self.parent = Some(parent);
            self
        }
        pub fn set_optional(mut self, value: bool) -> Self {
            self.optional = value;
            self
        }
    }

    static REPO: std::sync::LazyLock<
        HashMap<(&'static str, &'static str), MetadataEntry<'static>>,
    > = std::sync::LazyLock::new(init_metadata_repo);

    fn init_metadata_repo<'a>() -> HashMap<(&'a str, &'a str), MetadataEntry<'a>> {
        let mut metadata = HashMap::new();

        metadata.insert(
            ("com.example", "module-a"),
            MetadataEntry {
                latest: "1.5.0",
                release: "1.5.0",
                versions: vec!["1.0.0", "1.1.0", "1.2.0", "1.3.0", "1.4.0", "1.5.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-b"),
            MetadataEntry {
                latest: "2.1.0",
                release: "2.1.0",
                versions: vec![
                    "1.0.0", "1.1.0", "1.2.0", "1.3.0", "1.4.1", "1.5.0", "2.0.0", "2.1.0",
                ],
            },
        );

        metadata.insert(
            ("com.example", "module-c"),
            MetadataEntry {
                latest: "3.0.0",
                release: "3.0.0",
                versions: vec!["1.0.0", "2.0.0", "2.1.0", "2.2.0", "3.0.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-d"),
            MetadataEntry {
                latest: "1.2.0",
                release: "1.2.0",
                versions: vec!["1.0.0", "1.1.0", "1.2.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-e"),
            MetadataEntry {
                latest: "4.0.0",
                release: "4.0.0",
                versions: vec![
                    "1.0.0", "1.5.0", "1.6.0", "1.7.0", "2.0.0", "2.1.0", "2.2.0", "2.3.0",
                    "2.4.0", "3.0.0", "4.0.0",
                ],
            },
        );

        metadata.insert(
            ("com.example", "module-f"),
            MetadataEntry {
                latest: "1.1.0",
                release: "1.1.0",
                versions: vec!["1.0.0", "1.1.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-g"),
            MetadataEntry {
                latest: "1.1.0",
                release: "1.1.0",
                versions: vec!["1.0.0", "1.1.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-h"),
            MetadataEntry {
                latest: "2.0.0",
                release: "2.0.0",
                versions: vec!["1.0.0", "2.0.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-i"),
            MetadataEntry {
                latest: "3.0.0",
                release: "3.0.0",
                versions: vec!["1.0.0", "2.0.0", "3.0.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-j"),
            MetadataEntry {
                latest: "1.2.0",
                release: "1.2.0",
                versions: vec!["1.0.0", "1.1.0", "1.2.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-k"),
            MetadataEntry {
                latest: "2.1.0",
                release: "2.1.0",
                versions: vec!["1.0.0", "2.0.0", "2.1.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-l"),
            MetadataEntry {
                latest: "3.0.0",
                release: "3.0.0",
                versions: vec!["1.0.0", "2.0.0", "3.0.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-m"),
            MetadataEntry {
                latest: "1.1.0",
                release: "1.1.0",
                versions: vec!["1.0.0", "1.1.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-n"),
            MetadataEntry {
                latest: "2.0.0",
                release: "2.0.0",
                versions: vec!["1.0.0", "2.0.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-o"),
            MetadataEntry {
                latest: "1.2.0",
                release: "1.2.0",
                versions: vec!["1.0.0", "1.1.0", "1.2.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-p"),
            MetadataEntry {
                latest: "3.0.0",
                release: "3.0.0",
                versions: vec!["1.0.0", "2.0.0", "3.0.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-q"),
            MetadataEntry {
                latest: "2.1.0",
                release: "2.1.0",
                versions: vec!["1.0.0", "2.0.0", "2.1.0"],
            },
        );

        metadata.insert(
            ("com.example", "module-r"),
            MetadataEntry {
                latest: "1.1.0",
                release: "1.1.0",
                versions: vec!["1.0.0", "1.1.0"],
            },
        );

        metadata
    }

    type Repository = Arc<Mutex<HashMap<String, ProjectEntry>>>;

    pub struct PomServer {
        pub port: u16,
        pub repo: Repository,
        pub properties: Arc<Mutex<HashMap<String, String>>>,
    }

    impl PomServer {
        pub fn new() -> anyhow::Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let port = listener.local_addr()?.port();
            let repo = Arc::new(Mutex::new(HashMap::new()));
            let properties = Arc::new(Mutex::new(HashMap::new()));

            let r = Arc::clone(&repo);
            let p = Arc::clone(&properties);

            thread::spawn(move || {
                for stream in listener.incoming() {
                    let stream = stream.unwrap();
                    let mut buffer = [0; 512];
                    stream.peek(&mut buffer).unwrap();

                    if buffer.starts_with(b"CLOSE") {
                        break;
                    }
                    Self::handle_request(&stream, r.clone(), p.clone()).unwrap();
                }
            });

            Ok(Self {
                port,
                repo,
                properties,
            })
        }

        pub fn close(&self) {
            if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", self.port)) {
                stream.write_all(b"CLOSE").unwrap();
                stream.flush().unwrap();
            }
        }

        /// spawns a new thread to handle response of this request
        pub fn handle_request(
            stream: &TcpStream,
            repo: Repository,
            properties: Arc<Mutex<HashMap<String, String>>>,
        ) -> anyhow::Result<()> {
            let stream = stream.try_clone()?;

            thread::spawn(move || {
                let mut writer = BufWriter::new(stream.try_clone().unwrap());
                // get request method and url
                let mut reader = BufReader::new(stream);
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();

                let sections: Vec<&str> = header.split_whitespace().collect();
                let mut iter = sections.iter();
                let _method = iter.next().expect("Missing http method");
                let uri = iter.next().expect("Request missing its uri");
                let _version = iter.next().expect("Request missing its version");

                // we mostly just need method and uri. we dont need a body

                // break down the uri into correct segments using regex
                let repo_re = Regex::new(
                    r"^\/(?<group_id>.*)\/(?<artifact_id>.*)\/(?<version>.*)\/(.*)\.pom$",
                )
                .unwrap();

                let metadata_re =
                    Regex::new("^/(?<group_id>.*)/(?<artifact_id>.*)/maven-metadata.xml$").unwrap();

                if let Some(m) = repo_re.captures(uri) {
                    let group_id = m.name("group_id").unwrap().as_str().replace('/', ".");
                    let artifact_id = m.name("artifact_id").unwrap();
                    let version = m.name("version").unwrap();

                    if let Some(resp) = Self::create_pom(
                        repo,
                        group_id.as_str(),
                        artifact_id.as_str(),
                        version.as_str(),
                        properties,
                    ) {
                        writer
                            .write_all(Self::create_response(200, resp.as_str()).as_bytes())
                            .unwrap();
                    } else {
                        writer
                            .write_all(Self::create_response(404, "").as_bytes())
                            .unwrap();
                    }
                } else if let Some(m) = metadata_re.captures(uri) {
                    let group_id = m.name("group_id").unwrap().as_str().replace('/', ".");
                    let artifact_id = m.name("artifact_id").unwrap();

                    if let Some(resp) =
                        Self::create_metadata(group_id.as_str(), artifact_id.as_str())
                    {
                        writer
                            .write_all(Self::create_response(200, resp.as_str()).as_bytes())
                            .unwrap();
                    } else {
                        writer
                            .write_all(Self::create_response(404, "").as_bytes())
                            .unwrap();
                    }
                } else {
                    writer
                        .write_all(Self::create_response(404, "").as_bytes())
                        .unwrap();
                }
                writer.flush().unwrap();
            });

            Ok(())
        }

        fn create_response(code: u16, body: &str) -> String {
            let reason = match code {
                200 => "Ok",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "",
            };

            let mut resp = format!("HTTP/1.1 {} {} \r\n", code, reason);
            resp.push_str("\r\n");
            resp.push_str(body);

            resp
        }

        /// Responds with pom.xml if repo is available. Otherwise None is just 404
        fn create_pom(
            repo: Repository,
            group_id: &str,
            artifact_id: &str,
            version: &str,
            properties: Arc<Mutex<HashMap<String, String>>>,
        ) -> Option<String> {
            let proj = ProjectEntry::new(group_id, artifact_id, version);
            if let Ok(repo) = repo.lock() {
                let proj = if let Some(proj) = repo.get(&proj.get_id()) {
                    proj.clone()
                } else {
                    // try to gen from metadata
                    if let Some(meta) = REPO.get(&(group_id, artifact_id)) {
                        if meta.versions.contains(&version) {
                            ProjectEntry::new(group_id, artifact_id, version)
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                };

                let mut body = String::new();
                body.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>\n"#);
                body.push_str(r#"<project xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 https://maven.apache.org/xsd/maven-4.0.0.xsd" xmlns="http://maven.apache.org/POM/4.0.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">\n"#);
                body.push_str(r#" <modelVersion>4.0.0</modelVersion>\n"#);
                // add parent properties
                if let Some(parent) = proj.parent {
                    body.push_str("  <parent>\n");

                    if !parent.group_id.is_empty() {
                        body.push_str(&format!("    <groupId>{}</groupId>\n", parent.group_id));
                    }
                    if !parent.artifact_id.is_empty() {
                        body.push_str(&format!(
                            "    <artifactId>{}</artifactId>\n",
                            parent.artifact_id
                        ));
                    }
                    if !parent.version.is_empty() {
                        body.push_str(&format!("    <version>{}</version>\n", parent.version));
                    }
                    body.push_str("  </parent>\n");
                }

                if !proj.dependency_management.is_empty() {
                    body.push_str("  <dependencyManagement>\n");
                    body.push_str("    <dependencies>\n");
                    for dep in &proj.dependency_management {
                        body.push_str("     <dependency>\n");
                        body.push_str(&format!("      <groupId>{}</groupId>\n", dep.group_id));
                        body.push_str(&format!(
                            "      <artifactId>{}</artifactId>\n",
                            dep.artifact_id
                        ));
                        body.push_str(&format!("      <version>{}</version>\n", dep.version));
                        body.push_str(&format!("      <scope>{}</scope>\n", dep.scope));
                        if dep.optional {
                            body.push_str("      <optional>true</optional>\n");
                        }
                        if !dep.exclusions.is_empty() {
                            body.push_str("      <exclusions>\n");
                            for (group_id, artifact_id) in &dep.exclusions {
                                body.push_str("        <exclusion>\n");
                                body.push_str(&format!(
                                    "         <groupId>{}</groupId>\n",
                                    group_id
                                ));
                                body.push_str(&format!(
                                    "         <artifactId>{}</artifactId>\n",
                                    artifact_id
                                ));
                                body.push_str("        </exclusion>\n");
                            }
                            body.push_str("      </exclusions>\n");
                        }
                        body.push_str("     </dependency>\n");
                    }

                    body.push_str("    </dependencies>\n");
                    body.push_str("  </dependencyManagement>\n");
                }

                body.push_str(&format!(" <groupId>{}</groupId>\n", group_id));
                body.push_str(&format!(" <artifactId>{}</artifactId>\n", artifact_id));
                body.push_str(&format!(" <version>{}</version>\n", version));
                body.push_str(&format!(" <name>{}:{}</name>\n", group_id, artifact_id));
                // deal with passed properties
                let p = properties.lock().unwrap();
                if !p.is_empty() {
                    body.push_str("  <properties>\n");
                    for property in p.iter() {
                        body.push_str(&format!("<{0}>{1}</{0}>\n", property.0, property.1));
                    }
                    body.push_str("  </properties>\n");
                }
                drop(p);

                body.push_str("  <dependencies>\n");
                for dep in proj.dependencies {
                    body.push_str("    <dependency>\n");
                    body.push_str(&format!("      <groupId>{}</groupId>\n", dep.group_id));
                    body.push_str(&format!(
                        "      <artifactId>{}</artifactId>\n",
                        dep.artifact_id
                    ));
                    body.push_str(&format!("      <version>{}</version>\n", dep.version));
                    body.push_str(&format!("      <scope>{}</scope>\n", dep.scope));
                    if dep.optional {
                        body.push_str("      <optional>true</optional>\n");
                    }
                    if !dep.exclusions.is_empty() {
                        body.push_str("      <exclusions>\n");
                        for (group_id, artifact_id) in dep.exclusions {
                            body.push_str("        <exclusion>\n");
                            body.push_str(&format!("         <groupId>{}</groupId>\n", group_id));
                            body.push_str(&format!(
                                "         <artifactId>{}</artifactId>\n",
                                artifact_id
                            ));
                            body.push_str("        </exclusion>\n");
                        }
                        body.push_str("      </exclusions>\n");
                    }
                    body.push_str("    </dependency>\n");
                }
                body.push_str("  </dependencies>\n");
                body.push_str("</project>");
                Some(body)
            } else {
                None
            }
        }
        fn create_metadata(group_id: &str, artifact_id: &str) -> Option<String> {
            if let Some(meta) = REPO.get(&(group_id, artifact_id)) {
                let mut body = String::new();
                body.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>\n"#);
                body.push_str(r#"<metadata modelVersion="1.1.0">"#);
                body.push_str(format!("\n<groupId>{}</groupId>\n", group_id).as_str());
                body.push_str(format!("<artifactId>{}</artifactId>\n", artifact_id).as_str());
                body.push_str(format!("<version>{}</version>\n", meta.latest).as_str());
                body.push_str("<versioning>\n".to_string().as_str());
                body.push_str(format!("\t<latest>{}</latest>\n", meta.latest).as_str());
                body.push_str(format!("\t<release>{}</release>\n", meta.release).as_str());
                body.push_str("\t\t<versions>\n".to_string().as_str());
                for v in &meta.versions {
                    body.push_str(format!("\t\t  <version>{}</version>\n", v).as_str())
                }
                body.push_str("\t\t</versions>\n".to_string().as_str());
                body.push_str("\t</versioning>\n".to_string().as_str());
                body.push_str("</metadata>".to_string().as_str());

                Some(body)
            } else {
                None
            }
        }
        /// Adds a project to the list available packages
        pub fn add_project(&self, project: ProjectEntry) {
            if let Ok(mut repo) = self.repo.lock() {
                repo.insert(project.get_id(), project);
            }
        }
        /// Add a property
        pub fn add_property(&self, key: &str, value: &str) {
            if let Ok(mut prop) = self.properties.lock() {
                prop.insert(key.to_string(), value.to_string());
            }
        }
        pub fn get_port(&self) -> u16 {
            self.port
        }
    }

    impl Drop for PomServer {
        fn drop(&mut self) {
            self.close()
        }
    }
}

#[test]
fn some_test() {
    let server = pom_faker::PomServer::new().unwrap();
    server.add_project(
        pom_faker::ProjectEntry::new("com.example", "module-a", "1.0.0").add_dependency(
            pom_faker::ProjectEntry::new("com.example", "module-a", "1.0.0"),
        ),
    );
    let port = server.get_port();

    let base_url = format!("http://localhost:{port}");
    let res = reqwest::blocking::get(format!(
        "{0}/{1}/{2}/{3}/{2}-{3}.pom",
        base_url, "com/example", "module-a", "1.0.0"
    ))
    .unwrap();

    assert_eq!(res.status(), reqwest::StatusCode::OK);
}
#[cfg(test)]
mod test {
    use std::{cell::RefCell, rc::Rc};

    // use pretty_assertions::assert_eq;

    use pretty_assertions::assert_eq;

    use crate::{
        pom::{Project, VersionRange},
        submodules::{
            resolve::{pom_faker::ParentEntry, Constraint},
            resolvers::{NetResolver, Resolver},
        },
    };

    use super::{
        pom_faker::{PomServer, ProjectEntry},
        BuildTree, ProjectDep, ProjectWrapper,
    };

    pub fn resolve(
        dependencies: Vec<Project>,
        resolved: &mut Vec<ProjectDep>,
        resolvers: Rc<RefCell<Vec<Box<dyn Resolver>>>>,
    ) -> anyhow::Result<Vec<Project>> {
        let mut resolved_projects = Vec::new();
        let mut unresolved = Vec::new();

        for project in dependencies {
            // create a new project wrapper for dependency resolution
            let mut wrapper = ProjectWrapper::new(project.clone(), Rc::clone(&resolvers));
            // walk the dependency tree
            wrapper.build_tree(resolved, &mut unresolved)?;
            resolved_projects.push(wrapper.project);
        }
        Ok(resolved_projects)
    }

    pub fn create_resolver(
        port: u16,
    ) -> std::vec::Vec<std::boxed::Box<dyn crate::submodules::resolvers::Resolver>> {
        let base_url = format!("http://localhost:{port}");
        let resolver: Vec<Box<dyn Resolver>> =
            vec![Box::new(NetResolver::init("test", &base_url).unwrap())];
        resolver
    }

    #[test]
    fn resolver_single_dependency() {
        let server = PomServer::new().unwrap();
        server.add_project(ProjectEntry::new("com.example", "module-a", "1.0.0"));

        let port = server.get_port();

        let base_url = format!("http://localhost:{port}");
        let resolver: Vec<Box<dyn Resolver>> =
            vec![Box::new(NetResolver::init("test", &base_url).unwrap())];

        // now create some fake dependencies
        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let resolved = resolve(
            dependencies,
            &mut Vec::new(),
            Rc::new(RefCell::new(resolver)),
        )
        .unwrap();
        let mut iter = resolved.iter();
        let project = iter.next();
        assert_eq!(
            project,
            Some(&Project::new("com.example", "module-a", "1.0.0"))
        );
    }

    #[test]
    fn resolver_multiple_dependencies() {
        let server = PomServer::new().unwrap();
        server.add_project(ProjectEntry::new("com.example", "module-a", "1.0.0"));
        server.add_project(ProjectEntry::new("com.example", "module-b", "2.0.0"));

        let port = server.get_port();

        // now create some fake dependencies
        let dependencies = vec![
            Project::new("com.example", "module-a", "1.0.0"),
            Project::new("com.example", "module-b", "2.0.0"),
        ];

        let mut resolved = Vec::new();
        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 2);
        let mut iter = resolved.iter();
        let project = iter.next();
        assert_eq!(
            project,
            Some(&ProjectDep {
                group_id: "com.example".to_string(),
                artifact_id: "module-a".to_string(),
                version: "1.0.0".to_string(),
                ..Default::default()
            })
        );
        let project = iter.next();
        assert_eq!(
            project,
            Some(&ProjectDep {
                group_id: "com.example".to_string(),
                artifact_id: "module-b".to_string(),
                version: "2.0.0".to_string(),
                ..Default::default()
            })
        );
    }
    #[test]
    fn direct_version_conflict() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(ProjectEntry::new("com.example", "module-a", "1.0.0"));
        server.add_project(ProjectEntry::new("com.example", "module-a", "2.0.0"));

        // Project declares dependencies on module-a:1.0 and module-a:2.0 directly.
        let dependencies = vec![
            Project::new("com.example", "module-a", "1.0.0"),
            Project::new("com.example", "module-a", "2.0.0"),
        ];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();

        // Expected: only 1  choosed with highest version
        assert_eq!(resolved.len(), 1);
        let module_a = &resolved[0];
        assert_eq!(module_a.group_id, "com.example".to_string());
        assert_eq!(module_a.artifact_id, "module-a".to_string());
        assert_eq!(module_a.version, "2.0.0".to_string());
    }

    #[test]
    fn transitive_version_conflict() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        let lib_a1 = ProjectEntry::new("com.example", "module-a", "1.0.0");
        let lib_a2 = ProjectEntry::new("com.example", "module-a", "2.0.0");
        server.add_project(lib_a1.clone());
        server.add_project(lib_a2.clone());

        server.add_project(
            ProjectEntry::new("com.example", "module-b", "1.0.0").add_dependency(lib_a1),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "1.0.0").add_dependency(lib_a2),
        );

        // module-b and c declares dependencies on module-a:1.0 and module-a:2.0 respectively.
        let dependencies = vec![
            Project::new("com.example", "module-b", "1.0.0"),
            Project::new("com.example", "module-c", "1.0.0"),
        ];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();

        // Expected: only 1  choosed with highest version
        assert_eq!(resolved.len(), 3);
        assert_eq!(
            resolved[0],
            ProjectDep {
                group_id: "com.example".to_string(),
                artifact_id: "module-a".to_string(),
                version: "2.0.0".to_string(),
                ..Default::default()
            }
        );
        assert_eq!(
            resolved[1],
            ProjectDep {
                group_id: "com.example".to_string(),
                artifact_id: "module-b".to_string(),
                version: "1.0.0".to_string(),
                ..Default::default()
            }
        );
        assert_eq!(
            resolved[2],
            ProjectDep {
                group_id: "com.example".to_string(),
                artifact_id: "module-c".to_string(),
                version: "1.0.0".to_string(),
                ..Default::default()
            }
        );
    }
    /// Test case: Overlapping Version Ranges
    ///
    /// This test checks the behavior of the resolver when two dependencies specify overlapping,
    /// but not identical version ranges for the same module. The expected behavior is that the
    /// resolver selects a version that satisfies both ranges.
    ///
    /// Setup:
    /// - `module-a` requires `module-b` with a version range of `[1.0, 2.0)`.
    /// - `module-c` requires `module-b` with a version range of `[1.5, 2.1)`.
    ///
    /// Expected Result:
    /// The resolver should successfully resolve a version of `module-b` that falls within both
    /// ranges. In this case, `module-b:1.5` should be selected as it is the intersection of the
    /// two version ranges.
    #[test]
    fn overlapping_version_range() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        let lib_b1 = ProjectEntry::new("com.example", "module-b", "[1.0, 2.0)");
        let lib_b2 = ProjectEntry::new("com.example", "module-b", "[1.5, 2.1)");

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0").add_dependency(lib_b1),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "1.0.0").add_dependency(lib_b2),
        );

        // Project declares dependencies on module-a:1.0 and module-a:2.0 directly.
        let dependencies = vec![
            Project::new("com.example", "module-a", "1.0.0"),
            Project::new("com.example", "module-c", "1.0.0"),
        ];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();

        // Expected: only module b choosed within version ranges
        assert_eq!(resolved.len(), 3);
        let module_b = &resolved[0];
        assert_eq!(module_b.group_id, String::from("com.example"));
        assert_eq!(module_b.artifact_id, String::from("module-b"));
        assert_eq!(
            module_b.constraints,
            Some(Constraint {
                min: Some((true, String::from("1.5"))),
                max: Some((false, String::from("2.0"))),
                ..Default::default()
            })
        );
        // The selected version
        assert_eq!(module_b.version, String::from("1.5.0"));
    }

    /// Test case: Non-overlapping Version Ranges
    ///
    /// This test checks how the resolver handles a situation where two dependencies specify
    /// non-overlapping version ranges for the same module. The expected behavior is that the
    /// resolver should detect the conflict and either throw an error or fail the resolution.
    ///
    /// Setup:
    /// - `module-a` requires `module-e` with a version range of `[1.0, 1.5)`.
    /// - `module-b` requires `module-e` with a version range of `[2.0, 2.5)`.
    ///
    /// Expected Result:
    /// The resolver should detect that there is no version of `module-e` that satisfies both
    /// version ranges and report a conflict.
    #[test]
    fn non_overlapping_version_range() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        let lib_e1 = ProjectEntry::new("com.example", "module-e", "[1.0, 1.5)");
        let lib_e2 = ProjectEntry::new("com.example", "module-e", "[2.0, 2.5)");

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0").add_dependency(lib_e1),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-b", "1.0.0").add_dependency(lib_e2),
        );

        let dependencies = vec![
            Project::new("com.example", "module-a", "1.0.0"),
            Project::new("com.example", "module-b", "1.0.0"),
        ];

        let mut resolved = Vec::new();

        // This is a hard version conflict
        assert!(resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .is_err()); // Maybe this error was a net related error and this test will be incorrect
    }
    /// Test case: Exclusion Ignored
    ///
    /// This test verifies the resolver's behavior when a module excludes a specific version of a
    /// transitive dependency, but another module forces its inclusion. The expected behavior is
    /// that the resolver respects the exclusion and does not include the excluded version.
    ///
    /// Setup:
    /// - `module-a` requires `module-d` and excludes `module-b:1.5`.
    /// - `module-c` requires `module-b:1.5`.
    ///
    /// Expected Result:
    /// The resolver should detect the conflict and ensure that `module-i:1.5` is not included
    /// due to the exclusion specified by `module-a`. The resolution should fail or find an
    /// alternative version if possible.
    #[test]
    fn exclusion_ignored() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-d", "[1.0, 2.0)"))
                // exclude module b
                .add_dependency(ProjectEntry::new(
                    "com.example",
                    "module-b",
                    "(,1.5.0),(1.5.0,)",
                )),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "1.0.0")
                // declare an excluded version
                .add_dependency(ProjectEntry::new("com.example", "module-b", "1.5.0")),
        );

        let dependencies = vec![
            Project::new("com.example", "module-a", "1.0.0"),
            Project::new("com.example", "module-c", "1.0.0"),
        ];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();

        // Expected: only module b choosed within version ranges
        assert_eq!(resolved.len(), 4);
        let module_b = &resolved[1];
        assert_eq!(module_b.group_id, String::from("com.example"));
        assert_eq!(module_b.artifact_id, String::from("module-b"));
        assert_eq!(
            module_b.constraints,
            Some(Constraint {
                exclusions: vec![(
                    VersionRange::Ge("1.5.0".to_string()),
                    VersionRange::Le("1.5.0".to_string()),
                )],
                ..Default::default()
            })
        );
        // The selected version
        assert_eq!(module_b.version, String::from("2.1.0"));
        drop(server);

        // What if module-c forces the version
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-d", "[1.0, 2.0)"))
                // exclude module b
                .add_dependency(ProjectEntry::new(
                    "com.example",
                    "module-b",
                    "(,1.5.0),(1.5.0,)",
                )),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "1.0.0")
                // declare an excluded version but this time enforce it
                .add_dependency(ProjectEntry::new("com.example", "module-b", "[1.5.0]")),
        );

        let dependencies = vec![
            Project::new("com.example", "module-a", "1.0.0"),
            Project::new("com.example", "module-c", "1.0.0"),
        ];

        let mut resolved = Vec::new();

        assert!(resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .is_err());
    }

    #[test]
    pub fn circular_dependency() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-b", "1.0.0")),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-b", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-c", "1.0.0"))
                .add_dependency(ProjectEntry::new("com.example", "module-a", "1.0.0")),
        );
        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 3);
        drop(server);

        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-b", "1.0.0")),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-b", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-c", "1.0.0")),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-a", "1.0.0")),
        );

        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 3);

        // A circular dependency with different versions
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-b", "1.0.0")),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-b", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-c", "1.0.0")),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-a", "1.2.0")),
        );

        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 3);
    }

    /// Test case: Dependency Exclusion
    ///
    /// This test verifies that when a module declares an exclusion for one of its transitive
    /// dependencies, the resolver respects that exclusion and does not include the excluded
    /// module in the resolved dependencies.
    ///
    /// Setup:
    /// - `module-a` depends on `module-b`.
    /// - `module-b` excludes `module-a` from its dependencies.
    ///
    /// Expected Result:
    /// The resolver should resolve `module-a` and `module-b` but exclude `module-c` due to the
    /// explicit exclusion declared by `module-b`.
    #[test]
    pub fn dependency_exclusion() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0").add_dependency(
                ProjectEntry::new("com.example", "module-b", "1.0.0")
                    .add_exclusion("com.example", "module-c"),
            ),
        );
        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 2);
        drop(server);
    }
    /// Test case: Dynamic Version Selection (Latest)
    ///
    /// This test verifies that when a module specifies a dynamic version like `LATEST`, the
    /// resolver correctly selects the highest available version.
    ///
    /// Setup:
    /// - `module-a` depends on `module-b` with version set to `LATEST`.
    ///
    /// Expected Result:
    /// The resolver should select `module-b:2.1.0` as it is the latest available version.
    #[test]
    pub fn dynamic_version_selection_latest() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-b", "LATEST")),
        );
        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 2);

        let module_b = &resolved[0];
        assert_eq!(module_b.group_id, String::from("com.example"));
        assert_eq!(module_b.artifact_id, String::from("module-b"));
        assert_eq!(module_b.version, String::from("2.1.0"));
        drop(server);
    }
    #[test]
    pub fn dynamic_version_selection_release() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-b", "RELEASE")),
        );
        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 2);

        let module_b = &resolved[0];
        assert_eq!(module_b.group_id, String::from("com.example"));
        assert_eq!(module_b.artifact_id, String::from("module-b"));
        assert_eq!(module_b.version, String::from("2.1.0"));
        drop(server);
    }
    #[test]
    pub fn properties_check_if_parsed() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-b", "RELEASE")),
        );
        server.add_property("my_version", "5.0.0");
        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 2);

        let module_b = &resolved[0];
        assert_eq!(module_b.group_id, String::from("com.example"));
        assert_eq!(module_b.artifact_id, String::from("module-b"));
        assert_eq!(module_b.version, String::from("2.1.0"));
        drop(server);
    }
    #[test]
    pub fn parent_pom() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-b", "RELEASE"))
                .add_dependency(ProjectEntry::new("com.example", "module-e", ""))
                .set_parent(ParentEntry {
                    artifact_id: String::from("module-c"),
                    group_id: String::from("com.example"),
                    version: String::from("3.0.0"),
                }),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "3.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-d", "RELEASE"))
                .add_dependency_management(ProjectEntry::new("com.example", "module-e", "2.0.0")),
        );

        let dependencies = vec![Project::new("com.example", "module-a", "1.0.0")];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 5);

        let module_d = &resolved[0];
        assert_eq!(module_d.group_id, String::from("com.example"));
        assert_eq!(module_d.artifact_id, String::from("module-d"));
        assert_eq!(module_d.version, String::from("1.2.0"));

        let module_c = &resolved[1];
        assert_eq!(module_c.group_id, String::from("com.example"));
        assert_eq!(module_c.artifact_id, String::from("module-c"));
        assert_eq!(module_c.version, String::from("3.0.0"));

        let module_b = &resolved[2];
        assert_eq!(module_b.group_id, String::from("com.example"));
        assert_eq!(module_b.artifact_id, String::from("module-b"));
        assert_eq!(module_b.version, String::from("2.1.0"));

        let module_b = &resolved[3];
        assert_eq!(module_b.group_id, String::from("com.example"));
        assert_eq!(module_b.artifact_id, String::from("module-e"));
        assert_eq!(module_b.version, String::from("2.0.0"));

        let module_a = &resolved[4];
        assert_eq!(module_a.group_id, String::from("com.example"));
        assert_eq!(module_a.artifact_id, String::from("module-a"));
        assert_eq!(module_a.version, String::from("1.0.0"));

        drop(server);
    }
    #[test]
    pub fn optional_dependency() {
        let server = PomServer::new().unwrap();
        let port = server.get_port();

        server.add_project(
            ProjectEntry::new("com.example", "module-a", "1.0.0").add_dependency(
                ProjectEntry::new("com.example", "module-b", "RELEASE").set_optional(true),
            ),
        );
        server.add_project(
            ProjectEntry::new("com.example", "module-c", "3.0.0")
                .add_dependency(ProjectEntry::new("com.example", "module-d", "RELEASE")),
        );

        let dependencies = vec![
            Project::new("com.example", "module-a", "1.0.0"),
            Project::new("com.example", "module-c", "3.0.0"),
        ];

        let mut resolved = Vec::new();

        resolve(
            dependencies,
            &mut resolved,
            Rc::new(RefCell::new(create_resolver(port))),
        )
        .unwrap();
        assert_eq!(resolved.len(), 3);
        let module_a = &resolved[0];
        assert_eq!(module_a.group_id, String::from("com.example"));
        assert_eq!(module_a.artifact_id, String::from("module-a"));
        assert_eq!(module_a.version, String::from("1.0.0"));

        let module_d = &resolved[1];
        assert_eq!(module_d.group_id, String::from("com.example"));
        assert_eq!(module_d.artifact_id, String::from("module-d"));
        assert_eq!(module_d.version, String::from("1.2.0"));

        let module_c = &resolved[2];
        assert_eq!(module_c.group_id, String::from("com.example"));
        assert_eq!(module_c.artifact_id, String::from("module-c"));
        assert_eq!(module_c.version, String::from("3.0.0"));
    }
}
