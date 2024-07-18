use std::collections::HashSet;
use std::rc::Rc;

use fuzzy_matcher::clangd::fuzzy_match;

use crate::config::repository::ChannelType;
use crate::config::repository::RemotePackage;
use crate::config::repository::RepositoryXml;
use crate::submodules::sdk::InstalledPackage;

use super::installed_list::InstalledList;

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum SdkFilters {
    /// Filter by display name
    Name(String),
    /// Filter by version
    Version(String),
    /// Select only packages that matches this obsolete state
    Obsolete(bool),
    /// Filter by installed packages
    Installed,
}

#[derive(Default)]
pub struct FilteredPackages {
    /// The original repo to filter from
    pub repo: Rc<RepositoryXml>,
    // After several hours fighting the compiler, just resolved to clone the entries,
    // There is no performace requirement now at this module
    // FIXME should use Vec<&RemotePackage> to prevent clonning of entrues
    /// Filtered backage list if filters are activated
    pub packages: Vec<RemotePackage>,
    /// List of filters to apply
    pub filters: Vec<SdkFilters>,
    /// These are singleton filters applied to all entries
    pub single_filters: HashSet<SdkFilters>,

    pub installed: InstalledList,
    /// The channel to show packages for. If set to None all channels are shown
    pub channel: Option<ChannelType>,
}

impl FilteredPackages {
    pub fn new(repo: Rc<RepositoryXml>, installed: InstalledList) -> Self {
        Self {
            repo,
            installed,
            packages: Vec::new(),
            filters: Vec::new(),
            single_filters: HashSet::new(),
            channel: None,
        }
    }
    /// Adds filter to the list of availabke filters
    pub fn push_filter(&mut self, filter: SdkFilters) {
        self.filters.push(filter);
    }
    // /// Enables a particular filter. Returns true if operation was successful
    // pub fn enable_filter(&mut self, index: usize) -> bool {
    //     if let Some(filter) = self.filters.get_mut(index) {
    //         filter.0 = true;
    //         return true;
    //     }
    //     false
    // }
    // /// Disables a particular filter. Returns true if operation was successful
    // pub fn disable_filter(&mut self, index: usize) -> bool {
    //     if let Some(filter) = self.filters.get_mut(index) {
    //         filter.0 = false;
    //         return true;
    //     }
    //     false
    // }
    /// Adds a singleton filter. Singleton filters are "AND"ed together
    pub fn insert_singleton_filter(&mut self, filter: SdkFilters) {
        self.single_filters.insert(filter);
    }
    /// Removes a singleton filter
    pub fn remove_singleton_filter(&mut self, filter: &SdkFilters) -> bool {
        self.single_filters.remove(filter)
    }
    /// removes and reteurns the last filter
    pub fn pop_filter(&mut self) -> Option<SdkFilters> {
        self.filters.pop()
    }

    pub fn set_channel(&mut self, channel: Option<ChannelType>) {
        self.channel = channel;
    }
    pub fn get_channel(&self) -> &Option<ChannelType> {
        &self.channel
    }
    /// Returns true if there are filters available
    pub fn has_filters(&self) -> bool {
        !self.filters.is_empty()
    }

    /// Applies the filters and saves the filtered packages for future
    /// references
    /// returns the number of entries collected
    pub fn apply(&mut self) -> usize {
        if self.filters.is_empty() && self.single_filters.is_empty() {
            // return the count to the original array
            self.repo.get_remote_packages().len()
        } else {
            let installed_hash = self.installed.get_hash_map();
            let mut ranked: Vec<(i64, &RemotePackage)> = self
                .repo
                .get_remote_packages()
                .iter()
                .filter(|p| {
                    for filter in self.single_filters.iter() {
                        match filter {
                            SdkFilters::Installed => {
                                if let Some(channel) =
                                    self.repo.get_channels().get(p.get_channel_ref())
                                {
                                    // short circuit for installed
                                    if !installed_hash.contains_key(
                                        &InstalledPackage::new(
                                            p.get_path().clone(),
                                            p.get_revision().clone(),
                                            channel.clone(),
                                        )
                                        .to_id(),
                                    ) {
                                        return false;
                                    }
                                } else {
                                    return false;
                                }
                            }
                            SdkFilters::Obsolete(obsolete) => {
                                // short circuit for obsolete
                                if p.is_obsolete() != *obsolete {
                                    return false;
                                }
                            }
                            _ => {}
                        }
                    }
                    // apply channel filters
                    if let Some(channel) = &self.channel {
                        if let Some(c) = self.repo.get_channels().get(p.get_channel_ref()) {
                            if channel != c {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }

                    true
                })
                .filter_map(|p| {
                    if self.filters.is_empty() {
                        return Some((1, p));
                    }
                    for filter in &self.filters {
                        match filter {
                            SdkFilters::Name(name) => {
                                if let Some(rank) = fuzzy_match(p.get_display_name(), name) {
                                    return Some((rank, p));
                                }
                                if let Some(rank) = fuzzy_match(p.get_path(), name) {
                                    return Some((rank, p));
                                }
                            }
                            SdkFilters::Version(version) => {
                                if let Some(rank) =
                                    fuzzy_match(&p.get_revision().to_string(), version)
                                {
                                    return Some((rank, p));
                                }
                            }
                            // obsolete and installed should undergo another filter
                            _ => {}
                        }
                    }
                    None
                })
                .collect();
            ranked.sort_unstable_by_key(|p| p.0);
            self.packages = ranked.iter().rev().map(|m| m.1.to_owned()).collect();
            self.packages.len()
        }
    }

    /// rerurns the package list. If no filters are available the original
    /// package list is returned, otherwise returns the "applied" filtering.
    /// Note that it does not apply filters set so you must call `apply` before reading this
    pub fn get_packages(&self) -> &Vec<RemotePackage> {
        if !self.filters.is_empty() || !self.single_filters.is_empty() {
            &self.packages
        } else {
            self.repo.get_remote_packages()
        }
    }
}
