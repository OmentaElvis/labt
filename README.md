[![Labt Logo](assets/logo_400x200.png)](https://gitlab.com/lab-tool/labt)
[![Crates.io Version](https://img.shields.io/crates/v/labt?logo=rust)](https://crates.io/crates/labt)
![GitLab License](https://img.shields.io/gitlab/license/lab-tool%2Flabt)
[![GitLab pipeline](https://gitlab.com/lab-tool/labt/badges/main/pipeline.svg)](https://gitlab.com/lab-tool/labt/-/pipelines)

# Lightweight Android Build tool (LABt)
**LABt** is a command line interface tool written in Rust for building your android projects
on the terminal. It provides dependency management tools to easily add and 
resolve your project dependencies. It aims to work fully offline and only
requiring network during resolution when the dependencies were not cached
locally.

## Goals
- Be very lightweight 
- Be very simple and clear on what its happening internally for easier debugging of problems in projects. We dont like black boxes of magic that the user can't explain what is happening.
- Be correct.
- Be cache friendly at all times.
- Be faster.
- Be IDE independent.

## Features
- M2 compatible dependency resolver
- SDK package managers for both google and third party sdk
- Simple build process
- Plugin system with Lua based scripting
- Priority to caching and less time doing "network stuff"
- Project templating using Jinja2 and Django template syntax.
- Only run plugins stage if code changes

At its core, labt provides a plugin system to build applications. The plugins
system provides lua scripting for easy implementation and lightweight plugins.

_NB: This project is a working progress and currently unstable. More feedback is needed before we hit v1_

## Installation
Install using cargo

```bash
cargo install labt
```

build from source

```bash
git clone https://gitlab.com/lab-tool/labt
cd labt
cargo build --release  
# or 
# cargo build --release --features vendored-lua
```

Check the releases page for prebuilt binaries.



### Os support
Currently the base support is on *Linux* based OS

future full cross platform support is planned

There are releases available for windows but they are mostly not properly tested.
The platform support may differ depending on plugin used.

## Usage
Initialize a new a new android project

```bash
labt init <initializer url>

labt init https://gitlab.com/lab-tool/plugins/labt-java@latest
```
This creates a new project. 


Add a dependency to your project

```bash 
labt add androidx.appcompat:appcompat:1.1.0
```

the add subcommand automatically downloads and caches the provided dependency.
LABt maintains this global cache of dependency files and plugins can query for 
these files during build.
You can also fetch the dependencies manually by running.

```bash
labt resolve
```

Labt on its own just manages your projects, its dependencies and sdkmodules. To do a build of your app, 
you will need a plugin. Choose a plugin of your choice from any git compatible repository
and `use` it for your build proccess. For example, use [labt-java](https://gitlab.com/lab-tool/plugins/labt-java)
to build a java application.

```
labt plugin https://gitlab.com/lab-tool/plugins/labt-java@v0.1.0
```

Now you can run `labt build` and the plugin will build the application for you. If you have special 
requirements to build your application check the [LABt Lua API documentation](doc/LuaAPI.md) on how to
create a custom plugin.

for more information you could try `labt help`

```bash
Usage: labt [COMMAND]

Commands:
  add      Adds a new project dependency
  init     Initializes a new project
  resolve  Fetches the project dependencies
  build    Builds the project
  plugin   Manage plugins
  sdk      Sdk manager
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

```


## SDK Manager
LABt's SDK Manager lets you manage Android SDK packages via a 
terminal interface. Sdk packages provide development tools to plugins e.g. aapt, r8, d8 and adb from google Android SDK packages.

- **Add repository**: Use `labt sdk add google` for default android google repo or any third party repository by `labt sdk add <name> <url>`.
- **Interactive Management**: Use `labt sdk list <repo-name>` to view 
  and toggle package actions (install, uninstall, upgrade/downgrade) 
  in a TUI. Use space to toggle actions and press enter to confirm.
- **Installing**: `labt sdk install <repo-name> --path <id> --version <version>` to install a package non interactively.
- **Lua API Integration**: Plugins can access SDK packages directly through 
  LABt's Lua API. [More details here](doc/LuaAPI.md).
  
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


## Plugin system
Labt on its own cant really do much. It provides tools to manage projects and their
dependencies. To extend the capability of labt, it provides a powerful Lua scripting
plugin system. This allows custom plugins to do the heavy lifting of building applications.
For more information on plugin system check the [LABt Lua API documentation](doc/LuaAPI.md).

## TODO
- [x] Add a FFI capability for plugins
- [x] Support for windows file system
- [x] Add a configurable template system
- [x] Stabilize the plugin api and interpret versions of plugins
- [x] Shorten the plugin use command
- [x] Sdkmanager support multiple repositories
- [ ] Add a way to tell plugins what build profile they are on (debug, release, preview)
