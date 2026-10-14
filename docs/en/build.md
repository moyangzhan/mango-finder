<sup>[English](../en/build.md) | [中文](../cn/build.md)</sup>

# Building Mango Finder

## 1. Frontend

### Node

`node` v20+ required

It is recommended to use [nvm](https://github.com/nvm-sh/nvm) to manage multiple `node` versions.

### PNPM

`pnpm` v9+ required

If you haven't installed `pnpm`, you can install it with the following command:

```shell
npm install pnpm -g
```

### Install dependencies

```sh
pnpm i
```

## 2. Backend (Rust)

`rust` v1.94.0+ required

Install tools: <https://www.rust-lang.org/tools/install>

## 3. Tauri

Install Tauri Prerequisites:
<https://tauri.app/start/prerequisites/>

## 4. Download Model Files

Download the required files from [Hugging Face: moyangzhan/mango-finder](https://huggingface.co/moyangzhan/mango-finder/tree/main) and place them in the `src-tauri/assets/model` directory.

**Required Files** (authoritative list: `bundle.resources` in `tauri.conf.json`):

- embedding.onnx
- embedding\_tokenizer.json
- vision.onnx
- vision\_tokenizer.json
- whisper-small-q8\_0.bin
- ch_PP-OCRv4_det_infer.onnx
- ch_ppocr_mobile_v2.0_cls_infer.onnx
- ch_PP-OCRv4_rec_infer.onnx
- ppocr_keys_v1.txt

The last four are the OCR models used by the local image text-recognition parser.

## 5. Whisper.cpp Dependencies

The audio transcription feature uses [whisper.cpp](https://github.com/ggerganov/whisper.cpp). Different operating systems require different dependencies.

### Windows

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

### macOS

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
5. **Build**:

   ```sh
   pnpm tauri build
   # Or explicitly specify target
   pnpm tauri build --target aarch64-apple-darwin
   ```

> **Note**: The minimum supported macOS version is 10.15 (Catalina).

### Linux

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

## 6. Running (Development)

### GUI

A Tauri app has at least [two processes](https://tauri.app/concept/process-model/):

- the Core Process (`backend`)
- the WebView process (`frontend`)

Both backend and frontend start with a single command:

```sh
pnpm tauri dev
```

### CLI

The CLI has no dev server — its dev loop is the debug build plus direct invocation:

```sh
# any subcommand, straight from the debug build
cargo run --bin mfd -- search "hello" --type keyword

# run the daemon in the foreground — the auto-spawned daemon is a hidden process
# with invisible output, so a manual `serve` is how you watch daemon logs
cargo run --bin mfd -- serve
```

`RUST_LOG` sets the log level (default `info`); the other CLI environment variables are listed in [cli.md](./cli.md).

## 7. Release Build

```sh
pnpm tauri build
```

```text
pnpm tauri build                    ← auto-merges tauri.<platform>.conf.json to pick installer formats
├─ beforeBuildCommand: pnpm build (frontend) + cargo build --release --bin mfd (builds the CLI first)
├─ cargo build (the GUI app itself)
└─ bundle: packages the installer; mfd rides in automatically as a cargo bin, models copied via resources
```

One command builds a single installer carrying both the GUI and the CLI (`mfd`): after installation, `mfd` and the model files sit in the install directory; add it to `PATH` from the GUI settings page ("Command Line Tool") to use `mfd` in any terminal.

Installers land in `src-tauri/target/release/bundle/` in platform-specific formats, e.g. on windows:
`src-tauri/target/release/bundle/msi/Mango.Finder_0.12.0_x64_en-US.msi`

For CLI usage, see [cli.md](./cli.md).
