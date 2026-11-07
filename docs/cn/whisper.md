<sup>[English](../en/whisper.md) | [中文](../cn/whisper.md)</sup>

# Whisper.cpp 编译依赖

音频转文字功能使用 [whisper.cpp](https://github.com/ggerganov/whisper.cpp)：`whisper-rs` 是 cargo 编译依赖，与模型文件无关——开发模式与 Full / Lite 构建都需要它。不同操作系统需要安装的依赖不同。

构建流程本身见 [build.md](./build.md)。

## Windows

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

## macOS

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
5. **验证构建（仅首次需要）**：

   ```sh
   pnpm build:full
   # 或显式指定 target
   pnpm tauri build --config src-tauri/tauri.full.conf.json --target aarch64-apple-darwin
   ```

> **注意**：最低支持的 macOS 版本为 10.15（Catalina）。

## Linux

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
