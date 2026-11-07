<sup>[English](../en/whisper.md) | [中文](../cn/whisper.md)</sup>

# Whisper.cpp Build Dependencies

The audio transcription feature uses [whisper.cpp](https://github.com/ggerganov/whisper.cpp): `whisper-rs` is a cargo build dependency, unrelated to the model files — dev mode and both Full / Lite builds need it. Different operating systems require different dependencies.

For the build flow itself, see [build.md](./build.md).

## Windows

Compiling on Windows requires **CMake** and **LLVM/Clang 18** (Note: LLVM 19/20/22 have compatibility issues, please use LLVM 18).

1. **Install CMake 4.3**

   Download from [cmake-4.3.0](https://github.com/Kitware/CMake/releases/tag/v4.3.0-rc2)
2. **Download and Install LLVM 18**

   - Visit [LLVM 18.1.8 Release](https://github.com/llvm/llvm-project/releases/tag/llvmorg-18.1.8)
   - Download `LLVM-18.1.8-win64.exe`
   - Check **"Add LLVM to the system PATH for all users"** during installation
3. **Verify installation**

   ```sh
   cmake --version
   clang --version
   ```

   The clang version should show `18.1.8`
4. **Set environment variables (permanent)**

   - Press `Win + R`, type `sysdm.cpl`, press Enter
   - Click **"Advanced"** tab → **"Environment Variables"**
   - Under **"User variables"**, click **"New"** and add:

   | Variable name | Value    |
   | ------------- | -------- |
   | `CXXFLAGS`    | `/utf-8` |
   | `CFLAGS`      | `/utf-8` |

   - Click OK and **restart your terminal** for changes to take effect
5. **Build the project (first time only)**

   Open **"x64 Native Tools Command Prompt for VS 2022"** (search from Start Menu), then build:

   ```cmd
   cd your-project-path\src-tauri
   cargo build
   ```

   > ⚠️ **Important Notes**:
   >
   > - The `/utf-8` flag is required to resolve encoding issues
   > - If previous build failed, run `cargo clean -p whisper-rs-sys` to clear cache first
   > - After whisper is compiled successfully, subsequent builds can use `pnpm tauri dev` directly in any terminal
   > - VSCode's rust-analyzer plugin auto-checks code on startup. Without MSVC environment, whisper-rs-sys build will fail and show as red in `target/debug/build` directory. If you've successfully built in "x64 Native Tools Command Prompt for VS 2022", you can ignore this error

## macOS

1. **Install Xcode Command Line Tools** (if not already installed):

   ```sh
   xcode-select --install
   ```
2. **Install CMake**:

   ```sh
   brew install cmake
   ```
3. **Set environment variables** (required for Apple Silicon):

   For Apple Silicon Macs (M1/M2/M3), you need to set the following environment variables:

   | Variable                   | Value                         | Purpose                                        |
   | -------------------------- | ----------------------------- | ---------------------------------------------- |
   | `CFLAGS`                   | `-U__ARM_FEATURE_MATMUL_INT8` | Avoid whisper.cpp compilation issues on ARM    |
   | `MACOSX_DEPLOYMENT_TARGET` | `10.15`                       | Set minimum supported macOS version (Catalina) |

   **Temporary (current terminal session):**

   ```sh
   export CFLAGS="-U__ARM_FEATURE_MATMUL_INT8"
   export MACOSX_DEPLOYMENT_TARGET="10.15"
   ```

   **Permanent (add to your shell config):**

   ```sh
   # For zsh (default on macOS)
   echo 'export CFLAGS="-U__ARM_FEATURE_MATMUL_INT8"' >> ~/.zshrc
   echo 'export MACOSX_DEPLOYMENT_TARGET="10.15"' >> ~/.zshrc
   source ~/.zshrc

   # For bash
   echo 'export CFLAGS="-U__ARM_FEATURE_MATMUL_INT8"' >> ~/.bash_profile
   echo 'export MACOSX_DEPLOYMENT_TARGET="10.15"' >> ~/.bash_profile
   source ~/.bash_profile
   ```
4. **Add Rust target**:

   ```sh
   rustup target add aarch64-apple-darwin
   ```
5. **Verify with a build (first time only)**:

   ```sh
   pnpm build:full
   # Or explicitly specify target
   pnpm tauri build --config src-tauri/tauri.full.conf.json --target aarch64-apple-darwin
   ```

> **Note**: The minimum supported macOS version is 10.15 (Catalina).

## Linux

Most Linux distributions require C/C++ build tools:

**Ubuntu/Debian:**

```sh
sudo apt update
sudo apt install build-essential cmake
```

**Fedora/RHEL:**

```sh
sudo dnf install gcc-c++ make cmake
```

**Arch Linux:**

```sh
sudo pacman -S base-devel cmake
```
