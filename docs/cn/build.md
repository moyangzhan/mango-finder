<sup>[English](../en/build.md) | [中文](../cn/build.md)</sup>

# 构建 Mango Finder

## 1. 前端环境准备

### Node

``node` **v20 及以上版本**

推荐使用 [nvm](https://github.com/nvm-sh/nvm) 来管理多个 `node` 版本。

#### PNPM

需要 `pnpm` **v9 及以上版本**

如果你还没有安装 `pnpm`，可以使用以下命令安装：

```shell
npm install pnpm -g
```

#### 安装依赖

```sh
pnpm i
```

## 2. 后端环境准备（Rust）

需要 `rust` v1.94.0 及以上

建议使用官方工具安装：<https://www.rust-lang.org/tools/install>

## 3. Tauri

在运行项目前，请先根据你的操作系统安装 Tauri 所需依赖：

<https://tauri.app/start/prerequisites/>

## 4. Whisper.cpp 依赖

音频转文字功能的 `whisper-rs` 是 cargo 编译依赖——与模型文件无关。各系统要求一览，完整步骤见 **[whisper.md](./whisper.md)**：

| 系统 | 关键依赖 |
| --- | --- |
| Windows | CMake 4.3 + LLVM/Clang **18**（19/20/22 有兼容问题）；环境变量 `CXXFLAGS`/`CFLAGS` = `/utf-8` |
| macOS | Xcode CLT + CMake；Apple Silicon 另需设置 `CFLAGS` 与 `MACOSX_DEPLOYMENT_TARGET`，最低支持 10.15 |
| Linux | 发行版 C/C++ 编译工具 + CMake |

## 5. 运行（开发模式）

### GUI

Tauri 应用至少包含两个进程（详见 [官方文档](https://tauri.app/concept/process-model/)）：

- **Core Process** ：Rust 后端
- **WebView Process** ：前端界面

使用一条命令即可同时启动前后端：

```sh
pnpm tauri dev   # 默认 Full 身份（即 pnpm dev:full）
pnpm dev:lite    # Lite 身份：独立数据目录 mango-finder-lite、端口 15679、窗口标题带 Lite
```

开发模式的本地模型直接从 `src-tauri/assets/model` 解析（放入即可，无需拷贝到 target）——能力仍按文件检测：`dev:lite` 下想测「本地选项置灰」形态，需临时移走该目录。两个身份的 dev 不要同时运行（共用 vite 端口 5173）。

### CLI

CLI 没有 dev server，开发方式是 debug 构建直接调用：

```sh
# 运行任意子命令
cargo run --bin mfd -- search "hello" --type keyword

# 前台运行 daemon——自动拉起的 daemon 是隐藏进程、输出不可见，
# 手动 `serve` 是观察 daemon 日志的方式
cargo run --bin mfd -- serve
```

`RUST_LOG` 控制日志级别（默认 `info`）；其余 CLI 环境变量见 [cli.md](./cli.md)。

## 6. 构建发布版本

两版本共用同一构建管线，差异全部由 cargo feature（`lite`）与配置覆盖文件（`tauri.full.conf.json` / `tauri.lite.conf.json`）在构建命令上表达，无需脚本改写任何文件。注意：裸跑 `pnpm tauri build` 会得到「Full 身份但不含模型」的安装包，请使用 `pnpm build:full` / `pnpm build:lite`。

### Full / Lite 双版本

两个**可共存安装**的发行版本（同一份代码、两种构建配置）：相互独立安装、卸载互不影响，数据目录各自一份（`mango-finder` / `mango-finder-lite`）；Linux 下两个 deb 因 `/usr/bin/mfd` 同名冲突不可共存（共存为 Windows/macOS 能力）。版本区别与 Lite 运行行为见 [README](../../README_CN.md) 的「发行版本」一节。

### 构建 Full 版本

先下载模型文件：从 [Hugging Face: moyangzhan/mango-finder](https://huggingface.co/moyangzhan/mango-finder/tree/main) 下载所需文件，放入 `src-tauri/assets/model` 目录。

**所需文件列表**（权威清单：`src-tauri/tauri.full.conf.json` 的 `bundle.resources`）：

- embedding.onnx
- embedding\_tokenizer.json
- vision.onnx
- vision\_tokenizer.json
- whisper-small-q8\_0.bin
- ch_PP-OCRv4_det_infer.onnx
- ch_ppocr_mobile_v2.0_cls_infer.onnx
- ch_PP-OCRv4_rec_infer.onnx
- ppocr_keys_v1.txt

后四个为 OCR 模型（本地图片文字识别使用）。

> 模型清单放在 Full 覆盖文件而非主配置，是因为 `--config` 深合并只能添加/覆盖、无法删除主配置已声明的键——「主配置不声明、Full 构建时添加」让 Lite 构建无需任何文件改写。

```sh
pnpm build:full   # 即 pnpm tauri build --config src-tauri/tauri.full.conf.json
```

```text
pnpm build:full                     ← 自动合并 tauri.<platform>.conf.json 与 tauri.full.conf.json（模型清单）
├─ beforeBuildCommand：pnpm build（前端）+ cargo build --release --bin mfd（先构建 CLI）
├─ cargo build（GUI 应用本体）
└─ bundle：打安装包；mfd 以 cargo bin 身份自动进包，模型文件由 resources 拷入
```

一条命令构建出同时包含 GUI 与 CLI（`mfd`）的安装包：安装后 `mfd` 与模型文件位于安装目录，在 GUI 设置页「命令行工具」将其加入 `PATH` 即可在任意终端使用。

安装包位于 `src-tauri/target/release/bundle/`，格式随平台不同，如 windows：
`src-tauri/target/release/bundle/msi/Mango.Finder_0.12.0_x64_en-US.msi`

CLI 用法见 [cli.md](./cli.md)。

### 构建 Lite 版本

Lite 构建无需下载模型文件；但 whisper.cpp 等编译依赖与 Full 完全相同（whisper-rs 是 cargo 编译依赖，与模型文件是两回事，第 4 节环境同样必需）：

```sh
pnpm build:lite   # 即 pnpm tauri build --features lite --config src-tauri/tauri.lite.conf.json
```

两处差异源：

- **cargo feature `lite`**（GUI 与 `mfd` 编译期同时生效）：全局开关 `IS_LITE`、数据目录名 `mango-finder-lite`、默认服务端口 `15679`（含全新库 `cluster_setting` 种子，见 `global.rs`）
- **`src-tauri/tauri.lite.conf.json` 覆盖**：productName / identifier / 窗口标题改为 Lite 专属

`mfd` 同样进包，产物安装名带 Lite，如 windows：
`src-tauri/target/release/bundle/msi/Mango.Finder.Lite_0.12.0_x64_en-US.msi`

### 同时构建两个版本（CI）

`build.yml` 每个平台在同一 job 内连续构建两次（复用 cargo 缓存）：

| 版本 | 构建命令 | 产物示例（windows） |
| --- | --- | --- |
| Full | `pnpm tauri build --config src-tauri/tauri.full.conf.json` | `Mango.Finder_0.12.0_x64_en-US_Full.msi` |
| Lite | `pnpm tauri build --features lite --config src-tauri/tauri.lite.conf.json` | `Mango.Finder.Lite_0.12.0_x64_en-US_Lite.msi` |

Full 的模型文件在构建前从 Hugging Face 下载（仅 Full 需要）。
