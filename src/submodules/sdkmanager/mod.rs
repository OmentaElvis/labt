pub mod filters;
pub mod installed_list;

pub use installed_list::write_installed_list;
pub use installed_list::InstalledPackage;

use crate::config::repository::ChannelType;
use crate::config::repository::Revision;

/// Creates a package id based on package path, version and channel
pub trait ToId {
    fn create_id(&self) -> (&String, &Revision, &ChannelType);
    fn to_id(&self) -> String {
        let (path, version, channel) = self.create_id();
        format!("{}:{}:{}", path, version, channel)
    }
}
/// Creates a package id based on package repo, path, version and channel
pub trait ToIdLong {
    fn create_id(&self) -> (&String, &String, &Revision, &ChannelType);
    fn to_id_long(&self) -> String {
        let (repo, path, version, channel) = self.create_id();
        format!("{}:{}:{}:{}", repo, path, version, channel)
    }
}
