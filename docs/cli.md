# Mango Finder CLI (mfd)

Mango Finder Command Line Interface for searching and managing documents.

## Installation

CLI tool is built with Mango Finder, located at `src-tauri/target/release/mfd.exe`

## Getting Help

```bash
# Show help
mfd --help
mfd -h

# Show subcommand help
mfd search --help
mfd index --help

# Show detailed documentation for a command
mfd help search
mfd help index

# Show full CLI documentation
mfd help

# Equivalent per-command flag
mfd search --help-doc
```

## Command List

### search - Search Documents

```bash
mfd search <query> [--type semantic|keyword] [--device <id>] [--limit <n>]
```

**Parameters:**
- `query`: Search query string
- `--type`: Search type, `semantic` (default) or `keyword`
- `--device`: Remote device ID (optional, not implemented yet)
- `--limit`: Max results, default 10

**Examples:**
```bash
# Semantic search
mfd search "machine learning"

# Keyword search
mfd search "report.docx" --type keyword --limit 5
```

**Notes:**
- Semantic search requires local embedding model files
- `total` in the output is the full match count; `returned` is the number of items actually returned after `--limit`

### similar - Find Similar Files

```bash
mfd similar <file_id> [--device <id>] [--limit <n>]
```

**Examples:**
```bash
mfd similar 123 --limit 5
```

### index - Index Management

```bash
mfd index <action>
```

**Actions:**
- `status`: Show index status and recent task history
- `start <paths...>`: Start indexing, runs in the foreground and returns when finished
- `list [--page <n>] [--page-size <n>]`: List indexed files
- `clear`: Clear all index

**Examples:**
```bash
# Show index status
mfd index status

# Start indexing (blocks until finished; progress logs go to stderr)
mfd index start "C:\Documents" "D:\Projects"

# List indexed files
mfd index list --page 1 --page-size 50
```

**Notes:**
- `index start` runs in the foreground; press `Ctrl+C` to stop it before completion
- Use `index status` to see indexed file counts and recent task history (persisted in the database)

### file - File Operations

```bash
mfd file <id> [--open] [--device <id>]
```

**Parameters:**
- `id`: File ID
- `--open`: Open the file with the system default program (works for any indexed file type)
- `--device`: Remote device ID (optional, not implemented yet)

**Examples:**
```bash
# Get file info
mfd file 123

# Open the file with the system default program
mfd file 123 --open
```

### device - Device Management

```bash
mfd device <action>
```

**Actions:**
- `list`: List online devices

### status - Application Status

```bash
mfd status
```

### version - Version Info

```bash
mfd version
```

### help - View Documentation

```bash
mfd help [command]
```

**Examples:**
```bash
# Show full CLI documentation
mfd help

# Show detailed documentation for the search command
mfd help search

# Show detailed documentation for the index command
mfd help index
```

### check - Check System Status

```bash
mfd check
```

**Check items:**
- System info (version, device name, client ID)
- Network status (local IP, network interfaces, port availability)
- Storage status (path, indexed files count, embedding count)
- Cluster status (enabled, paired devices count)
- AI model status (model files existence)

**Examples:**
```bash
# Check system status
mfd check

# JSON output
mfd check --output json
```

### locale - Get or Set Locale

```bash
mfd locale [value]
```

**Parameters:**
- `value`: Locale to set (e.g., `zh-CN`, `en-US`). If not specified, show current locale.

**Examples:**
```bash
# Show current locale
mfd locale

# Set locale to Chinese
mfd locale zh-CN

# Set locale to English
mfd locale en-US
```

## Global Options

| Option | Environment Variable | Description |
|--------|---------------------|-------------|
| `--output json\|table` | `MANGO_FINDER_OUTPUT` | Output format, default `json` |
| `--quiet` | `MANGO_FINDER_QUIET` | Suppress logs, only output result |

## Environment Variables

Configure default behavior via environment variables:

```bash
# Set default output format
export MANGO_FINDER_OUTPUT=table

# Enable quiet mode
export MANGO_FINDER_QUIET=1
```

## Output Format

Default JSON output:

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

Error output:

```json
{
  "success": false,
  "data": null,
  "error": "Error message"
}
```

## AI Agent Examples

```bash
# Search documents
mfd search "machine learning algorithms" --output json

# Get file details
mfd file 123 --output json

# Check index status
mfd index status --output json

# Find similar files
mfd similar 456 --limit 5 --output json
```
