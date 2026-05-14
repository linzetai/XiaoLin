#!/usr/bin/env bash
set -euo pipefail

#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# FastClaw — Linux 本地打包脚本
#
# 用法:
#   ./scripts/build-linux.sh              # 正常构建
#   ./scripts/build-linux.sh --release    # 构建 + 生成 latest.json
#   ./scripts/build-linux.sh --skip-lint  # 跳过 clippy 检查
#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_DIR="$PROJECT_ROOT/crates/fastclaw-app"
TAURI_DIR="$APP_DIR/src-tauri"
DIST_DIR="$PROJECT_ROOT/dist"
KEY_PATH="$HOME/.tauri/fastclaw.key"

SKIP_LINT=false
RELEASE_MODE=false

for arg in "$@"; do
  case "$arg" in
    --skip-lint) SKIP_LINT=true ;;
    --release)   RELEASE_MODE=true ;;
  esac
done

log() { echo -e "\033[1;36m▸ $1\033[0m"; }
err() { echo -e "\033[1;31m✗ $1\033[0m" >&2; }
ok()  { echo -e "\033[1;32m✓ $1\033[0m"; }

#── 并行构建参数 ──────────────────────────────────────────────────────
NPROC=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)

export CARGO_BUILD_JOBS="$NPROC"
export RUSTFLAGS="${RUSTFLAGS:-} -C codegen-units=$NPROC -C link-arg=-fuse-ld=mold"

if command -v sccache &>/dev/null; then
  export RUSTC_WRAPPER=sccache
fi

if command -v mold &>/dev/null; then
  export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="clang"
  export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-fuse-ld=mold"
else
  RUSTFLAGS="${RUSTFLAGS//-C link-arg=-fuse-ld=mold/}"
  export RUSTFLAGS
fi

#── 环境检查 ──────────────────────────────────────────────────────────

log "检查构建环境..."

for cmd in cargo pnpm node; do
  if ! command -v "$cmd" &>/dev/null; then
    err "未找到 $cmd，请先安装"
    exit 1
  fi
done

if [ ! -f "$KEY_PATH" ]; then
  err "签名私钥不存在: $KEY_PATH"
  echo "  运行以下命令生成:"
  echo "  npx @tauri-apps/cli@latest signer generate --write-keys $KEY_PATH --force -p \"\""
  exit 1
fi

export TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_PATH")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

VERSION=$(grep '"version"' "$TAURI_DIR/tauri.conf.json" | head -1 | sed 's/.*: *"\(.*\)".*/\1/')
ok "版本号: v$VERSION"
ok "CPU 核数: $NPROC (cargo jobs=$CARGO_BUILD_JOBS)"
ok "Cargo: $(cargo --version)"
ok "Node:  $(node --version)"
ok "pnpm:  $(pnpm --version)"
if [ -n "${RUSTC_WRAPPER:-}" ]; then ok "sccache: 已启用"; fi
if command -v mold &>/dev/null; then ok "链接器: mold"; fi

#── 前端构建 + Lint 并行 ──────────────────────────────────────────────

log "安装前端依赖..."
(cd "$APP_DIR" && pnpm install --prefer-offline)

LINT_PID=""
if [ "$SKIP_LINT" = false ]; then
  log "并行启动: clippy + 前端构建..."
  (cd "$PROJECT_ROOT" && cargo clippy --manifest-path "$TAURI_DIR/Cargo.toml" --no-deps -j "$NPROC") &
  LINT_PID=$!
fi

log "构建前端..."
(cd "$APP_DIR" && NODE_OPTIONS="--max-old-space-size=4096" pnpm build)
ok "前端构建完成"

if [ -n "$LINT_PID" ]; then
  if wait "$LINT_PID"; then
    ok "Clippy 通过"
  else
    err "Clippy 检查失败"
    exit 1
  fi
fi

#── Tauri 构建 ────────────────────────────────────────────────────────

log "构建 Tauri 应用 (Linux) [$NPROC jobs]..."
if [ "${CI:-}" = "1" ]; then
  export CI=true
elif [ "${CI:-}" = "0" ]; then
  export CI=false
fi

(cd "$APP_DIR" && APPIMAGE_EXTRACT_AND_RUN=1 pnpm exec -- tauri build -- -j "$NPROC")
ok "Tauri 构建完成"

#── 收集产物 ──────────────────────────────────────────────────────────

log "收集构建产物..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# Tauri 2.10+ uses target/tauri/release/bundle/
for BUNDLE_DIR in "$PROJECT_ROOT/target/tauri/release/bundle" "$PROJECT_ROOT/target/release/bundle"; do
  [ -d "$BUNDLE_DIR" ] || continue
  for pattern in "*.AppImage" "*.AppImage.sig" "*.AppImage.tar.gz" "*.AppImage.tar.gz.sig" "*.deb" "*.deb.sig"; do
    find "$BUNDLE_DIR" -maxdepth 2 -name "$pattern" -exec cp {} "$DIST_DIR/" \; 2>/dev/null || true
  done
done

ok "产物已收集到 $DIST_DIR/"
ls -lh "$DIST_DIR/"

#── 生成 latest.json (--release 模式) ────────────────────────────────

if [ "$RELEASE_MODE" = true ]; then
  log "生成 latest.json..."

  # Prefer .AppImage.tar.gz (if exists), fallback to .AppImage
  LINUX_ARCHIVE=$(find "$DIST_DIR" -name "*.AppImage.tar.gz" ! -name "*.sig" | head -1)
  LINUX_SIG=$(find "$DIST_DIR" -name "*.AppImage.tar.gz.sig" | head -1)

  # Fallback: Tauri 2.10+ may generate .AppImage + .AppImage.sig directly
  if [ -z "$LINUX_ARCHIVE" ] || [ -z "$LINUX_SIG" ]; then
    LINUX_ARCHIVE=$(find "$DIST_DIR" -name "*.AppImage" ! -name "*.sig" | head -1)
    LINUX_SIG=$(find "$DIST_DIR" -name "*.AppImage.sig" | head -1)
  fi

  if [ -z "$LINUX_ARCHIVE" ] || [ -z "$LINUX_SIG" ]; then
    err "未找到 AppImage 或签名文件"
    exit 1
  fi

  LINUX_FILENAME=$(basename "$LINUX_ARCHIVE")
  LINUX_SIG_CONTENT=$(cat "$LINUX_SIG")
  PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  cat > "$DIST_DIR/latest.json" <<EOF
{
  "version": "$VERSION",
  "notes": "FastClaw v$VERSION",
  "pub_date": "$PUB_DATE",
  "platforms": {
    "linux-x86_64": {
      "url": "REPLACE_WITH_DOWNLOAD_URL/$LINUX_FILENAME",
      "signature": "$LINUX_SIG_CONTENT"
    }
  }
}
EOF

  ok "latest.json 已生成"
  echo ""
  echo "  ⚠ 请编辑 $DIST_DIR/latest.json 中的 url 字段"
  echo "    将 REPLACE_WITH_DOWNLOAD_URL 替换为实际的下载地址"
  echo ""
fi

#── 完成 ──────────────────────────────────────────────────────────────

echo ""
ok "Linux 构建完成! 产物位于: $DIST_DIR/"
echo ""
echo "  产物列表:"
for f in "$DIST_DIR"/*; do
  SIZE=$(du -h "$f" | cut -f1)
  echo "    $(basename "$f")  ($SIZE)"
done
echo ""

if [ "$RELEASE_MODE" = true ]; then
  echo "  发布步骤:"
  echo "    1. 上传 dist/ 中的所有文件到发布渠道"
  echo "    2. 编辑 latest.json 中的 url 为实际下载地址"
  echo "    3. 将 latest.json 放到更新端点 URL 可访问的位置"
fi
