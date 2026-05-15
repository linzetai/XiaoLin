#!/usr/bin/env bash
set -euo pipefail

#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# FastClaw — macOS 本地打包脚本
#
# 用法:
#   ./scripts/build-macos.sh              # 正常构建
#   ./scripts/build-macos.sh --release    # 构建 + 生成 latest.json
#   ./scripts/build-macos.sh --skip-lint  # 跳过 clippy 检查
#   ./scripts/build-macos.sh --universal  # 构建 Universal Binary (Intel + Apple Silicon)
#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_DIR="$PROJECT_ROOT/crates/fastclaw-app"
TAURI_DIR="$APP_DIR/src-tauri"
DIST_DIR="$PROJECT_ROOT/dist"
KEY_PATH="$HOME/.tauri/fastclaw.key"

SKIP_LINT=false
RELEASE_MODE=false
UNIVERSAL=false

for arg in "$@"; do
  case "$arg" in
    --skip-lint) SKIP_LINT=true ;;
    --release)   RELEASE_MODE=true ;;
    --universal) UNIVERSAL=true ;;
  esac
done

log() { echo -e "\033[1;36m▸ $1\033[0m"; }
err() { echo -e "\033[1;31m✗ $1\033[0m" >&2; }
ok()  { echo -e "\033[1;32m✓ $1\033[0m"; }

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
ok "Cargo: $(cargo --version)"
ok "Node:  $(node --version)"
ok "pnpm:  $(pnpm --version)"

#── Lint ──────────────────────────────────────────────────────────────

if [ "$SKIP_LINT" = false ]; then
  log "运行 clippy..."
  (cd "$PROJECT_ROOT" && cargo clippy --manifest-path "$TAURI_DIR/Cargo.toml" --no-deps)
  ok "Clippy 通过"
fi

#── 前端构建 ──────────────────────────────────────────────────────────

log "安装前端依赖..."
(cd "$APP_DIR" && pnpm install --frozen-lockfile)

log "构建前端..."
(cd "$APP_DIR" && pnpm build)
ok "前端构建完成"

#── Tauri 构建 ────────────────────────────────────────────────────────

log "构建 Tauri 应用 (macOS)..."

if [ "${CI:-}" = "1" ]; then
  export CI=true
elif [ "${CI:-}" = "0" ]; then
  export CI=false
fi

if [ "$UNIVERSAL" = true ]; then
  log "构建 Universal Binary (Intel + Apple Silicon)..."
  (cd "$APP_DIR" && pnpm exec -- tauri build --target universal-apple-darwin)
else
  (cd "$APP_DIR" && pnpm exec -- tauri build)
fi

ok "Tauri 构建完成"

#── DMG Fallback ──────────────────────────────────────────────────────

# 如果 Tauri 没有生成 DMG（AppleScript 失败），用 hdiutil 手动创建
APP_PATH=""
for BUNDLE_DIR in "$PROJECT_ROOT/target/tauri/release/bundle" "$PROJECT_ROOT/target/release/bundle"; do
  [ -d "$BUNDLE_DIR/macos" ] || continue
  APP_PATH=$(find "$BUNDLE_DIR/macos" -name "*.app" -type d 2>/dev/null | head -1)
  [ -n "$APP_PATH" ] && break
done

if [ -n "$APP_PATH" ] && [ -d "$APP_PATH" ]; then
  DMG_EXISTS=false
  for BUNDLE_DIR in "$PROJECT_ROOT/target/tauri/release/bundle" "$PROJECT_ROOT/target/release/bundle"; do
    [ -d "$BUNDLE_DIR/dmg" ] || continue
    if find "$BUNDLE_DIR/dmg" -name "*.dmg" ! -name "rw.*" -type f 2>/dev/null | grep -q .; then
      DMG_EXISTS=true
      break
    fi
  done

  if [ "$DMG_EXISTS" = false ]; then
    log "Tauri DMG 打包未成功，使用 hdiutil 手动创建..."
    DMG_NAME="FastClaw_${VERSION}_$(uname -m).dmg"
    DMG_PATH="$PROJECT_ROOT/target/tauri/release/bundle/dmg/$DMG_NAME"
    mkdir -p "$(dirname "$DMG_PATH")"
    hdiutil create -volname "FastClaw" -srcfolder "$APP_PATH" -ov -format UDZO -imagekey zlib-level=9 "$DMG_PATH"
    ok "DMG 已创建: $DMG_NAME"
  fi
fi

#── 构建 CLI 二进制 ──────────────────────────────────────────────────

log "构建 FastClaw CLI (fastclaw)..."
if [ "$UNIVERSAL" = true ]; then
  (cd "$PROJECT_ROOT" && cargo build --release --package fastclaw-cli --target aarch64-apple-darwin)
  (cd "$PROJECT_ROOT" && cargo build --release --package fastclaw-cli --target x86_64-apple-darwin)
else
  (cd "$PROJECT_ROOT" && cargo build --release --package fastclaw-cli)
fi
ok "CLI 构建完成"

#── 收集产物 ──────────────────────────────────────────────────────────

log "收集构建产物..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# Tauri 2.10+ uses target/tauri/release/bundle/
for BUNDLE_DIR in "$PROJECT_ROOT/target/tauri/release/bundle" "$PROJECT_ROOT/target/release/bundle"; do
  [ -d "$BUNDLE_DIR" ] || continue

  # macOS .app bundle
  find "$BUNDLE_DIR" -name "*.app" -type d -exec cp -R {} "$DIST_DIR/" \; 2>/dev/null || true

  # macOS .dmg
  for pattern in "*.dmg" "*.dmg.sig"; do
    find "$BUNDLE_DIR" -maxdepth 2 -name "$pattern" -exec cp {} "$DIST_DIR/" \; 2>/dev/null || true
  done

  # Universal binary targets
  if [ "$UNIVERSAL" = true ]; then
    for pattern in "*.app.tar.gz" "*.app.tar.gz.sig"; do
      find "$BUNDLE_DIR" -maxdepth 3 -name "$pattern" -exec cp {} "$DIST_DIR/" \; 2>/dev/null || true
    done
  fi
done

# CLI binary
if [ "$UNIVERSAL" = true ]; then
  AARCH64_BIN="$PROJECT_ROOT/target/aarch64-apple-darwin/release/fastclaw"
  X86_BIN="$PROJECT_ROOT/target/x86_64-apple-darwin/release/fastclaw"
  if [ -f "$AARCH64_BIN" ] && [ -f "$X86_BIN" ]; then
    lipo -create "$AARCH64_BIN" "$X86_BIN" -output "$DIST_DIR/fastclaw"
    chmod +x "$DIST_DIR/fastclaw"
    (cd "$DIST_DIR" && tar czf "fastclaw-cli-${VERSION}-darwin-universal.tar.gz" fastclaw)
    ok "CLI 已打包: fastclaw-cli-${VERSION}-darwin-universal.tar.gz (universal)"
  else
    err "CLI 二进制未找到 (universal targets)"
  fi
else
  CLI_BIN="$PROJECT_ROOT/target/release/fastclaw"
  if [ -f "$CLI_BIN" ]; then
    cp "$CLI_BIN" "$DIST_DIR/fastclaw"
    chmod +x "$DIST_DIR/fastclaw"
    ARCH="x86_64"
    if [ "$(uname -m)" = "arm64" ]; then ARCH="aarch64"; fi
    (cd "$DIST_DIR" && tar czf "fastclaw-cli-${VERSION}-darwin-${ARCH}.tar.gz" fastclaw)
    ok "CLI 已打包: fastclaw-cli-${VERSION}-darwin-${ARCH}.tar.gz"
  else
    err "CLI 二进制未找到: $CLI_BIN"
  fi
fi

ok "产物已收集到 $DIST_DIR/"
ls -lh "$DIST_DIR/"

#── 生成 latest.json (--release 模式) ────────────────────────────────

if [ "$RELEASE_MODE" = true ]; then
  log "生成 latest.json..."

  # macOS .dmg
  MACOS_ARCHIVE=$(find "$DIST_DIR" -name "*.dmg" ! -name "*.sig" | head -1)
  MACOS_SIG=$(find "$DIST_DIR" -name "*.dmg.sig" | head -1)

  if [ -z "$MACOS_ARCHIVE" ]; then
    # Fallback to .app.tar.gz for universal builds
    MACOS_ARCHIVE=$(find "$DIST_DIR" -name "*.app.tar.gz" ! -name "*.sig" | head -1)
    MACOS_SIG=$(find "$DIST_DIR" -name "*.app.tar.gz.sig" | head -1)
  fi

  if [ -z "$MACOS_ARCHIVE" ] || [ -z "$MACOS_SIG" ]; then
    err "未找到 dmg/app.tar.gz 或签名文件"
    exit 1
  fi

  MACOS_FILENAME=$(basename "$MACOS_ARCHIVE")
  MACOS_SIG_CONTENT=$(cat "$MACOS_SIG")
  PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  # Detect architecture
  ARCH="x86_64"
  if [ "$UNIVERSAL" = true ]; then
    ARCH="universal"
  elif [ "$(uname -m)" = "arm64" ]; then
    ARCH="aarch64"
  fi

  cat > "$DIST_DIR/latest.json" <<EOF
{
  "version": "$VERSION",
  "notes": "FastClaw v$VERSION",
  "pub_date": "$PUB_DATE",
  "platforms": {
    "darwin-$ARCH": {
      "url": "REPLACE_WITH_DOWNLOAD_URL/$MACOS_FILENAME",
      "signature": "$MACOS_SIG_CONTENT"
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
ok "macOS 构建完成! 产物位于: $DIST_DIR/"
echo ""
echo "  产物列表:"
for f in "$DIST_DIR"/*; do
  if [ -d "$f" ]; then
    SIZE=$(du -sh "$f" | cut -f1)
    echo "    $(basename "$f")/  ($SIZE)"
  else
    SIZE=$(du -h "$f" | cut -f1)
    echo "    $(basename "$f")  ($SIZE)"
  fi
done
echo ""

if [ "$RELEASE_MODE" = true ]; then
  echo "  发布步骤:"
  echo "    1. 上传 dist/ 中的所有文件到发布渠道"
  echo "    2. 编辑 latest.json 中的 url 为实际下载地址"
  echo "    3. 将 latest.json 放到更新端点 URL 可访问的位置"
fi