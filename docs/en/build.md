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

## 4. Whisper.cpp Dependencies

The `whisper-rs` crate is a cargo build dependency of the audio transcription feature — unrelated to the model files; dev mode and both Full / Lite builds need it. Requirements per OS; full steps in **[whisper.md](./whisper.md)**:

| OS | Key requirements |
| --- | --- |
| Windows | CMake 4.3 + LLVM/Clang **18** (19/20/22 have compatibility issues); env vars `CXXFLAGS`/`CFLAGS` = `/utf-8` |
| macOS | Xcode CLT + CMake; Apple Silicon additionally needs `CFLAGS` and `MACOSX_DEPLOYMENT_TARGET`, minimum 10.15 |
| Linux | Distro C/C++ build tools + CMake |

## 5. Running (Development)

### GUI

A Tauri app has at least [two processes](https://tauri.app/concept/process-model/):

- the Core Process (`backend`)
- the WebView process (`frontend`)

Both backend and frontend start with a single command:

```sh
pnpm tauri dev   # default Full identity (i.e. pnpm dev:full)
pnpm dev:lite    # Lite identity: own data dir mango-finder-lite, port 15679, Lite window title
```

In dev mode, local models resolve straight from `src-tauri/assets/model` (just drop them in; no copy to target needed) — capability still follows file detection: to test the "local options greyed" shape under `dev:lite`, temporarily move that directory away. Do not run both identities' dev at once (they share the vite port 5173).

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

## 6. Release Build

Both editions share one build pipeline; every difference is expressed on the build command via the `lite` cargo feature and the config overlays (`tauri.full.conf.json` / `tauri.lite.conf.json`) — no file rewriting scripts. Note: a bare `pnpm tauri build` yields a Full-identity installer WITHOUT models; use `pnpm build:full` / `pnpm build:lite`.

### Full / Lite variants

Two coinstallable editions (one codebase, two build configurations): independent installs that never affect each other, each with its own data directory (`mango-finder` / `mango-finder-lite`); on Linux the two debs conflict over `/usr/bin/mfd` and cannot coinstall (coexistence is a Windows/macOS capability). For the edition comparison and Lite runtime behavior, see the "Editions" section of the [README](../../README.md).

### Building Full

First download the model files: fetch the required files from [Hugging Face: moyangzhan/mango-finder](https://huggingface.co/moyangzhan/mango-finder/tree/main) and place them in the `src-tauri/assets/model` directory.

**Required Files** (authoritative list: `bundle.resources` in `src-tauri/tauri.full.conf.json`):

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

> The model list lives in the Full overlay rather than the main config because `--config` deep-merge can only add/override — it cannot delete keys declared in the main config. "Not declared in the base, added by the Full build" is what lets the Lite build rewrite nothing.

```sh
pnpm build:full   # i.e. pnpm tauri build --config src-tauri/tauri.full.conf.json
```

```text
pnpm build:full                     ← auto-merges tauri.<platform>.conf.json and tauri.full.conf.json (model manifest)
├─ beforeBuildCommand: pnpm build (frontend) + cargo build --release --bin mfd (builds the CLI first)
├─ cargo build (the GUI app itself)
└─ bundle: packages the installer; mfd rides in automatically as a cargo bin, models copied via resources
```

One command builds a single installer carrying both the GUI and the CLI (`mfd`): after installation, `mfd` and the model files sit in the install directory; add it to `PATH` from the GUI settings page ("Command Line Tool") to use `mfd` in any terminal.

Installers land in `src-tauri/target/release/bundle/` in platform-specific formats, e.g. on windows:
`src-tauri/target/release/bundle/msi/Mango.Finder_0.12.0_x64_en-US.msi`

For CLI usage, see [cli.md](./cli.md).

### Building Lite

The Lite build needs no model files; compile-time dependencies are identical to Full (whisper-rs is a cargo build dependency — separate from the model files — so the Section 4 toolchain is equally required):

```sh
pnpm build:lite   # i.e. pnpm tauri build --features lite --config src-tauri/tauri.lite.conf.json
```

Two sources of difference:

- **the `lite` cargo feature** (applies to the GUI and `mfd` alike at compile time): the `IS_LITE` global switch, data directory name `mango-finder-lite`, default service port `15679` (including the fresh-DB `cluster_setting` seed; see `global.rs`)
- **the `src-tauri/tauri.lite.conf.json` overlay**: Lite-specific productName / identifier / window title

`mfd` ships in the package too; the installer name carries Lite, e.g. on windows:
`src-tauri/target/release/bundle/msi/Mango.Finder.Lite_0.12.0_x64_en-US.msi`

> The in-place edit exists because Tauri's `--config` deep-merges and cannot delete declared keys (an empty object merges as a no-op).

### Building both (CI)

`build.yml` builds twice per platform in the same job (cargo cache reused):

| Edition | Build command | Example artifact (windows) |
| --- | --- | --- |
| Full | `pnpm tauri build --config src-tauri/tauri.full.conf.json` | `Mango.Finder_0.12.0_x64_en-US_Full.msi` |
| Lite | `pnpm tauri build --features lite --config src-tauri/tauri.lite.conf.json` | `Mango.Finder.Lite_0.12.0_x64_en-US_Lite.msi` |

Full's model files are downloaded from Hugging Face before the builds (Full only).
