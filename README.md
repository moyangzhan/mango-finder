<sup>[English](./README.md) | [中文](./README_CN.md)</sup>

# Mango Finder

**🥭 Awake your data**

![image](./screenshots/work.png)

[![Download](./screenshots/download-github.svg)](https://github.com/moyangzhan/mango-finder/releases) [![Download](./screenshots/download-baidu.svg)](https://pan.baidu.com/s/1k2WDDzV7v5L86P-Uyup6yw?pwd=8jk1)

## What is Mango Finder?

Mango Finder (formerly MangoDesk) is a local-first desktop app for searching your local documents with natural language, with cross-device search support.

It helps you find information based on what you remember, not file names or folder structures.

![search](./screenshots/search.gif)

### 📌 Use Cases

Intelligent search for documents, images, and audio files across multi-device environments.

- 📝 **Personal Document Libraries** - PDFs, Word, Markdown and other accumulated files
- 🔗 **Multi-Device Environment** - Search across NAS, Mac, Linux, Windows on local network
- 🏢 **Team Knowledge Base** - Internal documents, project docs, meeting notes, etc.

### ✨ Features

- 💭 **Search by meaning**

  - Find documents by describing what you remember, even if you don’t recall exact titles or locations

- 📍 **Exact Keyword Match**

  - Instantly locate files using precise terms from file paths or content, ideal for finding specific phrases or technical strings.

- 🔍 **Find Similar Files**

  - Find visually similar images using perceptual hashing, semantically similar documents, or audio files with matching content
  - One click to discover related files based on visual, semantic, or audio fingerprint similarity

- 🔗 **Cross-Device Search**

  - Connect multiple devices on your local network to search across all connected devices
  - Find files from your other computers without manually transferring them

- 🌐 **Multilingual & Cross-language Search**

  - Search across **100+ languages** seamlessly. Find English documents using Chinese queries, or vice versa, with zero configuration required

- 🔒 **Private by default**

  - All data stays on your device, ensuring your privacy

- 🖥️ **Self-Hosted Model Support**

  - Integration with **Ollama** and **vLLM** for deploying private model services
  - Ideal for team or enterprise intranet environments, data stays within internal network

- ⚡ **Fast and efficient**

  - Instant search results with optimized indexing system

- 👀 **Real-time file & directory watching**

  - Automatically detects file and folder changes (add / modify / delete) and keeps index and search results up to date

- 📂 **Works with your existing local files**

  - No need to reorganize folders or rename files — Mango Finder works with what you already have

- 🤖 **AI Agent Friendly**

  - Documents are automatically parsed into structured Markdown — ready for tools like Claude Code, Cursor, and OpenClaw
  - Built-in CLI (`mfd`) — bundled with the installer, one click in settings to add it to `PATH`
  
### 🏗️ Architecture

**Indexing**

![indexing](./screenshots/indexing.png)

Supports three processing modes: **Local** (fully offline), **Self-Hosted** (Ollama/vLLM), and **Cloud** (remote AI services).

**Search**

![search](./screenshots/search.png)

### 🛠️ Technology Stack

- Frontend

  - WebView（Tauri）
  - PNPM
  - Node.js

- Backend

  - Rust
  - Tauri Core

## 🚀 Development

Environment setup & build: [docs/en/build.md](./docs/en/build.md)

```sh
pnpm i               # install frontend dependencies
pnpm tauri dev       # run the GUI in dev mode
pnpm tauri build     # build the installer (GUI + CLI) -> src-tauri/target/release/bundle/
```

The CLI builds from the same crate — run any subcommand directly in dev mode:

```sh
cd src-tauri
cargo run --bin mfd -- search "machine learning" --type keyword
```

CLI usage: [docs/en/cli.md](./docs/en/cli.md)

## ❓ FAQ

### Q: How does Mango Finder ensure data privacy?

A: Mango Finder follows a local-first architecture to ensure data privacy:

#### Local Data Processing

- All document indexing and search operations are performed locally on your device
- By default, all features run completely offline without network dependency
- Remote models are only used for image and audio processing when users **manually enable remote model services**

#### Data storage

- All user data remains on the local device by default

#### Architecture Details

As shown in the architecture diagram above, the entire processing pipeline is designed to keep data local, ensuring maximum privacy and security.

### Q: Why are so many models used in the code?

A: The codebase includes multiple models serving different purposes:

#### 1. Active Local Models (Enabled by Default)

- `src-tauri/assets/model/*`
- These models run locally on users' computers for document, image, and audio processing
- Prioritized for privacy and performance

#### 2. Self-Hosted Models (Optional)

- Deploy private model services via **Ollama** or **vLLM**
- Ideal for teams or enterprises sharing models internally
- Data stays within the internal network, ensuring enterprise-level privacy

#### 3. Remote Models (Optional)

- `gpt-5.6-terra` and `gpt-4o-mini-transcribe`
- Designed for image and audio parsing
- Disabled by default, can be enabled if needed
- Note: We plan to replace these with local alternatives when available

### Q: Why can't I discover other devices on my local network when using the multi-device feature?

A: Prerequisites for using the multi-device feature:

- All devices are connected to the **same local network**
- All devices have **Mango Finder running** with the multi-device feature enabled

The multi-device feature relies on the mDNS protocol for device discovery. The following situations may prevent devices from being discovered:

#### Common Causes

1. **Network Isolation**

   - Some routers or network environments have "AP Isolation" or "Client Isolation" enabled
   - This blocks direct communication between devices on the same network
   - Solution: Log into your router's admin interface and disable "AP Isolation" or similar options
2. **Firewall Restrictions**

   - Windows Firewall or third-party security software may block inbound connections
   - Solution: Allow Mango Finder through the firewall, or temporarily disable the firewall for testing
3. **Different Subnets**

   - Devices connected to different subnets (e.g., 2.4GHz and 5GHz bands sometimes get assigned different subnets)
   - Solution: Ensure all devices are connected to the same subnet
4. **Port Already in Use**

   - The default port 15678 is occupied by another program
   - Solution: Change to a different port in Multi-Device Settings

#### Diagnostic Steps

1. **Test Network Connectivity**

   ```sh
   # On device A, ping device B's IP address
   ping 192.168.1.xxx
   ```

   If ping fails, there's a network isolation issue.
2. **Check Port**

   ```sh
   # Test if the target device's HTTP service is reachable
   curl http://192.168.1.xxx:7890/ping
   ```
3. **Check Firewall**

   - Windows: Control Panel → Windows Defender Firewall → Allow an app through firewall
   - macOS: System Preferences → Security & Privacy → Firewall
4. **Add Device Manually**

   - If auto-discovery still doesn't work, you can click "Add Device" in the device list and manually enter the IP and port

## 📝 License

see the [LICENSE](LICENSE) file for details.

## 🤝 Contributing

Contributions of all kinds are welcome, including but not limited to:

- 🐛 Reporting bugs
- 💡 Suggesting new features or improvements
- 📖 Improving documentation
- 🔧 Submitting code (pull requests)

Before submitting a pull request, please consider:

1. Fork this repository
2. Create a new branch (git checkout -b feature/xxx)
3. Ensure pnpm tauri dev runs successfully locally
4. Commit changes (git commit -m 'feat: xxx')
5. Push the branch (git push origin feature/xxx)
6. Submit a Pull Request

## ⭐ Support the Project

Support Mango Finder if you find it helpful:

- Starring the repository on GitHub
- Recommending it to others
- Sharing your experience
