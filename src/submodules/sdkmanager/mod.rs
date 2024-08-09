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
