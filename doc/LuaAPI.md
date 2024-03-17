# LABt Lua API Documentation

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
