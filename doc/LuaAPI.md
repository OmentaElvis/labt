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

## Selective execution
For each plugin stage, you can specify the inputs and outputs the stage depends on.
LABt will compare the inputs based on their last modification date. It checks if the
inputs are more recent than the output. If true, then we can execute the stage otherwise
we skip the stage. This gives a basis of a simple caching system to plugins.

Example to only resolve the project dependencies when `Labt.toml` changes.

```toml
# plugin.toml
#... other config

[stage.pre]
file="pre.lua"
priority=2
inputs=["Labt.toml"]
outputs=["Labt.lock"]
#... other config
```
and in your lua

```lua
-- pre.lua
-- other code
labt.resolve()
-- other code
```

for folders and multiple files you can use globbing patterns and LABt will compare
all files that match that pattern. This uses "short circuiting" which means only one 
entry needs to be outdated for the execution of the stage to trigger.

```toml
# only run aapt when app resources change or res.apk is missing (first time execution)
[stage.aapt]
file = "aapt.lua"
priority = 1
inputs = ["app/res/**/*", "app/AndroidManifest.xml"]
outputs = ["build/res.apk"]
```

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
### Third party repositories
The default repository for sdk packages is google [https://dl.google.com/android/repository/repository2-1.xml](https://dl.google.com/android/repository/repository2-1.xml).
You can host your own repository by using the google android repository format and providing an accessible url to the repository xml.

To use the repository in your plugin:

```toml
name="example"
version="0.1.0"
author="omentum"

[[repository]]
name = "labt"
url = "https://example.com/labt/sdk/repository.xml"

[sdk]
r55 = {repo = "labt", path = "r55;lua", version = "0.1.0", channel = "stable"}
# or
# r55 = "labt:r55;lua:0.1.0:stable"
```

#### Custom behaviour implemented in parsing the repository xml.
You can specify an archive download url using `<base-url>` tag.
You need to specify this for archives otherwise it will default to google repo base url.

```xml
<remotePackage path="platforms;android-34">
	<revision>
		<major>1</major>
	</revision>
	<display-name>Android SDK Platform 34-ext11</display-name>
	<uses-license ref="android-sdk-license"/>
	<channelRef ref="channel-0"/>
	+ <base-url>https://example.com/</base-url>
	<archives>
		<archive>
			<complete>
				<size>63446827</size>
					<checksum>dfb498e3d0d97769aef5e1eb9ddff5b001e65829</checksum>
					<url>platform-34-ext11_r01.zip</url>
					<!-- or -->
					<!-- <url>https://example.com/repository/platform-34-ext11_r01.zip</url> -->
			</complete>
		</archive>
	</archives>
</remotePackage>
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
- Failed to convert path to its Unicode string representation

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
### `copy`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string - source path, string - destination path, boolean? - recursive <br>
**returns**: Nil
***

Copies a file or directory from the source path to the destination path. 
If the source is a directory, the recursive argument must be set to 
true to enable copying of its contents. If recursive is false and the 
source is a directory, an error will be returned.

If the destination path is a directory, the source file's name 
will be appended to the destination path. If the source path is relative
, it will be resolved against the project root directory.

Returns an error if: 

- The source path does not exist.
- The destination path cannot be created.
- An attempt is made to copy a directory without enabling recursive mode.
- Any I/O operation fails during the copy process.

```lua
fs.copy("path/to/source.txt", "path/to/destination.txt") -- Copy a file
fs.copy("path/to/source_dir", "path/to/destination_dir", true) -- Copy a directory recursively
```

***
### `mv`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string - source path, string - destination path <br>
**returns**: Nil
***

Renames or moves a file or directory from the source path to the destination 
path. If the source path is relative, it will be resolved against the project root directory.

Returns an error if: 

- The source path does not exist.
- The destination path cannot be created or is invalid.
- Any I/O operation fails during the rename/move process.

```lua
fs.mv("path/to/source.txt", "path/to/destination.txt") -- Move a file
fs.mv("path/to/source_dir", "path/to/destination_dir") -- Move a directory
```

***
### `rm`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string - target path <br>
**returns**: Nil
***

Removes a file or directory at the specified path. If the path is 
a directory, it can be removed either recursively or non-recursively. 
If recursive is set to true, the directory and all its contents will 
be deleted. Warning: Recursive deletion is very dangerous if not implemented correctly.

If the path is relative, it will be resolved against the project root 
directory.

Returns an error if: 

- The specified path does not exist.
- The path is a directory and cannot be removed (e.g., if it is not empty and recursive is not set).
- Any I/O operation fails during the removal process.

```lua
fs.rm("path/to/file.txt") -- Remove a file
fs.rm("path/to/directory", true) -- Recursive delete
```

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
## `sys` table
The sys table provides a mechanism for executing system commands directly from Lua scripts. 
It includes an __index metatable for dynamically resolving command names to executable functions.
This functions are implemented in rust at [src/plugin/api/sys.rs](../src/plugin/api/sys.rs).

***
### `<command>`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: ... string: Arguments to be passed to the system command. <br>
**returns**: 

- boolean: Indicates whether the command executed successfully., 
- number?: The exit code of the command (if available).

***

Executes a system command without using a shell, ensuring that arguments are passed directly 
to the command without interpretation. 
This avoids issues with shell-based operations and improves security.

```lua
local ok, exitcode = sys.ls("-l");
```
or call javac to compile your code

```lua
local ok, exitcode = sys.javac("app/java/com/example/Main.java", "-classpath", android_jar, "-d", "build")
if not ok then
  log.error("javac", "Failed to compile java code")
	return
end

```
***
### `get_<command>`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: ... string: Arguments to be passed to the system command. <br>
**returns**: 

 - boolean: Indicates whether the command executed successfully., 
 - stdout: Captured standard output (stdout) of the command., 
 - stderr: Captured standard error (stderr) of the command.

***

Executes a system command and captures its output streams (stdout and stderr) for further processing. Like the `<command>` function, 
it does not use a shell to interpret the arguments.

```lua
local ok, stdout, stderr = sys.get_ls("-l");
```
or call javac to compile your code

```lua
local ok, stdout, stderr = sys.get_javac("app/java/com/example/Main.java", "-classpath", android_jar, "-d", "build")
if not ok then
  log.error("javac"  stderr)
  log.error("javac", "Failed to compile java code")
	return
end

```
Returns an error if:

- The command was not found on system PATH

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

## `prompt` table
This table offers utility functions to obtain input from users.
The Rust internal implementation can be found at
[src/plugin/api/prompt.rs](../src/plugin/api/prompt.rs)

```lua
local confirm = prompt.confirm("Launch adb?")
```

***
### `confirm`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: prompt, boolean?: default<br>
**returns**: boolean
***
Prompt the user with a true false question. This prompt is not
cancellable.

Errors if:

- Failed to show prompt to user

***
### `confirm_optional`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: prompt, boolean?: default<br>
**returns**: boolean | nil
***
Prompt the user with a true false question. This prompt can be cancelled if the user presses ESC.
Returns nil if the user cancels the prompt.

Errors if:

- Failed to show prompt to user

***
### `input`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: prompt, string?: default, validator?: function<br>
**returns**: string
***

Prompt the user for a string input.
You can set a default value
You can provide an optional validator callback that is going to verify the input and return an error string if invalid or nil if valid.

Returns the entered string

```lua
local file = prompt.input("Enter output file name?", nil, function(input)
  if input == "COMM" then
    return "You cannot use COMM as a file name."
  end
end)

print(file)

local b = prompt.input("Enter package name?", "com.labt") -- with default
local c = prompt.input("Enter package name?") -- with no default
```

Errors if:

- Failed to show prompt to user

***
### `input_number`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: prompt, number?: default, validator?: function<br>
**returns**: number
***

Prompt the user for a number input.
You can set a default value
You can provide an optional validator callback that is going to verify the input and return an error string if invalid or nil if valid.

Returns the entered string

```lua
local percentage = prompt.input_number("Enter percentage?", nil, function(input)
  if input < 0 or input > 100 then
    return "Select a number between 0 and 100"
  end
end)

print(percentage)
```

Errors if:

- Failed to show prompt to user

***
### `input_password`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: prompt, validator?: function<br>
**returns**: string
***

Prompt the user for a hidden input.
You can provide an optional validator callback that is going to verify the input and return an error string if invalid or nil if valid.

Returns the entered string

```lua
local password = prompt.input_password("Enter password for signing certificate?", function(password)
  if #password == 0 then
    return "Password cannot be empty"
  end
end)

print(password)

-- without validation
local p = prompt.input_password("Enter secret key?")
```

Errors if:

- Failed to show prompt to user

***
### `select`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: prompt, table: array of choices, number?: default index<br>
**returns**: number
***

Prompt the user to choose a value from a list of choices.
You can set a default choice which is highlighted.

Returns the selected option as a lua index to the provided array.

```lua
local devices = {
	  "emulator 1", "device a", "device b"
};

local select = prompt.select("Select adb device to push apk.", devices, 1);

print(devices[select])
```

Errors if:

- Failed to show prompt to user

***
### `multi_select`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: string: prompt, table: array of choices, table?: array of default values<br>
**returns**: table: array of selected indexes
***

Prompt the user to choose multiple choices from a list of choices.
You can set a default selected coices. The array of default options
 are matched in order of their indexes on the choices table.

Returns the selected choices as an array of lua indexes into the choices table.

```lua
local features = {
  "tea", "mug", "power", "noise"
};

local select = prompt.multi_select("Select additional features", features, {
  true, false, true
});

print("Adding: ")
for i = 1, #select do
  print(features[i]);
end
```

Errors if:

- Failed to show prompt to user

## `zip` Module
Android apks are just fancy zip files. So it makes sense to include
a zip modules so that you can zip and unzip at ease. LABt injects 
zip module onto the global scope to allow archiving operations.

### class `ZipWriter`

***
#### `new`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: file string<br>
**returns**: ZipWriter writer Zip file info
***

Starts a new zip archive.

***
#### `new_append`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: file string<br>
**returns**: ZipWriter writer Zip file info
***
Starts a new zip archive in append mode.

***
#### `add_file`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: 
	
- _name_: string - The file name as shown in the zip file
- _disk_path_: string - The path to the file that is to be added to the archive
- _alignment?_: number - sets the alignment of this file entry on the archive

**returns**: ZipEntry entry An entry to be added on the zip archive
***

Adds a file entry to the zip. This file will be added to the zip file during write.

***
#### `add_directory`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: name string The directory name entry as shown in the zip file<br>
**returns**: ZipWriter
***

Adds a directory entry to the zipinfo object. This directory will be added to the final zip archive file. Returns self.

Note: This function does not add entire disk directory onto the archive. It only creates a directory tree in the zip archive.

***
#### `write`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: None<br>
**returns**: nil
***
Commits this zip object to disk. This function starts the actual archive writing operation.
Returns an error if:

- internal IO error occurs such as file access error


### class `ZipReader`

#### `open`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: file string<br>
**returns**: ZipReader reader Zip file reader
***
Opens a zip archive in read mode. This allows the user to extract files from an archive to disk.

***
#### `with_name`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: 

- _name_: string -  A valid name entry in the zip archive
- _extract_path_: string -  The path where this file will be written on disk

**returns**: ZipReader reader The zip reader
***
Adds the file name to the list of entries that you would like to extract from the archive. 
The file name must be a valid file entry in the archive.
If extract_path is nil, the root extract path set by the extract method is used.

***
#### `extract`
**stage**: `PRE, AAPT, COMPILE, DEX, BUNDLE, POST`
**arguments**: 

- _output_path_: string - The extraction destination path. Must exist on the file system
- _extract_all_: boolean - If we should extract everything to destination path

**returns**: ZipReader reader The zip reader
***
Extracts the listed files or all files onto the specified directory. 
This method goes through the list of entries added with `with_name` and extracts them one by one. If a file entry has its own output directory specified by `with_name` function, it is used to write the file otherwise the `output_path` argument is used as the destination directory.
If a file entry name has a directory tree e.g. path/to/my/file.txt, all the missing paths are created.
if `extract_all` option is specified as true, all the files in the archive are extracted ignoring the filter entries added by with_name
This function overwrites output file if conflicted by an existing file.
Returns an error if:

- Underlying IO error occurs that was unexpected
- The zip file to extract was not found or failed to open
- An entry set by `with_name` was not found in zip file
- Failed to create output directory tree
- Invalid or insecure zip entry name
- Invalid file entry base name
- Failed to open output file for write.

```lua
local zipinfo = zip.open("test.zip");
-- extract all files
zipinfo:extract("path/to/output", true);
```


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


## FFI Capabilities

### Overview

LABt supports Foreign Function Interface (FFI) capabilities through LuaJIT, allowing developers to call C functions directly from Lua scripts. 
This feature enables the use of native libraries and legacy tools seamlessly within your plugin. To utilize FFI, you must enable the "unsafe" 
mode in your `plugin.toml` file by adding `unsafe = true`.

### Enabling FFI in Your Plugin

To enable FFI in your plugin, modify your `plugin.toml` file as follows:

```toml
name = "custom"
version = "1.0"
unsafe = true

[stage.pre]
file = "pre.lua"
priority = 10
```

or enable per file/build stage.

```toml
name = "custom"
version = "1.0"

[stage.pre]
file = "pre.lua"
priority = 10
unsafe = true

[stage.post]
file = "post.lua"
priority = 10
```

There are two flavors of FFI you can use.

- Through lua c modules: good for extending your plugin with custom native code
- Through LuaJIT FFI module: good for quickly utilizing existing native libraries

### Example: Custom native code
These are dynamic libraries that you can "require" in your lua code. They are usually .so or .dll
files in Linux and windows respectively. You have to write your code, compile it and make it available
to be loaded by lua. The entry point for these modules is a function named `luaopen_XXXX` eg `luaopen_hello` for module hello.
To communicate with lua you need to understand the lua embedding API.
The following is a sample Hello world in c called from LABt's lua.

#### step 1
Create a file named `hello.c` and insert the following code.

```c
// hello.c
#include <lua.h>
#include <lauxlib.h>
#include <lualib.h>
#include <stdio.h>

// Function to be called from Lua
static int l_hello(lua_State *L) {
    printf("Hello from C!\n");
    return 0; // No return values
}

// Function to register the module
int luaopen_hello(lua_State *L) {
    // create a table and map hello key to l_hello c function
		static const luaL_Reg table[] ={
			{"hello", l_hello},
			{NULL, NULL}
		};
		
    luaL_newlib(L, table);
    return 1; // Return no values
}
```

#### step 2: Compile and link
Your code needs to be compiled into a shared object. So you need to have appropriate tools
for your system. For Linux you need LuaJIT installation, and clang.

```bash
clang hello.c -shared -fPIC	-I/usr/include/luajit-2.1 -lluajit-5.1 -o hello.so
```

that will produce a hello.so file that you need.

#### step 3: Load your library.
You should make your library accessible to lua. You can place the module in various lua search paths.
In this example we will make it available on our plugin root directory. 

```
custom-0.1.0
    ├── plugin.toml
    ├── pre.lua
    └── hello.so
```

Do not forget to enable unsafe mode in plugin.toml 

```toml
name = "custom"
version = "0.1.0"

[stage.pre]
file = "pre.lua"
unsafe = true
priority = 10
```

You can now effortlessly "require" your module and lua will handle the rest.

```lua
local module = require("hello");
module.hello(); ---execute your hello function
```

run with `labt build pre`

more info can be found [here](https://epics-lua.readthedocs.io/en/latest/adding-libraries.html?origin=serp_auto)

### Example: Using FFI module
This uses LuaJIT FFI module to dynamically link to shared objects at runtime.
In this example, we will demonstrate how to use `libcurl` to perform a simple HTTP GET request.
Full LuaJIT FFI module documentation can be found [here](http://luajit.org/ext_ffi.html)

#### Step 1: Install libcurl

Make sure you have `libcurl` installed on your system. You can usually install it via your package manager. For example, on Ubuntu, you can run:

```bash
sudo apt-get install libcurl4-openssl-dev
```
or you can provide libcurl binary in your plugin

#### Step 2: Create a plugin script
You should enable unsafe mode in your plugin to make the module available for use.
In your desired plugin stage lua code add the following.

```lua
--- Import ffi module
local ffi = require("ffi")

-- Load the libcurl library
local curl = ffi.load("curl")

-- Define the necessary C functions and structures
ffi.cdef[[
    typedef struct {
        char *url;
    } CURL;

    typedef enum {
      CURLOPT_URL = 10002
    } CURLoption;

    CURL *curl_easy_init();
    void curl_easy_cleanup(CURL *curl);
    int curl_easy_setopt(CURL *curl, CURLoption option, ...);
    int curl_easy_perform(CURL *curl);
    const char *curl_easy_strerror(int error);
    
]]

-- Initialize CURL
--- man curl_easy_init for usage
local curl_handle = curl.curl_easy_init()
if curl_handle == nil then
    error("Failed to initialize CURL")
end

-- Set the URL option
local url = "http://www.example.com"
local result = curl.curl_easy_setopt(curl_handle, curl.CURLOPT_URL, url)
if result ~= 0 then
    error("Failed to set URL: " .. ffi.string(curl.curl_easy_strerror(result)))
end

-- Perform the request
result = curl.curl_easy_perform(curl_handle)
if result ~= 0 then
    error("Failed to perform request: " .. ffi.string(curl.curl_easy_strerror(result)))
end

-- Cleanup
curl.curl_easy_cleanup(curl_handle)
print("Request completed successfully!")

```

#### Step 3: Run the Script

You can run the script using LABt:

```bash
labt build
```

### Conclusion

With the FFI capabilities and the ability to load shared libraries, 
LABt empowers developers to leverage native libraries and legacy tools 
directly within their Lua scripts. This flexibility allows for more powerful 
and efficient plugin development, enhancing the overall functionality of your projects.

With great power comes, great responsibility. Loading external libraries is considered
unsafe in rust context. So good programmer behavior should be considered to ensure
memory safety an prevent memory leaks as it would greatly degrade performance.

Using this module would require devs to ensure cross compatibility with other platforms.

# Templating system
The templating system in LABt allows plugins to initialize their own projects based 
on user input. A plugin can serve as a project initializer by 
defining a templating script and associated template files. This functionality enables plugins 
to initialize and build projects simultaneously, utilizing all the APIs for project 
building provided by LABt, along with additional template-specific APIs.

LABt uses the Tera library for templating, which is inspired by Jinja2 
and Django templates. For detailed information on the templating syntax, please 
refer to the [Tera documentation.](https://keats.github.io/tera/docs/#templates).

## Configuration
You need to provide a templating script that LABt will call when the user executes the following command:

```bash
labt init <your project url> <target dir>
```
In your `plugin.toml` you need to define the `init` table that holds configuration for templating config.

```toml
name="example"
version="0.1.0"
author="omentum"

[init]
file = "template.lua"
	
```

Directory structure:

```
labt-java
├── plugin.toml
├── template.lua
└── templates
    ├── Activity.java
    ├── activity_main.xml
    ├── AndroidManifest.xml
    └── strings.xml
```

### `init` Table

The init table contains the configuration that instructs LABt on how to load the template script. The following keys are allowed:

- **file** (Required): Specifies the Lua file that LABt will call 
	during project initialization.
- **templates** (Optional): Defaults to "templates/*" if not specified
	. This key points to the directory where template files are collected and 
	parsed. The default value expects a templates folder in the plugin'
	s root directory containing all required templates. It should match a globbing 
	pattern that encompasses all your templates.


## Execution
When a user runs the init subcommand against your plugin, LABt follows these steps:

### Step 1: Plugin Installation
LABt fetches and installs your plugin. Note that it does not install 
any of your plugin's SDK dependencies, as these are only 
required during the build process.

### Step 2: Configuration Loading
After installation and version selection, LABt loads the plugin configuration and checks 
for the presence of the `init` table. If the `init` table is 
absent, project initialization fails, indicating that the plugin cannot create a 
project. If the `init` table is present, LABt loads the specified 
template script file and the template files defined in the `templates` field.

### Step 3: Calling the init Function
LABt calls the `init` function defined in the loaded script. The first 
argument is the path provided by the user as the target directory, 
or the current directory if no target path is specified. This function 
should return a Lua table equivalent to `Labt.toml`. The init function can have 
a second return value that point to a new target directory that the `Labt.toml` 
will be written to. If not provided LABt will write it to the path passed to the
init function path argment.

Example init Function

```lua
	function init(path)
		-- get input from users
	  return {
	    project = {
	      name = "test",
	      description = "",
	      version = "0.1.0",
	      version_number = 1,
	      package = "com.example",
	    },
			-- a hash table of plugins
	    plugins = {
	      ["labt-java"] = {
	        version = "0.1.0",
	        location = "https://gitlab.com/lab-tool/plugins/labt-java"
	      },
	    },
			-- a hash table of dependencies
	    dependencies = {
	      appcompact = {
	        version = "1.1.0",
	        group_id = "androidx.appcompact"
	      },
	    }
	  }
	end
```

The returned table will be output to `Labt.toml`, giving you 
full control over the project configuration. You can configure aspects such as 
plugins, dependencies, and resolvers.

### Step 4: Finalizing Project Initialization
LABt outputs the returned configuration to Labt.toml in the target directory
, completing the project initialization process. The user can now run the 
following commands to fetch plugins and resolve dependencies: 

```
labt plugin fetch
labt resolve
```

## Modules Available to the Loaded Templating Script.
When LABt loads a plugin, it provides access to all standard tables 
from the plugin API. Note that unsafe mode is disabled, meaning 
that the `ffi` module and loading of Lua shared objects are not available
. Additionally, an extra global Lua table is introduced to facilitate interaction 
with the templating API.

### `template` table
The template table contains functions that allow you to call rendering functions for 
your templates. The following function is available through this table:

***
#### `render` function
**arguments**: string: template name, table: template data  
**returns**: string

The render function calls the rendering function of the compiled templates. It 
expects the template data to be provided for substitution within the template. 

Returns an error if:

- unable to load the provided template name

**Example Template:** templates/Activity.java

```java
// templates/Activity.java
package {{ package_name }};

import android.app.Activity;
import android.os.Bundle;

import {{ package_name }}.R;

public class {{ class_name }} extends Activity {

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        {% if xml_layout %}
        setContentView(R.layout.{{ layout }});
        {% endif %}

    }

}

```

**Example Usage in Your Template Script:**

```lua
function init(path)
	local activity = template.render("Activity.java", {
		package_name = "com.example",
		xml_layout = "activity_main",
		class_name = "MainActivity",
	});

	 -- Write the generated code to MainActivity.java
end
```
