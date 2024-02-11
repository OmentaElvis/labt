use crate::{
    config::{LabToml, Project},
    templating::{Activity, ActivityXml, AndroidManifest, StringsRes},
};
use anyhow::{bail, Context};
use clap::Args;
use dialoguer::{theme::ColorfulTheme, Input};
use regex::Regex;
use sailfish::TemplateOnce;
use std::{
    fs::{self, File},
    io::{self, Write},
    path::PathBuf,
};

use super::Submodule;

#[derive(Args, Clone)]
pub struct InitArgs {
    /// Project name
    #[arg(short, long)]
    name: Option<String>,
    /// Java package name
    #[arg(long)]
    package: Option<String>,
    /// Run interactive
    #[arg(long, action)]
    interactive: bool,
    /// Directory to create project in
    path: PathBuf,
    #[arg(long)]
    /// Internal version number
    version_number: Option<i32>,
    #[arg(long)]
    /// External version name visible to users
    version_name: Option<String>,
    #[arg(long)]
    /// Application Main activity
    main_activity: Option<String>,
    #[arg(long, short, action)]
    /// Suppress logs
    quiet: bool,
    /// Project description
    #[arg(long, short)]
    description: Option<String>,
}

pub struct Init {
    pub args: InitArgs,
}

pub struct ProjectPaths {
    pub root: PathBuf,
    pub app: PathBuf,
    pub package: PathBuf,
    pub res: PathBuf,
}

impl Init {
    pub fn new(args: &InitArgs) -> Init {
        return Init { args: args.clone() };
    }
    fn interactive(&mut self) -> dialoguer::Result<()> {
        // Query user for the project name
        // Check if the name was provided as an argument
        let default_name = match &self.args.name {
            Some(n) => n.clone(),
            None => String::from(""),
        };

        // prompt the user, add the provided name as placeholder
        let name = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Project name")
            .allow_empty(false)
            .show_default(self.args.name.is_some())
            .validate_with(|input: &String| {
                if input.trim().is_empty() {
                    Err("Value is required!")
                } else {
                    Ok(())
                }
            })
            .default(default_name)
            .interact_text()?;

        // check if the description was already fed through command line args
        // if it exists , set the value of default description to that of the
        // argument
        let default_description = match &self.args.description {
            Some(d) => d.clone(),
            None => String::from(""),
        };

        // prompt the user for the project description
        let descriprion = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("description")
            .allow_empty(true)
            .show_default(self.args.description.is_some())
            .default(default_description)
            .interact_text()?;

        // prompt the user, for package name
        let package = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Package name")
            .validate_with(|input: &String| {
                if input.is_empty() {
                    Err("Value required")
                } else {
                    if let Ok(re) = Regex::new(r"^([a-z]+(\.)?)+$") {
                        if !re.is_match(input.as_str()) {
                            Err("Please provide a valid package Name. e.g. com.example.app")
                        } else {
                            Ok(())
                        }
                    } else {
                        Ok(())
                    }
                }
            })
            .interact_text()?;
        self.args.package = Some(package);
        self.args.name = Some(name);
        self.args.description = Some(descriprion);

        // prompt user for version number
        let version_number = Input::<i32>::with_theme(&ColorfulTheme::default())
            .with_prompt("Version number")
            .validate_with(|input: &i32| {
                if input.is_negative() {
                    Err("Provide a positive number")
                } else {
                    Ok(())
                }
            })
            .default(1)
            .show_default(true)
            .allow_empty(false)
            .interact_text()?;

        self.args.version_number = Some(version_number);

        // prompt user for version none
        let version_name = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Version name")
            .allow_empty(false)
            .default("0.1.0".to_string())
            .show_default(true)
            .validate_with(|input: &String| {
                if input.trim().is_empty() {
                    Err("Value cant be empty!")
                } else {
                    Ok(())
                }
            })
            .interact_text()?;

        self.args.version_name = Some(version_name);

        let main_activity = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Main Activity")
            .default("MainActivity".to_string())
            .show_default(true)
            .validate_with(|input: &String| {
                if input.trim().is_empty() {
                    Err("Value can't be empty!")
                } else {
                    Ok(())
                }
            })
            .interact()?;
        self.args.main_activity = Some(main_activity);

        Ok(())
    }
    fn build_tree(&self) -> Result<ProjectPaths, io::Error> {
        // Create project folder
        let mut path = self.args.path.clone();

        // canonicalize if relative in order to allow extracting of
        // the parent folder name
        if path.is_relative() {
            path = path.canonicalize().map_err(|err| {
                return io::Error::new(
                    err.kind(),
                    format!("Unable to canonicalize path: {}", err.to_string()),
                );
            })?;
        }

        // Try to extract project name. Current path may have been provided
        // by the user, check to see if the user provided name of the project as
        // argument, if not use the parent folder as the project name

        if let Some(name) = &self.args.name {
            path.push(name.clone());
        }

        // check if directory exists, else create a new dir
        if path.exists() {
            // check if path is empty
            if !self.is_dir_empty(&path)? {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "The target directory exists and is not empty",
                ));
            }
        } else {
            fs::create_dir(&path)?;
        }

        // Create corresponding paths
        let app_path: PathBuf = path.join("app");
        if !app_path.exists() {
            fs::create_dir(&app_path)?;
        }

        // java folder with packages
        let package = &self
            .args
            .package
            .clone()
            .unwrap_or("com.example.app".to_string())
            .replace(".", "/");

        let mut java_path: PathBuf = app_path.join("java");
        java_path.push(&package);

        fs::create_dir_all(&java_path)?;

        // res folder

        let res_path: PathBuf = app_path.join("res");

        let drawables = ["", "hdpi", "ldpi", "mdpi", "xhdpi", "xxhdpi", "xxxhdpi"];

        // create drawables
        for drawable_type in drawables {
            if drawable_type.is_empty() {
                fs::create_dir_all(res_path.join("drawables"))?;
            } else {
                fs::create_dir_all(res_path.join(format!("drawable-{}", drawable_type)))?;
            }
        }

        // layout
        fs::create_dir_all(res_path.join("layout"))?;
        // menu
        fs::create_dir_all(res_path.join("menu"))?;
        // values
        fs::create_dir_all(res_path.join("values"))?;
        // xml
        fs::create_dir_all(res_path.join("xml"))?;

        // assets
        fs::create_dir_all(app_path.join("assets"))?;

        Ok(ProjectPaths {
            res: res_path,
            root: path,
            package: java_path,
            app: app_path,
        })
    }
    fn is_dir_empty(&self, path: &PathBuf) -> io::Result<bool> {
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Path does not exist",
            ));
        }
        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "The path provided is not a directory. A file with the same name exists!",
            ));
        }

        match path.read_dir() {
            Ok(mut files) => return Ok(files.next().is_none()),
            Err(e) => return Err(e),
        }
    }
    pub fn template_files(&self, paths: &ProjectPaths) -> anyhow::Result<()> {
        let args = self.args.clone();

        {
            // Render an AndroidManifest.xml
            let mut path = paths.app.clone();
            let action = "AndroidManifest-Templating";
            path.push("AndroidManifest.xml");

            // Create manifest file
            let mut file = File::create(&path).context(format!(
                "Creating file for {} - Path: {}",
                &action,
                path.to_str().unwrap_or("")
            ))?;

            // Run template
            let manifest = AndroidManifest::new(
                args.package
                    .to_owned()
                    .unwrap_or("com.example.app".to_string())
                    .as_str(),
                args.version_number.to_owned().unwrap_or(1),
                args.version_name
                    .to_owned()
                    .unwrap_or("1.0.0".to_string())
                    .as_str(),
                args.main_activity
                    .to_owned()
                    .unwrap_or("MainActivity".to_string())
                    .as_str(),
            );
            // Render mabifest and return rendered string
            let data = manifest.render_once().context(format!(
                "Rendering for {} Path: {}",
                action,
                path.to_str().unwrap_or("")
            ))?;

            // Write rendered string to file
            file.write_all(data.as_bytes()).context(format!(
                "Writing data for {} to {}",
                action,
                path.to_str().unwrap_or("")
            ))?;
        }

        // Main Activity.java
        {
            let mut path = paths.package.clone();
            let action = "MainActivity-Templating";
            path.push({
                match args.main_activity.to_owned() {
                    Some(class) => class + ".java",
                    None => "MainActivity.java".to_string(),
                }
            });
            // Create file
            let mut file = File::create(&path).context(format!(
                "Creating file for {} - Path: {}",
                &action,
                path.to_str().unwrap_or("")
            ))?;

            // initialize template
            let activity = Activity::new(
                args.package
                    .to_owned()
                    .unwrap_or("com.example.app".to_string())
                    .as_str(),
                args.main_activity
                    .to_owned()
                    .unwrap_or("MainActivity".to_string())
                    .as_str(),
                Some("activity_main".to_string()),
            );

            // generate template
            let data = activity.render_once().context(format!(
                "Rendering for {} Path: {}",
                &action,
                path.to_str().unwrap_or("")
            ))?;

            // write data to file
            file.write_all(data.as_bytes()).context(format!(
                "Writing data for {} - to File: {}",
                action,
                path.to_str().unwrap_or("")
            ))?;
        }
        {
            let path = paths.res.join("layout/activity_main.xml");
            let action = "XML-Activity-Templating";
            let mut file = File::create(&path).context(format!(
                "Creating file for {} - Path: {}",
                &action,
                path.to_str().unwrap_or("")
            ))?;
            let activity_main = ActivityXml::new();

            // render template
            let data = activity_main.render_once().context(format!(
                "Rendering for {} Path: {}",
                &action,
                path.to_str().unwrap_or("")
            ))?;

            // write rendered template to file
            file.write_all(data.as_bytes()).context(format!(
                "Writing data for {} - to File: {}",
                action,
                path.to_str().unwrap_or("")
            ))?;
        }
        {
            let mut path = paths.res.clone();
            path.push("values/strings.xml");
            let action = "Strings-Templating";

            let mut file = File::create(&path).context(format!(
                "Creating file for {} - Path: {}",
                &action,
                path.to_str().unwrap_or("")
            ))?;
            let strings_xml = StringsRes::new(
                args.name.to_owned().unwrap_or("App".to_string()).as_str(),
                args.main_activity
                    .to_owned()
                    .unwrap_or("App".to_string())
                    .as_str(),
            );
            // render template
            let data = strings_xml.render_once().context(format!(
                "Rendering for {} Path: {}",
                &action,
                path.to_str().unwrap_or("")
            ))?;

            // write rendered template to file
            file.write_all(data.as_bytes()).context(format!(
                "Writing data for {} - to File: {}",
                action,
                path.to_str().unwrap_or("")
            ))?;
        }
        {
            // Write the project .toml file
            let mut path = paths.root.clone();
            let args = self.args.clone();
            let toml = LabToml {
                project: Project {
                    name: args.name.unwrap_or("myapp".to_string()),
                    description: args.description.unwrap_or("".to_string()),
                    version_number: args.version_number.unwrap_or(1),
                    version: args.version_name.unwrap_or("0.1.0".to_string()),
                    package: args.package.unwrap_or("com.example".to_string()),
                },
            };
            // serialize to toml string
            let toml = toml::to_string(&toml).context("Serializing LabtToml to toml string")?;
            path.push("Labt.toml");

            // create file target to write toml file
            let mut file = File::create(&path).context(format!(
                "Creating Labt.toml file at {}",
                path.to_str().unwrap_or("[unknown]")
            ))?;

            // write the toml to file
            file.write_all(toml.as_bytes()).context(format!(
                "Writing LabtToml string to toml file at {}",
                path.to_str().unwrap_or("[unknown]")
            ))?;
        }
        Ok(())
    }
}

impl Submodule for Init {
    fn run(&mut self) -> anyhow::Result<()> {
        if self.args.interactive {
            if let Err(err) = self.interactive() {
                bail!(err);
            }
        }

        let project_paths = self.build_tree()?;
        self.template_files(&project_paths)?;

        Ok(())
    }
}
