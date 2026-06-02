#!/usr/bin/env bash
set -euo pipefail

#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# XiaoLin — 合并多平台 latest.json
#
# 在不同机器上分别构建 Linux 和 Windows 后，使用此脚本合并
# 两个 latest.json 为最终可发布的版本。
#
# 用法:
#   ./scripts/merge-latest-json.sh <linux-dir> <windows-dir> [-o output-dir]
#
# 示例:
#   ./scripts/merge-latest-json.sh ./dist-linux ./dist-windows -o ./release
#   # 将 dist-linux/latest.json + dist-windows/latest.json 合并
#   # 输出到 ./release/latest.json
#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

if [ $# -lt 2 ]; then
  echo "用法: $0 <linux-dist-dir> <windows-dist-dir> [-o output-dir]"
  echo ""
  echo "示例: $0 ./dist-linux ./dist-windows -o ./release"
  exit 1
fi

LINUX_DIR="$1"
WINDOWS_DIR="$2"
OUTPUT_DIR="."

shift 2
while [ $# -gt 0 ]; do
  case "$1" in
    -o) OUTPUT_DIR="$2"; shift 2 ;;
    *)  shift ;;
  esac
done

LINUX_JSON="$LINUX_DIR/latest.json"
WINDOWS_JSON="$WINDOWS_DIR/latest.json"

if [ ! -f "$LINUX_JSON" ]; then
  echo "错误: 未找到 $LINUX_JSON"
  exit 1
fi
if [ ! -f "$WINDOWS_JSON" ]; then
  echo "错误: 未找到 $WINDOWS_JSON"
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

if command -v python3 &>/dev/null; then
  python3 -c "
import json, sys

with open('$LINUX_JSON') as f: linux = json.load(f)
with open('$WINDOWS_JSON') as f: windows = json.load(f)

merged = linux.copy()
merged['platforms'].update(windows.get('platforms', {}))

with open('$OUTPUT_DIR/latest.json', 'w') as f:
    json.dump(merged, f, indent=2)

print('✓ 合并完成')
print(f'  平台: {list(merged[\"platforms\"].keys())}')
print(f'  版本: {merged[\"version\"]}')
"
elif command -v jq &>/dev/null; then
  jq -s '.[0] * { platforms: (.[0].platforms + .[1].platforms) }' \
    "$LINUX_JSON" "$WINDOWS_JSON" > "$OUTPUT_DIR/latest.json"
  echo "✓ 合并完成 (jq)"
else
  echo "错误: 需要 python3 或 jq 来合并 JSON"
  exit 1
fi

echo ""
echo "输出: $OUTPUT_DIR/latest.json"
echo ""
cat "$OUTPUT_DIR/latest.json"
