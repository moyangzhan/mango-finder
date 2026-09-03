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
mfd similar <file_id> [--device <id>] [--limit <n>]
```

**示例：**
```bash
mfd similar 123 --limit 5
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

## 全局选项

| 选项 | 环境变量 | 说明 |
|------|----------|------|
| `--output json\|table` | `MANGO_FINDER_OUTPUT` | 输出格式，默认 `json` |
| `--quiet` | `MANGO_FINDER_QUIET` | 静默模式，只输出结果，不输出日志 |

## 环境变量

可以通过环境变量配置默认行为：

```bash
# 设置默认输出格式
export MANGO_FINDER_OUTPUT=table

# 启用静默模式
export MANGO_FINDER_QUIET=1
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
