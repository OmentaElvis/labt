use std::{
    fmt::Display,
    fs::{create_dir_all, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use anyhow::Context;
use labt_proc_macro::labt_lua;
use mlua::{FromLua, Lua, Table};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::plugin::api::MluaAnyhowWrapper;

struct ZipEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    alignment: Option<u16>,
    no_compress: Option<bool>,
}

impl ZipEntry {
    pub fn new(name: String, path: PathBuf, is_dir: bool) -> Self {
        Self {
            name,
            path,
            is_dir,
            alignment: None,
            no_compress: None,
        }
    }
}

impl Display for ZipEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at {} with type {}",
            self.name,
            self.path.to_string_lossy(),
            if self.is_dir { "directory" } else { "file" }
        )
    }
}

impl FromLua<'_> for ZipEntry {
    fn from_lua(value: mlua::prelude::LuaValue<'_>, _lua: &Lua) -> mlua::prelude::LuaResult<Self> {
        if !value.is_table() {
            return Err(mlua::Error::RuntimeError(format!(
                "Expected table but found {}",
                value.type_name()
            )));
        }
        let table = value.as_table().unwrap();
        let name: String = table.get("name")?;
        let path: String = table.get("path")?;
        let is_dir: bool = table.get("is_dir")?;
        let alignment: Option<u16> = table.get("alignment")?;
        let no_compress: Option<bool> = table.get("no_compress")?;

        let mut entry = ZipEntry::new(name, PathBuf::from(path), is_dir);
        entry.alignment = alignment;
        entry.no_compress = no_compress;

        Ok(entry)
    }
}

/// Commits all the files onto the zip output file
/// # Errors
/// Returns an error if:
///  - self is not a valid zipinfo object
///  - zipinfo.file does not exist
///  - one of zipinfo.entries path does not exist
///  - General IO error
#[labt_lua]
fn write(_lua: &Lua, table_self: Table) {
    let file_str: String = table_self.get("file")?;
    let append: bool = table_self.get("append")?;

    let global_alignment: Option<u16> = table_self.get("alignment")?;

    let path = Path::new(file_str.as_str());

    let mut zip = if append {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .context(format!(
                "Error opening zip output file: {}",
                path.to_string_lossy()
            ))
            .map_err(MluaAnyhowWrapper::external)?;
        // Open zip in append mode
        ZipWriter::new_append(file)
            .context(format!(
                "Failed to open zip file: {} in append mode",
                path.to_string_lossy()
            ))
            .map_err(MluaAnyhowWrapper::external)?
    } else {
        let file = File::create(path)
            .context(format!(
                "Error opening zip output file: {}",
                path.to_string_lossy()
            ))
            .map_err(MluaAnyhowWrapper::external)?;
        // Create new archive
        ZipWriter::new(file)
    };

    let entries: Vec<ZipEntry> = table_self.get("entries")?;

    for entry in &entries {
        let mut option = SimpleFileOptions::default();

        // set the alignment
        // check if specified otherwise check for global alignment
        if let Some(alignment) = entry.alignment {
            option = option.with_alignment(alignment);
        } else if let Some(alignment) = global_alignment {
            // use global alignment
            option = option.with_alignment(alignment);
        }

        // check if no compress was requested for this entry
        if let Some(true) = entry.no_compress {
            option = option.compression_method(zip::CompressionMethod::Stored);
        }

        if entry.is_dir {
            zip.add_directory_from_path(&entry.path, option)
                .context(format!(
                    "Failed to add directory entry into zip: [{}]",
                    entry
                ))
                .map_err(MluaAnyhowWrapper::external)?;
        } else {
            zip.start_file(entry.name.as_str(), option)
                .context(format!("Failed to start zip entry for file [{}]", entry))
                .map_err(MluaAnyhowWrapper::external)?;

            let mut file = File::open(&entry.path)
                .context(format!(
                    "Failed to open file \"{}\" to write to zip",
                    entry.path.to_string_lossy()
                ))
                .map_err(MluaAnyhowWrapper::external)?;

            io::copy(&mut file, &mut zip)?;
        }
    }

    zip.finish()
        .context("Failed to correctly complete zip file ")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(())
}

/// Adds a file entry to the zip
/// # Errors
/// Returns an error if self is not a valid zipinfo object
#[labt_lua]
fn add_file(lua: &Lua, (table_self, name, disk_path): (Table, String, String)) {
    let entries: Table = table_self
        .get("entries")
        .context("Missing field \"entries\" on self table")
        .map_err(MluaAnyhowWrapper::external)?;

    let entry = lua.create_table()?;
    entry.set("name", name)?;
    entry.set("path", disk_path)?;
    entry.set("is_dir", false)?;
    set_alignment(lua, &entry)?;
    set_no_compress(lua, &entry)?;

    entries
        .push(&entry)
        .context("Failed to add entry to zip entries")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(entry)
}

/// Sets file entry alignment
/// # Errors
/// Returns an error if failed to set alignment property
#[labt_lua]
fn set_alignment(_lua: &Lua, (table_self, alignment): (Table, u16)) {
    table_self.set("alignment", alignment)?;
    Ok(table_self)
}

/// Sets if the entry should be saved into the archive as is without compression
/// # Errors
/// Returns an error if failed to set no_compress property
#[labt_lua]
fn set_no_compress(_lua: &Lua, (table_self, store_only): (Table, bool)) {
    table_self.set("no_compress", store_only)?;
    Ok(table_self)
}

/// Adds a directory entry to the zip
/// # Errors
/// Returns an error if self is not a valid zipinfo object
#[labt_lua]
fn add_directory(lua: &Lua, (table_self, name): (Table, String)) {
    let entries: Table = table_self
        .get("entries")
        .context("Missing field \"entries\" on self table")
        .map_err(MluaAnyhowWrapper::external)?;

    let entry = lua.create_table()?;
    entry.set("name", "")?;
    entry.set("path", name)?;
    entry.set("is_dir", true)?;

    entries
        .push(entry)
        .context("Failed to add entry to zip entries")
        .map_err(MluaAnyhowWrapper::external)?;

    Ok(table_self)
}

fn new_zip_config(lua: &Lua, file: String, append: bool) -> mlua::Result<Table> {
    let zipinfo: Table = lua.create_table()?;
    let entries: Table = lua.create_table()?;

    // fields
    zipinfo.set("file", file)?;
    zipinfo.set("entries", entries)?;
    zipinfo.set("append", append)?;

    // functions
    write(lua, &zipinfo)?;
    add_file(lua, &zipinfo)?;
    add_directory(lua, &zipinfo)?;
    set_alignment(lua, &zipinfo)?;

    Ok(zipinfo)
}
/// Create a new zip file overwriting existing archive and its contents
#[labt_lua]
fn new(lua: &Lua, file: String) {
    Ok(new_zip_config(lua, file, false))
}

/// Open an existing archive in append mode
#[labt_lua]
fn new_append(lua: &Lua, file: String) {
    Ok(new_zip_config(lua, file, true))
}

#[labt_lua]
fn extract(_lua: &Lua, (table_self, output, extract_all): (Table, String, Option<bool>)) {
    let file_str: String = table_self
        .get("file")
        .context("Missing field \"file\" on self table")
        .map_err(MluaAnyhowWrapper::external)?;

    let path = Path::new(file_str.as_str());
    let file = File::open(path)
        .context(format!("Failed to open file \"{}\" ", file_str))
        .map_err(MluaAnyhowWrapper::external)?;

    let mut zip = ZipArchive::new(file)
        .context(format!("Failed to open zip archive \"{}\" ", file_str))
        .map_err(MluaAnyhowWrapper)?;

    let output_path = Path::new(output.as_str());

    let should_extract_all = if let Some(should_extract_all) = extract_all {
        should_extract_all
    } else {
        false
    };

    if should_extract_all {
        zip.extract(output_path)
            .context(format!("Failed to extract zip archive to \"{}\" ", output))
            .map_err(MluaAnyhowWrapper::external)?;
        return Ok(());
    }

    let entries: Vec<ZipEntry> = table_self
        .get("entries")
        .context("Missing field \"entries\" on self table")
        .map_err(MluaAnyhowWrapper::external)?;

    for entry in &entries {
        let mut zipfile = zip
            .by_name(&entry.name)
            .context(format!("Failed to locate \"{}\" in archive", entry.name))
            .map_err(MluaAnyhowWrapper::external)?;

        if zipfile.is_file() {
            let name_path = zipfile
                .enclosed_name()
                .context(format!(
                    "Invalid or insecure zip entry name {}",
                    zipfile.name()
                ))
                .map_err(MluaAnyhowWrapper::external)?;

            let mut extract_path = if entry.path == PathBuf::new() {
                PathBuf::from(output_path)
            } else {
                entry.path.clone()
            };

            let parent_path = name_path.parent().unwrap_or(Path::new(""));

            extract_path.push(parent_path);

            if !extract_path.exists() {
                create_dir_all(&extract_path)
                    .context(format!(
                        "Failed to create directory: {}",
                        extract_path.to_string_lossy()
                    ))
                    .map_err(MluaAnyhowWrapper::external)?;
            }

            let basename = name_path
                .file_name()
                .context(format!(
                    "Invalid file entry base name. File name cannot be empty. Path \"{}\" ",
                    name_path.to_string_lossy()
                ))
                .map_err(MluaAnyhowWrapper::external)?;

            extract_path.push(basename);

            let mut file = File::create(&extract_path)
                .context(format!(
                    "Failed opening output file \"{}\" ",
                    extract_path.to_string_lossy()
                ))
                .map_err(MluaAnyhowWrapper::external)?;

            io::copy(&mut zipfile, &mut file)
                .context("Failed to copy zip file entry bytes to output file")
                .map_err(MluaAnyhowWrapper::external)?;
        }
    }

    Ok(())
}

#[labt_lua]
fn with_name(lua: &Lua, (table_self, name, extract_path): (Table, String, Option<String>)) {
    let entries: Table = table_self
        .get("entries")
        .context("Missing field \"entries\" on self table")
        .map_err(MluaAnyhowWrapper::external)?;
    let entry = lua.create_table()?;
    entry.set("name", name)?;
    entry.set("path", extract_path.unwrap_or(String::new()))?;
    entry.set("is_dir", false)?;
    entries.push(entry)?;
    Ok(table_self)
}
/// Open a zip file for extraction
#[labt_lua]
fn open(lua: &Lua, file: String) {
    let zipinfo = lua.create_table()?;
    let entries = lua.create_table()?;

    zipinfo.set("file", file)?;
    zipinfo.set("entries", entries)?;
    with_name(lua, &zipinfo)?;
    extract(lua, &zipinfo)?;

    Ok(zipinfo)
}

#[labt_lua]
fn dump(_lua: &Lua, table: Table) {
    println!("{:#?}", table);
    Ok(())
}
/// Generates zip table and loads all its api functions
///
/// # Errors
///
/// This function will return an error if adding functions to fs table fails
/// or the underlying lua operations return errors.
pub fn load_zip_table(lua: &mut Lua) -> anyhow::Result<()> {
    let table = lua.create_table()?;

    new(lua, &table)?;
    new_append(lua, &table)?;
    open(lua, &table)?;
    dump(lua, &table)?;

    lua.globals().set("zip", table)?;
    Ok(())
}
