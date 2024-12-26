use std::fs;
use std::fs::create_dir;
use std::fs::create_dir_all;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use anyhow::anyhow;
use anyhow::Context;
use labt_proc_macro::labt_lua;
use mlua::Lua;

use crate::submodules::build::is_file_newer;

use super::MluaAnyhowWrapper;

/// creates the directory specified
/// Returns en error if obtaining the project root directory fails or
/// creating the directory fails
#[labt_lua]
fn mkdir(_lua: &Lua, path: String) {
    let path = PathBuf::from(path);
    let path = if path.is_relative() {
        // if path is relative, then build from project root
        let mut root = crate::get_project_root()
            .context("Failed to get project root directory")
            .map_err(MluaAnyhowWrapper::external)?
            .clone();
        root.push(path);
        root
    } else {
        path
    };

    create_dir(&path)
        .context(format!("Failed to create directory {:?}", path))
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(())
}

pub fn copy_recursively(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let filetype = entry.file_type()?;
        if filetype.is_dir() {
            copy_recursively(&entry.path(), &dest.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dest.join(entry.file_name()))?;
        }
    }
    Ok(())
}
/// Copies a file or directory from the source path to the destination path.
///
/// This function supports both file and directory copying. If the source is a directory,
/// the `recursive` parameter must be set to `true` to enable recursive copying of its contents.
/// If `recursive` is `false` and the source is a directory, an error will be returned.
///
/// If the destination path is a directory, the source file's name will be appended to the destination path.
/// If the source path is relative, it will be resolved against the project root directory.
///
/// # Parameters
///
/// - `lua`: A reference to the Lua state, used for error handling and context.
/// - `src`: A string representing the source file or directory path.
/// - `dest`: A string representing the destination file or directory path.
/// - `recursive`: An optional boolean indicating whether to enable recursive copying. Defaults to `false`.
///
/// # Errors
///
/// This function will return an error if:
/// - The source path does not exist.
/// - The destination path cannot be created.
/// - An attempt is made to copy a directory without enabling recursive mode.
/// - Any I/O operation fails during the copy process.
///
/// # Example
///
/// ```rust
/// copy("path/to/source.txt", "path/to/destination.txt", None)?;
/// copy("path/to/source_dir", "path/to/destination_dir", Some(true))?;
/// ```
///
#[labt_lua]
fn copy(_lua: &Lua, (src, dest, recursive): (String, String, Option<bool>)) {
    let src_path = PathBuf::from(src);
    let src_path = if src_path.is_relative() {
        // if path is relative, then build from project root
        let mut root = crate::get_project_root()
            .context("Failed to get project root directory")
            .map_err(MluaAnyhowWrapper::external)?
            .clone();
        root.push(src_path);
        root
    } else {
        src_path
    };
    let dest_path = PathBuf::from(dest);
    let mut dest_path = if dest_path.is_relative() {
        // if path is relative, then build from project root
        let mut root = crate::get_project_root()
            .context("Failed to get project root directory")
            .map_err(MluaAnyhowWrapper::external)?
            .clone();
        root.push(dest_path);
        root
    } else {
        dest_path
    };

    let recursive = recursive.unwrap_or(false);

    // recursive mode must be enabled to copy folders
    if src_path.is_dir() {
        if !recursive {
            Err(anyhow!("Attempt to copy directory without recursive mode"))
                .map_err(MluaAnyhowWrapper::external)?;
        }

        copy_recursively(&src_path, &dest_path)
            .context("Failed to copy folder recursively")
            .map_err(MluaAnyhowWrapper::external)?;
    } else {
        // if dest path is a directory, then append the target file name to the end
        if dest_path.is_dir() {
            let name = src_path
                .file_name()
                .context("Unable to obtain the source file name")
                .map_err(MluaAnyhowWrapper::external)?;
            dest_path.push(name);
        }
        fs::copy(src_path, dest_path)
            .context("Failed to copy file to destination")
            .map_err(MluaAnyhowWrapper::external)?;
    }

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
            .context("Failed to get project root directory")
            .map_err(MluaAnyhowWrapper::external)?
            .clone();
        root.push(path);
        root
    } else {
        path
    };

    create_dir_all(&path)
        .context(format!("Failed to create directory {:?}", path))
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(())
}

/// Returns true if file exists and false if does not exist.
/// if the file/dir in question cannot be verified to exist or not exist due
/// to file system related errors, It returns the error instead.
#[labt_lua]
fn exists(_lua: &Lua, path: String) {
    let path = PathBuf::from(path);
    let exists = path
        .try_exists()
        .context("Failed to test if file exists")
        .map_err(MluaAnyhowWrapper::external)?;
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
    // check if path is relative
    let path: PathBuf = PathBuf::from(&pattern);
    let pattern = if path.is_relative() {
        let mut root = crate::get_project_root()
            .context("Failed to get project root directory")
            .map_err(MluaAnyhowWrapper::external)?
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
        .context("Failed to parse glob pattern")
        .map_err(MluaAnyhowWrapper::external)?
        .filter_map(|p| match p {
            Ok(path) => path.to_str().map(|n| n.to_string()),
            Err(_) => None,
        })
        .collect();
    Ok(globals)
}

/// Returns true if:
///   - file a is newer than file b
///   - file b does not exist
///
/// Returns false if:
///   - file a does not exist (Technically b should be nwer if a is missing)
///
/// # Errors
///
/// Returns an error if we fail to get the metadata of the file
#[labt_lua]
fn is_newer(_lua: &Lua, (a, b): (String, String)) {
    let path_a = PathBuf::from(a);
    let path_b = PathBuf::from(b);

    let result = is_file_newer(&path_a, &path_b)
        .context("Failed to compare files a and b")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(result)
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
    is_newer(lua, &table)?;
    copy(lua, &table)?;

    lua.globals().set("fs", table)?;

    Ok(())
}
