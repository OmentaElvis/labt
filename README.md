![Labt Logo](assets/logo_400x200.png)
# Lightweight Android Build tool (LABt)
**LABt** is a command line interface tool written in Rust for building your android projects
on the terminal. It provides dependency management tools to easily add and 
resolve your project dependencies. It aims to work fully offline and only
requiring network during resolution when the dependencies were not cached
locally.

At its core, labt provides a plugin system to build applications. The plugins
system provides lua scripting for easy to implement and lightweight plugins.

_NB: This project is a working progress and currently very unstable_

## Installation
Install using cargo

```bash
cargo install labt
```

### Os support
Currently the base support is on *Linux* based OS

future cross platform support is planned

## Usage
Initialize a new a new android project

```bash
labt init
```
This creates a new project. 


Add a dependency to your project

```bash 
labt add androidx.appcompat:appcompat:1.1.0
```
the add subcommand automatically downloads and caches the provided dependency.
You can also fetch the dependencies manually by running.

```bash
labt resolve
```

for more information you could try `labt help`

```bash
Usage: labt [COMMAND]

Commands:
  add      Adds a new project dependency
  init     Initializes a new project
  resolve  Fetches the project dependencies
  build    Builds the project
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help

```

## Plugin system
Labt on its own cant really do much. It provides tools to manage projects and their
dependencies. To extend the capability of labt, it provides a powerful Lua scripting
plugin system. This allows custom plugins to do the heavy lifting of building applications.
For more information on plugin system check the [LABt Lua API documentation](doc/LuaAPI.md).

## TODO
- [ ] Add a FFI capability for plugins
- [ ] Support for windows file system
- [ ] Add a configurable template system
