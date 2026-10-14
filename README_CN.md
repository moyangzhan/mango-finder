<sup>[English](./README.md) | [中文](./README_CN.md)</sup>

# Mango Finder

**🥭 Awake your data**

![1691585544443](./screenshots/work.png)

[![Download](./screenshots/download-github.svg)](https://github.com/moyangzhan/mango-finder/releases) [![Download](./screenshots/download-baidu.svg)](https://pan.baidu.com/s/1k2WDDzV7v5L86P-Uyup6yw?pwd=8jk1)

## 📖 项目简介

Mango Finder（原名 MangoDesk）是一款用自然语言搜索本地文件的桌面应用，支持跨设备搜索功能。

帮助您根据记忆中的内容查找信息，而不需要记住文件名或文件夹结构。

![search](./screenshots/search.gif)

### 📌 使用场景

满足文档、图片、音频等多类型、多设备场景下的智能搜索需求。

- 📝 **个人文档库** - PDF、Word、Markdown 等多年积累的文件
- 🔗 **多设备环境** - 局域网内跨 NAS、Mac、Linux、Windows 搜索
- 🏢 **团队知识库** - 内部文档、项目文档、会议记录等

### ✨ 特性

- 💭 **按内容意思搜索**

  - 通过描述您记得的意思相近内容来查找文档

- 📍 **关键词精确匹配**

  - 通过文件路径或内容中的精确关键词快速定位。当你记得特定的术语或短语时，这是最理想的选择

- 🔍 **相似文件查找**

  - 支持视觉相似图片查找（感知哈希）、语义相似文档查找、音频内容相似查找
  - 一键发现基于视觉、语义或音频指纹相似的相关文件

- 🔗 **跨设备搜索**

  - 在局域网内连接多台设备，实现跨设备搜索
  - 无需手动传输文件，即可搜索其他电脑上的文件

- 🌐 **多语言与跨语言搜索**

  - 无缝支持 100 多种语言。支持跨语种检索（例如使用中文搜索英文内容），无需任何额外配置

- 🔒 **默认保护隐私**

  - 所有数据都保留在您的设备上，确保隐私安全

- 🖥️ **自托管模型支持**

  - 集成 **Ollama** 和 **vLLM**，可部署私有模型服务
  - 适用于团队或企业内网环境，数据不出内网

- ⚡ **快速高效**

  - 通过优化的索引系统提供即时搜索结果

- 👀 **实时文件和目录监控**

  - 自动检测文件和文件夹的变更（添加/修改/删除），并保持索引及搜索结果的实时更新

- 📂 **兼容现有本地文件**

  - 无需重新整理文件夹或重命名文件 — Mango Finder 直接使用您已有的文件

- 🤖 **AI Agent 友好**

  - 文档自动解析为结构化 Markdown 格式，可直接供 Claude Code、Cursor、OpenClaw 等 AI Agent 工具使用
  - 内置命令行工具 `mfd`，安装包自带，可在设置页一键加入 `PATH`
  
### 🏗️ 架构

**索引**

![indexing](./screenshots/indexing.png)

支持三种处理模式：**本地模式**（完全离线）、**自托管模式**（Ollama/vLLM）、**云端模式**（远程 AI 服务）。

**搜索**

![search](./screenshots/search.png)

### 🛠️ 技术栈

- Frontend

  - WebView（Tauri）
  - PNPM
  - Node.js

- Backend

  - Rust
  - Tauri Core

## 🚀 开发

环境搭建与构建：[docs/cn/build.md](./docs/cn/build.md)

```sh
pnpm i               # 安装前端依赖
pnpm tauri dev       # 开发模式运行 GUI
pnpm tauri build     # 构建安装包（GUI + CLI） -> src-tauri/target/release/bundle/
```

CLI 与 GUI 同 crate，开发模式直接运行任意子命令：

```sh
cd src-tauri
cargo run --bin mfd -- search "机器学习" --type keyword
```

CLI 用法见 [docs/cn/cli.md](./docs/cn/cli.md)

## ❓ FAQ

### Q: Mango Finder 如何确保数据隐私？

A: Mango Finder 采用本地优先（local-first）架构来确保数据隐私：

#### 本地数据处理

- 所有文档索引和搜索操作都在本地设备上执行
- 默认情况下完全离线运行，不依赖网络
- 仅在用户**手动启用远程模型服务**后，处理图片和音频文件时才会使用远程模型

#### 数据存储

- 默认情况下，所有用户数据都保留在本地设备上

#### 架构详情

如上面的架构图所示，整个处理流程都设计为本地运行，以确保最大程度的隐私和安全性。

### Q: 为什么代码中使用了这么多模型？

A: 代码库包含多个模型，各自服务于不同目的：

#### 1. 本地模型（默认启用）

- `src-tauri/assets/model`中为本地模型文件
- 这些模型在用户计算机上本地运行，用于文档、图片及音频处理
- 优先考虑隐私和性能

#### 2. 自托管模型（可选）

- 通过 **Ollama** 或 **vLLM** 部署私有模型服务
- 适用于团队或企业内部共用模型场景
- 数据不出内网，保障企业级隐私安全

#### 3. 远程模型（可选）

- `gpt-5.6-terra` 和 `gpt-4o-mini-transcribe`
- 用于图片和音频解析
- 默认禁用，可根据需要启用
- 注意：后续如有本地替代方案，则优先使用本地方案

### Q: 使用多机互联功能时，为什么发现不了局域网中的其他设备？

A: 使用多机互联功能的前提条件：

- 所有设备连接到**同一局域网**
- 所有设备都已**启动 Mango Finder** 并开启了多机互联功能

多机互联功能依赖 mDNS 协议进行设备发现，以下情况可能导致设备无法被发现：

#### 常见原因

1. **网络隔离**

   - 某些路由器或网络环境启用了"AP 隔离"或"客户端隔离"功能
   - 这会阻止局域网内设备之间的直接通信
   - 解决方法：登录路由器管理界面，关闭"AP 隔离"或类似选项
2. **防火墙限制**

   - Windows 防火墙或第三方安全软件可能阻止了入站连接
   - 解决方法：允许 Mango Finder 通过防火墙，或临时关闭防火墙测试
3. **不在同一网段**

   - 设备连接到不同的子网（如 2.4G 和 5G 频段有时会被分配不同网段）
   - 解决方法：确保所有设备连接到同一网段
4. **端口被占用**

   - 默认端口 15678 被其他程序占用
   - 解决方法：在多机互联设置中修改为其他端口

#### 诊断步骤

1. **测试网络连通性**

   ```sh
   # 在设备 A 上 ping 设备 B 的 IP 地址
   ping 192.168.1.xxx
   ```

   如果 ping 不通，说明存在网络隔离问题。
2. **检查端口**

   ```sh
   # 测试目标设备的 HTTP 服务是否可达
   curl http://192.168.1.xxx:7890/ping
   ```
3. **检查防火墙**

   - Windows：控制面板 → Windows Defender 防火墙 → 允许应用通过防火墙
   - macOS：系统偏好设置 → 安全性与隐私 → 防火墙
4. **手动添加设备**

   - 如果自动发现仍不工作，可以在设备列表中点击"添加设备"，手动输入 IP 和端口

## 📝 LICENSE

[LICENSE](LICENSE)

## 🤝 贡献指南

欢迎任何形式的贡献，包括但不限于：

- 🐛 提交 Bug 报告
- 💡 提出功能建议
- 📖 改进文档
- 🔧 提交代码（PR）

在提交 PR 之前，建议：

1. Fork 本仓库
2. 创建新分支 (git checkout -b feature/xxx)
3. 确保本地可以正常运行 pnpm tauri dev
4. 提交更改 (git commit -m 'feat: xxx')
5. 推送分支 (git push origin feature/xxx)
6. 提交 Pull Request

## ⭐ 支持我们

如果 Mango Finder 对您有帮助，欢迎：

- 在 GitHub 上给项目一个 Star
- 向朋友推荐
- 分享使用体验
