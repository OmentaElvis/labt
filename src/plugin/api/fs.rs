use std::fs::create_dir;
use std::fs::create_dir_all;
use std::path::PathBuf;

use labt_proc_macro::labt_lua;
use mlua::Lua;

/// creates the directory specified
/// Returns en error if obtaining the project root directory fails or
/// creating the directory fails
#[labt_lua]
fn mkdir(_lua: &Lua, path: String) {
    let path = PathBuf::from(path);
    let path = if path.is_relative() {
        // if path is relative, then build from project root
        let mut root = crate::get_project_root()
            .map_err(mlua::Error::external)?
            .clone();
        root.push(path);
        root
    } else {
        path
    };

    create_dir(path).map_err(mlua::Error::external)?;

    Ok(())
}
/// creates the directory specified and all the parent directories if missing
/// Returns en error if obtaining the project root directory fails or
/// creating the directory fails
#[labt_lua]
fn mkdir_all(_lua: &Lua, path: String) {
    let path = PathBuf::from(path);
    let path = if path.is_relative() {
        // if path is relative, then build from project root
        let mut root = crate::get_project_root()
            .map_err(mlua::Error::external)?
            .clone();
        root.push(path);
        root
    } else {
        path
    };

    create_dir_all(path).map_err(mlua::Error::external)?;

    Ok(())
}

/// Returns true if file exists and false if does not exist.
/// if the file/dir in question cannot be verified to exist or not exist due
/// to file system related errors, It returns the error instead.
#[labt_lua]
fn exists(_lua: &Lua, path: String) {
    let path = PathBuf::from(path);
    let exists = path.try_exists().map_err(mlua::Error::external)?;
    Ok(exists)
}

/// Returns all files that match a globbing pattern. It returns only files that are
/// readable (did not return IO errors when trying to list them) and files whose path
/// string representation is a valid unicode.
/// Returns an error if:
/// - failed to parse the globbing pattern;
/// - Failed to get the project root for relative paths
/// - Failed to convert project root + glob pattern into unicode
#[labt_lua]
fn glob(_lua: &Lua, pattern: String) {
    use mlua::ErrorContext;
    // check if path is relative
    let path: PathBuf = PathBuf::from(&pattern);
    let pattern = if path.is_relative() {
        let mut root = crate::get_project_root()
            .map_err(mlua::Error::external)
            .context("Failed to get project root directory")?
            .clone();
        root.push(path);
        if let Some(pattern) = root.to_str() {
            pattern.to_string()
        } else {
            return Err(mlua::Error::runtime(
                "Failed to convert pattern to unicode format",
            ));
        }
    } else {
        pattern
    };

    let globals: Vec<String> = glob::glob(pattern.as_str())
        .map_err(mlua::Error::external)?
        .filter_map(|p| match p {
            Ok(path) => path.to_str().map(|n| n.to_string()),
            Err(_) => None,
        })
        .collect();
    Ok(globals)
}

/// Generates fs table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to fs table fails
/// or the underlying lua operations return errors.
pub fn load_fs_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;

    mkdir(lua, &table)?;
    mkdir_all(lua, &table)?;
    exists(lua, &table)?;
    glob(lua, &table)?;

    lua.globals().set("fs", table)?;

    Ok(())
}
