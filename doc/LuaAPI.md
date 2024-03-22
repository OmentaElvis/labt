# LABt Lua API Documentation
[TOC]
## Introduction
The LABt Lua API serves as the internally supported interface for 
developing plugins using Lua scripting language. These plugins are executed
during the build stage (the build subcommand).

### Build Steps
Plugins in LABt are organized into distinct build steps, each serving a specific purpose in the build pipeline. These steps include:

- **PRE**: Pre-compilation step for generating necessary dependencies.
- **AAPT**: Android Asset Packaging Tool for compiling Android resource files.
- **COMPILE**: Compilation step for application source code, producing JAR files.
- **DEX**: Dexing step for converting JAR files to Dalvik Executable format.
- **BUNDLE**: Bundling stage for assembling all build components into a single installable APK file.
- **POST**: Post-build step for additional tasks or actions.

## Plugin Guidelines
These are just suggestions on what a plugin should do at each step. You could do anything
on any step and its not limited to what is described.

### PRE
- Generate pre-build dependencies.
- Execute tasks such as running tests or generating protobuf code.
- Extra points for mining crypto ;)

### AAPT
- Compile Android resource files for project.
- Compile Libraries aar files and cache them for future builds.
- Generate R.java files.

### COMPILE
- Compile application source code into JAR files using any supported compiler: java, kotlin, node, cargo etc.
- Optimize by compiling only modified files.

### DEX
- Convert JAR files to Dalvik Executable format.
- Use any dexing compiler eg. dex, d8 or even r8 for release
- Implement caching mechanisms to optimize performance.

### BUNDLE
- Bundle all build components into a single APK file.
- Sign the APK with debug/release key based on build mode.

### POST
- Execute post-build tasks such as running the application on an emulator, pushing to a device, or performing additional tests.

## Directory structure
Plugins are stored on `$LABT_HOME/plugins` directory. If `LABT_HOME` is not set `$HOME/.labt/plugins` is used.

The plugin directory tree consists of a single `plugin.toml` file and any number of sub-folders or Lua files.
For the following example, the directory structure of a plugin `example-0.1.0`.

```
/home/.labt/plugins
└── example-0.1.0
    ├── aapt.lua
    ├── bundle.lua
    ├── compile.lua
    ├── dex.lua
    ├── plugin.toml
    ├── post.lua
    └── pre.lua

2 directories, 7 files
```

## plugin.toml
This is the configuration file for each plugin. It is required for every plugin.
Here is an example for the directory tree shown earlier.

```toml
name="example"
version="0.1.0"
author="omentum"

# pre build
[stage.pre]
file="pre.lua"
priority=1

# android asset packaging tool step.
[stage.aapt]
file="aapt.lua"
priority=1

# java compilation
[stage.compile]
file="compile.lua"
priority=1

# dexing
[stage.dex]
file="dex.lua"
priority=1

# bundling
[stage.bundle]
file="dex.lua"
priority=1

# post build
[stage.post]
file="post.lua"
priority=1

```
for every stage that the plugin needs to be executed, it must provide
a target file to be loaded relative to the plugin root directory.

# The Lua API
At plugin loading, labt injects several functions and tables into the Lua
context. These provide the plugin with utility functions and the project
metadata. The plugin system is still on its infancy and more functionality
will be added as it evolves. To supplement this, the Lua instance provided
is not sand-boxed and the plugins can utilize the full power of the Lua standard
API.

## `labt` table
A table named labt is injected into the global context of the plugin. This table
contains utility functions implemented directly in rust and include some of the
functions used by labt itself internally. The implementation of this functions 
can be found at [src/plugin/api/labt.rs](../src/plugin/api/labt.rs).

***
### `get_project_config`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: None <br>
**returns**: table
***

Returns the project configuration stored on `Labt.toml` file. The result of this
function is a mapping of the configuration into the Lua table.
This function may fail if an error occurs parsing Labt.toml.

```lua

-- Get project config
local config = labt.get_project_config();

-- get project name
local project_name = config.project.name;
print(project_name)

-- get project direct dependencies
local deps = config.dependencies;
-- ensure dependencies is not nil
if deps then
	-- loop through project dependency
  for artifact_id, info in pairs(deps) do
  	print(artifact_id..":"..info.group_id..":"..info.version)
  end
else
  print("Project has no direct dependencies")
end

```

***
### `get_build_step`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: None <br>
**returns**: string
***
Returns the current build stage/step the plugin was executed on.

```lua
-- Get build stage
local step = labt.get_build_step()
-- check if its bundling stage
if step == "BUNDLE" then
  print("Bundling the application")
end

```

***
### `get_lock_dependencies`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: None <br>
**returns**: table
***

Returns an array of the project resolved dependencies. This is a full list
of what the project needs to build. The function does not start a dependency
resolution, instead it parses Labt.lock file and returns its representation 
as a Lua array of tables. If the lock file is empty then an empty array is returned.
Therefore it is assumed that the project dependencies are already resolved.

```lua
local deps = labt.get_lock_dependencies();

-- Print all dependencies
for index, dep in ipairs(deps) do
	print(dep.group_id..":"..dep.artifact_id..":"..dep.version)
end

```

***
### `get_project_root`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: None <br>
**returns**: string
***

Returns the project root directory by recursively looking for Labt.toml up the
directory tree. Returns an error if the project root was not located or labt 
encountered a file system related error during the search.


***
### `resolve`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: None <br>
**returns**: nil
***
Calls dependency resolution algorithm on dependencies found in Labt.toml
Returns an error if:

- resolving the dependencies fail
- failed to read project config [`Labt.toml`]
- failed to read and configure resolvers from config

## `fs` table
A table containing utility functions for working with the file system.
This functions are implemented in rust at [src/plugin/api/fs.rs](../src/plugin/api/fs.rs).

***
### `mkdir`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string <br>
**returns**: Nil
***

creates the directory specified. If the path provided is relative, this function
creates the path relative to the project root.
Returns an error if:

- obtaining the project root directory fails
- creating the directory fails
- directory already exists

***
### `mkdir_all`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string <br>
**returns**: Nil
***

creates the directory specified and all its parent directories if they don't already exist.
If the path provided is relative, this functioncreates the path relative to the project root.
Returns an error if: 

- obtaining the project root directory fails
- creating the directory fails

***
### `exists`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: None <br>
**returns**: boolean
***

Returns true if file exists and false if does not exist.
if the file/dir in question cannot be verified to exist or not exist due
to file system related errors, the function errors instead.
