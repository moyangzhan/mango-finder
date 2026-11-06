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

> 「Lite 变体」构建不含这些文件也完全可行：应用检测到文件缺失时会将对应本地后端置灰并引导使用远程/自托管后端，见[第 7 节](#7-构建发布版本)。

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

### Full / Lite 双变体

「0 本地模型」是受支持的运行形态，不是独立版本：一份代码、一个运行时、两种发行变体。

| 变体 | 内容 | 体积 | 适用 |
| --- | --- | --- | --- |
| **Full** | 可执行文件 + 全部本地模型 | ~1GB | 个人离线用户，开箱即用 |
| **Lite** | 仅可执行文件 | 预计 50-70MB（MSI LZX 压缩后） | 团队场景（Ollama 集中部署 + 成员自托管调用）、纯远程/仅关键词用户 |

两种变体使用同一 identifier/UpgradeCode，互为原地升级：Full→Lite（或反向）直接安装即可，MajorUpgrade 会清理多余文件，数据库位于数据目录不受影响。

**Lite 的运行行为：**

- 关键词检索（FTS）不依赖任何模型，始终可用
- 设置页中「向量/视觉/音频」三个类目各自独立选择本地/自托管/远程后端；本地选项按**模型文件检测**自动灰亮（缺任一文件即置灰并提示缺失清单），检测在启动与每次打开设置页时刷新——手动把模型文件放回数据目录的 `model` 文件夹后选项即亮起，无需重启
- 向量（embedding）后端支持本地 ONNX 与平台 API（OpenAI 兼容 `/v1/embeddings`，Ollama/vLLM 同协议）；切换向量后端 = 切换向量库，应用内按「模型 · 后端」分库并存、默认保留，切回已有库即时生效（详见 GUI 设置页「向量库」标签）
- 远程视觉/音频由多模态模型/ASR 端点覆盖；OCR 无独立远程通道（远程模式由多模态模型顺带覆盖，精度不等价）

**CI 双产物**：`build.yml` 每个平台连续构建两次——先 Full，再剥离 `bundle.resources` 构建 Lite，产物以 `_Full` / `_Lite` 后缀命名（如 `Mango.Finder_0.12.0_x64_en-US_Full.msi` / `..._Lite.msi`）。

**本地构建 Lite**：

```sh
python scripts/strip_bundle_resources.py   # 就地移除 tauri.conf.json 的 bundle.resources
pnpm tauri build
git checkout -- src-tauri/tauri.conf.json  # 构建后恢复配置
```

> 之所以就地修改配置：Tauri 的 `--config` 是深合并，无法删除已声明的键（空对象合并为无操作）。
