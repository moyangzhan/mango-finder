# Mango Finder CLI (mfd)

Mango Finder 命令行工具，方便开发者和 AI Agent 使用。

## 安装

CLI 工具随 Mango Finder 一起构建，位于 `src-tauri/target/release/mfd.exe`

## 获取帮助

```bash
# 显示帮助信息
mfd --help
mfd -h

# 显示子命令帮助
mfd search --help
mfd index --help

# 显示某个命令的详细文档
mfd help search
mfd help index

# 显示完整 CLI 文档
mfd help

# 等价的命令内参数
mfd search --help-doc
```

## 命令列表

### search - 搜索文档

```bash
mfd search <query> [--type semantic|keyword] [--device <id>] [--limit <n>]
```

**参数说明：**
- `query`：搜索关键词
- `--type`：搜索类型，`semantic`（语义搜索，默认）或 `keyword`（关键词搜索）
- `--device`：远程设备 ID（可选，暂未实现）
- `--limit`：返回结果数量限制，默认 10

**示例：**
```bash
# 语义搜索
mfd search "机器学习"

# 关键词搜索
mfd search "report.docx" --type keyword --limit 5
```

**说明：**
- 语义搜索需要本地存在 embedding 模型文件
- 输出中的 `total` 为完整匹配数量，`returned` 为应用 `--limit` 后实际返回的数量

### similar - 查找相似文件

```bash
mfd similar <file> [--device <id>] [--limit <n>] [--index]
```

**参数说明：**
- `file`：文件 ID、文件名或绝对路径
  - 纯数字按文件 ID 解析
  - 含路径分隔符（`\` 或 `/`）按绝对路径精确匹配；路径未索引但磁盘上存在时进入**现场模式**（见下）
  - 其余按文件名精确匹配；存在多个同名文件时使用 id 最小的一个，输出中的 `notice` 与 `matched_files`（最多 5 条）会列出其余匹配，便于改用 ID 或路径重试

**现场模式（外部未索引文件）：**
- 绝对路径在索引中未命中但磁盘存在时，自动现场解析/打标/转写并计算嵌入后检索，**不写索引**
- 输出标识 `"mode": "external"`、`"notice"`（耗时来源与建索引建议）、`"chunk_count"`（分块嵌入数），`file.id` 为 `null`
- 执行前 stderr 输出耗时提示（`--quiet` 时不输出）；各类耗时预期：
  - 文档：加载嵌入模型 + 逐分块推理，大文件较慢
  - 图片：感知哈希（毫秒级）+ 视觉模型打标，通常秒级；OCR 模型缺失时自动跳过
  - 音频-音乐：提取音频指纹（无模型），较快
  - 音频-语音：Whisper 转写，长音频可能需要数分钟
- 视频/未知类型不支持，报错并建议先 `mfd index start` 建索引

**`--index`（外部文件顺手入库）：**
- 仅对外部文件生效：先单文件入库（解析与嵌入只做一次），再走已索引快路径查询；对已索引文件是空操作
- 输出含 `"indexed": true` 与新分配的 `file.id`，`notice` 说明已入库
- 路径不在监听目录（fs_watcher 设置的 directories）时，`notice` 额外提示：文件变更不会自动重新索引
- 入库失败时自动回退纯现场计算，并在 `notice` 中说明

**示例：**
```bash
# 按文件 ID
mfd similar 123 --limit 5

# 按文件名
mfd similar "report.docx"

# 按绝对路径
mfd similar "C:\Documents\report.docx"

# 未索引的外部文件（现场计算，不写索引）
mfd similar "D:\Downloads\new_report.pdf"

# 外部文件查完的同时加入索引
mfd similar "D:\Downloads\new_report.pdf" --index
```

### index - 索引管理

```bash
mfd index <action>
```

**可用操作：**
- `status`：查看索引状态和最近任务历史
- `start <paths...>`：执行索引任务，前台运行直到完成后返回
- `list [--page <n>] [--page-size <n>]`：列出已索引文件
- `clear`：清空所有索引

**示例：**
```bash
# 查看索引状态（包含最近任务历史）
mfd index status

# 开始索引（前台执行直到完成，进度日志输出到 stderr）
mfd index start "C:\Documents" "D:\Projects"

# 列出已索引文件
mfd index list --page 1 --page-size 50
```

**说明：**
- `index start` 在前台执行，需要中途停止时按 `Ctrl+C` 终止
- 索引任务历史持久化在数据库中，可用 `index status` 查看

### file - 文件操作

```bash
mfd file <id> [--open] [--device <id>]
```

**参数说明：**
- `id`：文件 ID
- `--open`：使用系统默认程序打开文件（适用于任意已索引的文件类型）
- `--device`：远程设备 ID（可选，暂未实现）

**示例：**
```bash
# 查看文件信息
mfd file 123

# 使用系统默认程序打开文件
mfd file 123 --open
```

### device - 设备管理

```bash
mfd device <action>
```

**可用操作：**
- `list`：列出在线设备

### status - 应用状态

```bash
mfd status
```

### version - 版本信息

```bash
mfd version
```

### help - 查看文档

```bash
mfd help [command]
```

**示例：**
```bash
# 显示完整 CLI 文档
mfd help

# 查看 search 命令的详细文档
mfd help search

# 查看 index 命令的详细文档
mfd help index
```

### check - 检查系统状态

```bash
mfd check
```

**检查内容：**
- 系统信息（版本、设备名称、客户端 ID）
- 网络状态（本地 IP、网络接口、端口可用性）
- 存储状态（路径、已索引文件数量、embedding 数量）
- 集群状态（是否启用、已配对设备数量）
- AI 模型状态（模型文件是否存在）

**示例：**
```bash
# 检查系统状态
mfd check

# JSON 格式输出
mfd check --output json
```

### locale - 获取或设置语言

```bash
mfd locale [value]
```

**参数说明：**
- `value`：要设置的语言（如 `zh-CN`、`en-US`）。如果不指定，则显示当前语言。

**示例：**
```bash
# 显示当前语言
mfd locale

# 设置为中文
mfd locale zh-CN

# 设置为英文
mfd locale en-US
```

### shutdown - 关闭本地服务

```bash
mfd shutdown
```

**说明：**
- 关闭 CLI 按需拉起的本地常驻服务（daemon），释放其占用的模型内存
- 若服务由 GUI 提供（多机互联开启），则不会关闭，提示服务随 GUI 退出
- 服务关闭后，下一次需要服务的命令会自动重新拉起

## 本地服务（快速路径）

`search`（语义/关键词）和 `similar`（已索引文件）会优先复用本机常驻服务中的热模型，避免每次查询重新加载模型：

- **GUI 正在运行且开启多机互联**：直接使用 GUI 进程内的服务，模型冷启动只发生一次
- **服务不存在**：自动在后台拉起 `mfd serve`（daemon，仅监听 127.0.0.1，空闲 30 分钟自动退出），首条命令包含启动耗时（约 1-3 秒），之后毫秒级
- **拉起失败或服务不可用**：静默回落进程内查询（较慢但结果一致），stderr 有日志说明
- 任何启动顺序都会收敛到单一宿主：GUI 启动时若端口被自家 daemon 占用，会先请其退出再接管

两条路径的输出结构完全一致，调用方无感知。路径选择日志输出到 stderr（`--quiet` 时不输出）。

## 全局选项

| 选项 | 环境变量 | 说明 |
|------|----------|------|
| `--output json\|table` | `MANGO_FINDER_OUTPUT` | 输出格式，默认 `json`。`table` 为人类可读输出（对齐表格、按命令选取重点列、表头跟随 locale），结果仍输出到 stdout |
| `--quiet` | `MANGO_FINDER_QUIET` | 静默模式，只输出结果，不输出日志 |

## 环境变量

可以通过环境变量配置默认行为：

```bash
# 设置默认输出格式
export MANGO_FINDER_OUTPUT=table

# 启用静默模式
export MANGO_FINDER_QUIET=1

# 不自动拉起本地服务 daemon（服务不存在时直接走进程内查询）
export MANGO_FINDER_NO_DAEMON=1

# 本地服务空闲退出阈值（秒，默认 1800，测试用）
export MANGO_FINDER_DAEMON_IDLE_SECS=60
```

## 输出格式

默认输出 JSON 格式：

```json
{
  "success": true,
  "data": {
    "results": [...],
    "total": 25,
    "returned": 10,
    "elapsed_ms": 156
  },
  "error": null
}
```

错误输出：

```json
{
  "success": false,
  "data": null,
  "error": "Error message"
}
```

## AI Agent 使用示例

```bash
# 搜索文档
mfd search "机器学习算法" --output json

# 获取文件详情
mfd file 123 --output json

# 检查索引状态
mfd index status --output json

# 查找相似文件
mfd similar 456 --limit 5 --output json
```
