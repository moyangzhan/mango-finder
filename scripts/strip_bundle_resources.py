#!/usr/bin/env python3
"""Build-time helper for the Lite package variant.

Removes `bundle.resources` (the ~950MB model files) from tauri.conf.json so the
Lite installer ships code-only. Tauri's `--config` flag deep-merges JSON and
cannot DELETE declared keys (an empty object merges to a no-op), so the Lite
build temporarily edits the main config in place; CI restores it with
`git checkout -- src-tauri/tauri.conf.json` afterwards.

Lite 包构建辅助:从 tauri.conf.json 移除 bundle.resources(约 950MB 模型文件)。
Tauri 的 --config 是深合并且无法删除已声明键(空对象合并为无操作),因此 Lite
构建直接就地改主配置,CI 构建完成后用 git checkout 恢复。
"""
import json
import sys

DEFAULT_PATH = "src-tauri/tauri.conf.json"


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_PATH
    with open(path, encoding="utf-8") as f:
        config = json.load(f)
    removed = config.get("bundle", {}).pop("resources", None)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(config, f, indent=2, ensure_ascii=False)
        f.write("\n")
    if removed:
        names = list(removed.keys()) if isinstance(removed, dict) else removed
        print(f"stripped bundle.resources: {names}")
    else:
        print("no bundle.resources found (already stripped?)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
