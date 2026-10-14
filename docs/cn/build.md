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

## 4. 下载模型文件

从 [Hugging Face: moyangzhan/mango-finder](https://huggingface.co/moyangzhan/mango-finder/tree/main) 下载所需文件，放入 `src-tauri/assets/model` 目录。

**所需文件列表**（权威清单：`tauri.conf.json` 的 `bundle.resources`）：

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

## 5. Whisper.cpp 依赖

音频转文字功能使用 [whisper.cpp](https://github.com/ggerganov/whisper.cpp)，不同操作系统需要安装不同的依赖。

### Windows

在 Windows 上编译需要安装 **CMake** 和 **LLVM/Clang 18**（注意：LLVM 19/20/22 版本存在兼容性问题，请使用 LLVM 18）。

1. **安装 CMake 4.3**

   从 [cmake-4.3.0](https://github.com/Kitware/CMake/releases/tag/v4.3.0-rc2) 下载安装
2. **下载并安装 LLVM 18**

   - 访问 [LLVM 18.1.8 Release](https://github.com/llvm/llvm-project/releases/tag/llvmorg-18.1.8)
   - 下载 `LLVM-18.1.8-win64.exe`
   - 安装时勾选 **"Add LLVM to the system PATH for all users"**
3. **验证安装**

   ```sh
   cmake --version
   clang --version
   ```

   clang 版本应显示 `18.1.8`
4. **设置环境变量（永久）**

   - 按 `Win + R`，输入 `sysdm.cpl`，回车
   - 点击 **"高级"** 选项卡 → **"环境变量"**
   - 在 **"用户变量"** 区域点击 **"新建"**，添加：

   | 变量名        | 值        |
   | ---------- | -------- |
   | `CXXFLAGS` | `/utf-8` |
   | `CFLAGS`   | `/utf-8` |

   - 点击确定，**重启终端** 使环境变量生效
5. **编译项目（仅首次需要）**

   打开 **"x64 Native Tools Command Prompt for VS 2022"**（从开始菜单搜索），然后编译：

   ```cmd
   cd your-project-path\src-tauri
   cargo build
   ```

   > ⚠️ **重要提示**：
   >
   > - `/utf-8` 参数是必需的，用于解决中文编码问题
   > - 如果之前编译失败，先运行 `cargo clean -p whisper-rs-sys` 清理缓存
   > - whisper 编译成功后，后续可直接在任意终端使用 `pnpm tauri dev`
   > - VSCode 的 rust-analyzer 插件会在启动时自动检查代码，由于没有 MSVC 环境，whisper-rs-sys 的构建会失败并以红色标识显示在 `target/debug/build` 目录中。如果你已在 "x64 Native Tools Command Prompt for VS 2022" 中构建成功，可以忽略此错误

### macOS

1. **安装 Xcode Command Line Tools**（如果尚未安装）：

   ```sh
   xcode-select --install
   ```
2. **安装 CMake**：

   ```sh
   brew install cmake
   ```
3. **设置环境变量**（Apple Silicon 必需）：

   对于 Apple Silicon Mac（M1/M2/M3），需要设置以下环境变量：

   | 变量名                        | 值                             | 用途                          |
   | -------------------------- | ----------------------------- | --------------------------- |
   | `CFLAGS`                   | `-U__ARM_FEATURE_MATMUL_INT8` | 避免 whisper.cpp 在 ARM 上的编译问题 |
   | `MACOSX_DEPLOYMENT_TARGET` | `10.15`                       | 设置最低支持的 macOS 版本（Catalina）  |

   **临时设置（当前终端会话）：**

   ```sh
   export CFLAGS="-U__ARM_FEATURE_MATMUL_INT8"
   export MACOSX_DEPLOYMENT_TARGET="10.15"
   ```

   **永久设置（添加到 shell 配置）：**

   ```sh
   # zsh（macOS 默认）
   echo 'export CFLAGS="-U__ARM_FEATURE_MATMUL_INT8"' >> ~/.zshrc
   echo 'export MACOSX_DEPLOYMENT_TARGET="10.15"' >> ~/.zshrc
   source ~/.zshrc

   # bash
   echo 'export CFLAGS="-U__ARM_FEATURE_MATMUL_INT8"' >> ~/.bash_profile
   echo 'export MACOSX_DEPLOYMENT_TARGET="10.15"' >> ~/.bash_profile
   source ~/.bash_profile
   ```
4. **添加 Rust target**：

   ```sh
   rustup target add aarch64-apple-darwin
   ```
5. **构建**：

   ```sh
   pnpm tauri build
   # 或显式指定 target
   pnpm tauri build --target aarch64-apple-darwin
   ```

> **注意**：最低支持的 macOS 版本为 10.15（Catalina）。

### Linux

大多数 Linux 发行版需要安装 C/C++ 编译工具：

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

## 6. 运行（开发模式）

### GUI

Tauri 应用至少包含两个进程（详见 [官方文档](https://tauri.app/concept/process-model/)）：

- **Core Process** ：Rust 后端
- **WebView Process** ：前端界面

使用一条命令即可同时启动前后端：

```sh
pnpm tauri dev
```

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

## 7. 构建发布版本

```sh
pnpm tauri build
```

```text
pnpm tauri build                    ← 自动合并 tauri.<platform>.conf.json，选定安装包格式
├─ beforeBuildCommand：pnpm build（前端）+ cargo build --release --bin mfd（先构建 CLI）
├─ cargo build（GUI 应用本体）
└─ bundle：打安装包；mfd 以 cargo bin 身份自动进包，模型文件由 resources 拷入
```

一条命令构建出同时包含 GUI 与 CLI（`mfd`）的安装包：安装后 `mfd` 与模型文件位于安装目录，在 GUI 设置页「命令行工具」将其加入 `PATH` 即可在任意终端使用。

安装包位于 `src-tauri/target/release/bundle/`，格式随平台不同，如 windows：
`src-tauri/target/release/bundle/msi/Mango.Finder_0.12.0_x64_en-US.msi`

CLI 用法见 [cli.md](./cli.md)。
