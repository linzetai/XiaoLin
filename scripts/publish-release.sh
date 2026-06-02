#!/usr/bin/env bash
set -euo pipefail

#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# XiaoLin — 发布脚本
#
# 在任一台能访问 GitHub 的机器上运行。
# 从已有的 GitHub Release 中读取签名文件，生成并上传 latest.json。
#
# 前提:
#   1. Linux/Windows 机器已分别执行构建脚本
#   2. 构建产物已上传到 GitHub Release (可手动拖拽上传)
#   3. 本机已安装 gh CLI 并已登录
#
# 用法:
#   ./scripts/publish-release.sh v0.1.0
#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

if [ $# -lt 1 ]; then
  echo "用法: $0 <tag>  (例: v0.1.0)"
  exit 1
fi

TAG="$1"
VERSION="${TAG#v}"

if ! command -v gh &>/dev/null; then
  echo "错误: 未找到 gh CLI。请安装: https://cli.github.com/"
  exit 1
fi

log() { echo -e "\033[1;36m▸ $1\033[0m"; }
ok()  { echo -e "\033[1;32m✓ $1\033[0m"; }
err() { echo -e "\033[1;31m✗ $1\033[0m" >&2; }

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || echo "")
if [ -z "$REPO" ]; then
  echo "错误: 无法获取仓库信息。请在项目目录中运行，或确认 gh 已登录。"
  exit 1
fi

BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"

log "仓库: $REPO"
log "Tag:  $TAG"
log "版本: $VERSION"

#── 获取 Release 中的文件列表 ─────────────────────────────────────────

log "获取 Release $TAG 的文件列表..."

ASSETS=$(gh release view "$TAG" --json assets -q '.assets[].name' 2>/dev/null || true)
if [ -z "$ASSETS" ]; then
  err "未找到 Release $TAG，或 Release 中没有文件。"
  echo ""
  echo "  请先创建 Release 并上传构建产物:"
  echo "    gh release create $TAG --title \"XiaoLin $TAG\" --generate-notes"
  echo ""
  echo "  然后手动上传产物（通过 GitHub 网页拖拽，或 gh release upload）:"
  echo "    gh release upload $TAG ./dist-linux/* ./dist-windows/*"
  exit 1
fi

echo "  已有文件:"
echo "$ASSETS" | sed 's/^/    /'

#── 下载签名文件 ──────────────────────────────────────────────────────

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

log "下载签名文件..."

for asset in $ASSETS; do
  case "$asset" in
    *.sig) gh release download "$TAG" -p "$asset" -D "$TMPDIR" ;;
  esac
done

#── 构建 latest.json ──────────────────────────────────────────────────

log "生成 latest.json..."

PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

python3 - "$TMPDIR" "$BASE_URL" "$VERSION" "$PUB_DATE" "$ASSETS" <<'PYEOF'
import sys, json, os

sig_dir = sys.argv[1]
base_url = sys.argv[2]
version = sys.argv[3]
pub_date = sys.argv[4]
all_assets = sys.argv[5].strip().split('\n')

platforms = {}

sig_files = {f: open(os.path.join(sig_dir, f)).read().strip()
             for f in os.listdir(sig_dir) if f.endswith('.sig')}

platform_map = [
    ('.AppImage.tar.gz', 'linux-x86_64'),
    ('.AppImage', 'linux-x86_64'),
    ('.nsis.zip', 'windows-x86_64'),
    ('.app.tar.gz', 'darwin-universal'),
]

for asset in all_assets:
    if asset.endswith('.sig'):
        continue
    for suffix, platform_key in platform_map:
        if asset.endswith(suffix) and platform_key not in platforms:
            sig_candidate = asset + '.sig'
            if sig_candidate in sig_files:
                platforms[platform_key] = {
                    'url': f'{base_url}/{asset}',
                    'signature': sig_files[sig_candidate]
                }
                break

result = {
    'version': version,
    'notes': f'XiaoLin v{version}',
    'pub_date': pub_date,
    'platforms': platforms
}

out_path = os.path.join(sig_dir, 'latest.json')
with open(out_path, 'w') as f:
    json.dump(result, f, indent=2)

print(f'  版本: {version}')
print(f'  平台: {list(platforms.keys())}')
PYEOF

ok "latest.json 已生成"
cat "$TMPDIR/latest.json"

#── 上传 latest.json ──────────────────────────────────────────────────

log "上传 latest.json 到 Release $TAG..."

if echo "$ASSETS" | grep -q "^latest.json$"; then
  gh release delete-asset "$TAG" latest.json --yes
fi

gh release upload "$TAG" "$TMPDIR/latest.json"
ok "latest.json 已上传!"

echo ""
echo "  验证:"
echo "    curl -sL ${BASE_URL}/latest.json | python3 -m json.tool"
echo ""
ok "发布完成! [$TAG]"
