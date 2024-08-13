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

[sdk]
build = { path = "build-tools;33.0.2", version = "33.0.2.0", channel = "stable"}
# or a full id in format (path:version:channel)
platform = "platforms;android-33:3.0.0.0:stable"

#or some toml dot table
[sdk.cmdtools]
path = "platforms;android-33"
version = "16.0.0.1"
channel = "stable"

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

## Sdkmanager
LABt provides a custom Lua module loader that simplifies loading SDK modules with 
the `require` statement.

### Loading SDK Modules
To load an SDK module, use the following syntax:

```lua
local build = require("sdk:build")
local platform = require("sdk:platform")
```

The `require` argument must be prefixed with `sdk:` followed 
by the package name as defined in `plugin.toml`. 

```toml
#plugin.toml
#the other configs
[sdk]
build = { path = "build-tools;33.0.2", version = "33.0.2.0", channel = "stable"}
platform = "platforms;android-33:3.0.0.0:stable"

#the other configs

```
This will return a table representing a virtual module.


### Executing SDK Commands
You can execute functions on the returned module, which maps the function 
name to an executable within the SDK's directory. For example:

```lua
local build = require("sdk:build")
local ok, exitcode = build.aapt2("version")
```

This will look for an executable named `aapt2` in the `
build-tools` package directory and run it with the argument `
version` (i.e., `aapt2 version`). The output 
is directed to the default `stdout` and `stderr`.

### Capturing Command Output

If you need to capture the output of an SDK tool, prefix 
the function name with `get_`:

```lua
local build = require("sdk:build")
local ok, stdout, stderr = build.get_aapt2("version")
local version

if ok then
    version = stderr  -- Note: `aapt2` outputs to `stderr`
else
    error(stderr)
end
```

### Retrieving File Paths

To retrieve the path to a specific file within an SDK package, 
use the `file` function:

```lua
local platform = require("sdk:platform")
local android_jar = platform.file("android.jar")

-- `android_jar` would expand to:
-- $LABT_HOME/sdk/platforms/android-33/android.jar
```

### Fine-Grained SDK Directory Access

You can also access specific sub directories within an SDK package:

```lua
local build = require("sdk:build/lib")
-- SDK directory is set to 'lib'
local d8_jar = build.file("d8.jar")

-- `d8_jar` would expand to:
-- $LABT_HOME/sdk/build-tools/33.0.2/lib/d8.jar
```

### Validation and Security
LABt performs basic validation of function names before constructing the executable path. 
For example, function names containing `/` or `\` are rejected. This 
validation might be expanded in the future as we implement stricter sandboxing for 
Lua plugins.

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


***
### `get_cache_path`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: group_id: string, artifact_id: string, version: string, packaging: string <br>
**returns**: string
***

Returns the cache location for this dependency. This does not check if the path
exists. It constructs a valid cache path according to the labt cache resolver.
Returns an error if:

- Labt home was not initialized
- Failed to convert path to its unicode string representation

```lua
-- Get project dependencies
local deps = labt.get_lock_dependencies()

-- loop through project dependencies
for _, dep in ipairs(deps) do
	local path = labt.get_cache_path(dep.group_id, dep.artifact_id, dep.version, dep.packaging)
	if dep.packaging == "aar" then
		-- This dep is an aar.
		-- Extract it into its res files and jar
		if fs.exists(path) then
		-- process the aar
		else
			-- ERROR
		end
	else
		-- The rest of deps eg. jar copy them to a libs folder
	end
end
```

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
If the path provided is relative, this function creates the path relative to the project root.
Returns an error if: 

- obtaining the project root directory fails
- creating the directory fails

***
### `exists`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string - path <br>
**returns**: boolean
***

Returns true if file exists and false if does not exist.
if the file/dir in question cannot be verified to exist or not exist due
to file system related errors, the function errors instead.

***
### `is_newer`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string - path a, string path b <br>
**returns**: boolean
***

Returns true if:
- file a is newer than file b
- file b does not exist

Returns false if:
- file a does not exist (Technically b should be newer if a is missing)

Note: if a folder is provided, it just checks the modification time of the folder,
therefore it would not pick changes made to internal files/folders. If you want to check
if files change in a folder, then select required files using `fs.glob` and scan through them.

Returns an error if we fail to get the metadata of the file

***
### `glob`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string - The globbing pattern<br>
**returns**: table - Array of paths 
***

Returns all files that match a globbing pattern. It returns only files that are
readable (did not return IO errors when trying to list them) and files whose path
string representation is a valid Unicode. If you specify a relative path, it is evaluated
from the root of the project. <br>
Returns an error if:

- failed to parse the globbing pattern;
- Failed to get the project root for relative paths
- Failed to convert project root + glob pattern into Unicode

```lua
local java = fs.glob("app/java/**/*.java")
-- pass the source files to the java compiler

```

## `log` table
Provides log utility functions used by labt internally. This allows for consistency
and working with labt's progress bars. The Rust internal implementation can be found at
[src/plugin/api/log.rs](../src/plugin/api/log.rs)

```lua
log.info("javac", "Compiling N source files")
log.error("aapt", "Failed to locate ANDROID_HOME")
log.warn("bundle", "Signing apk with development key")
```

***
### `info`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: target, string: message <br>
**returns**: nil
***

Logs at the info log level

***
### `warn`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: target, string: message <br>
**returns**: nil
***
Logs at the warn log level

***
### `error`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: target, string: message <br>
**returns**: nil
***
Logs at the error log level

## `sdk` Module

The `sdk` module is returned by `require("sdk:<
package_name>")` and provides access to various properties and functions associated with the 
SDK package. This module includes both predefined fields and dynamic functions.

### Fields

- **`path`**:  
  **Type**: `string`  
  **Description**: The unique identifier for the SDK package, derived 
	from the `repository.xml` schema. In LABt, this path is processed by replacing `;` 
	with `/`, and the corresponding directory tree is created during package installation.

- **`version`**:  
  **Type**: `string`  
  **Description**: The version of the SDK package, formatted as 
	`major.minor.micro.preview`.

- **`channel`**:  
  **Type**: `string`  
  **Description**: The release channel of the SDK package, such 
	as `stable`, `beta`, etc.

### Functions

***
#### `file`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`  
**arguments**: `string`  
**returns**: `string`  
***

**Description**:  
Returns the full path to a specified file within the SDK package. 
The path is relative to the package's root directory.

**Example Usage**:

```lua
local platform = require("sdk:platforms")
local android_jar = platform.file("android.jar")
```

***
#### `<dynamic_function_name>`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`  
**arguments**: variable (depends on the executable's requirements)  
**returns**: `(bool, number|nil)`  
***

**Description**:  
Represents a dynamic function call mapped to an executable within the SDK package 
directory. The function name corresponds directly to the name of the executable.

**Example Usage**:

```lua
local build = require("sdk:build")
local ok, exitcode = build.aapt2("version")  -- Executes 'aapt2 version' in the build-tools package
```

**Returns**:
- `bool`: `true` if the executable runs successfully, 
	`false` otherwise.
- `number|nil`: The exit code of the executable, 
	or `nil` if the command failed to execute.

***
#### `get_<dynamic_function_name>`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`  
**arguments**: variable (depends on the executable's requirements)  
**returns**: `(bool, string, string)`  
***

**Description**:  
A variant of `<dynamic_function_name>` that captures and returns the `stdout
` and `stderr` output of the SDK tool as strings.

**Example Usage**:

```lua
local build = require("sdk:build")
local ok, stdout, stderr = build.get_aapt2("version")
```

**Returns**:
- `bool`: `true` if the executable runs successfully, `false` otherwise.
- `string`: `stdout` output from the command.
- `string`: `stderr` output from the command.

### Notes on Dynamic Functions

- **Dynamic Function Names**: The function names are dynamic and correspond 
	directly to the executables in the SDK package's directory. For 
	example, if you require the `build` SDK, calling `
	build.aapt2("version")` would map to the `aapt2` executable in the `build-tools` directory.

- **Function Name Validation**: LABt currently performs basic validation on function 
	names to prevent the use of illegal characters (e.g., 
	`/` or `\`). This validation may be enhanced in future versions to support 
	stricter sandboxing for Lua plugins.
