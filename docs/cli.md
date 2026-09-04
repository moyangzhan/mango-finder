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
mfd similar <file> [--device <id>] [--limit <n>] [--index]
```

**Parameters:**
- `file`: file ID, file name, or absolute path
  - Pure digits are treated as a file ID
  - Values containing a path separator (`\` or `/`) match the absolute path exactly; a path that is not indexed but exists on disk switches to **on-the-fly mode** (see below)
  - Anything else matches the file name exactly; when multiple files share the name, the one with the smallest id is used, and `notice` plus `matched_files` (up to 5) in the output list the other matches so you can retry with an ID or path

**On-the-fly mode (un-indexed external files):**
- When an absolute path misses the index but exists on disk, the file is parsed / captioned / transcribed and embedded in memory before searching; **nothing is written to the index**
- The output carries `"mode": "external"`, a `notice` (cost explanation and indexing suggestion), and `chunk_count`; `file.id` is `null`
- A stderr hint is printed before the work starts (suppressed by `--quiet`); expected cost per type:
  - Documents: embedding model load + per-chunk inference; large files are slow
  - Images: perceptual hash (milliseconds) + vision model captioning, usually seconds; OCR is skipped automatically when its models are missing
  - Music audio: fingerprint extraction (no model), fast
  - Speech audio: Whisper transcription; long audio may take minutes
- Video and unknown types are not supported; the command errors and suggests `mfd index start` first

**`--index` (add the external file to the index):**
- Applies to external files only: the file is indexed first (parsing and embedding run once), then queried via the fast path; a no-op for already indexed files
- The output carries `"indexed": true` with the newly assigned `file.id`, and the `notice` confirms indexing
- If the path is outside watched directories (fs_watcher setting), the `notice` adds that later changes are not re-indexed automatically
- If indexing fails, the command falls back to pure on-the-fly computation and says so in the `notice`

**Examples:**
```bash
# By file ID
mfd similar 123 --limit 5

# By file name
mfd similar "report.docx"

# By absolute path
mfd similar "C:\Documents\report.docx"

# Un-indexed external file (on-the-fly, nothing written to the index)
mfd similar "D:\Downloads\new_report.pdf"

# Add the external file to the index while querying
mfd similar "D:\Downloads\new_report.pdf" --index
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
