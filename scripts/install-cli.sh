#!/usr/bin/env bash
set -euo pipefail

#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# FastClaw CLI — 安装脚本
#
# 用法:
#   ./scripts/install-cli.sh              # 安装到 ~/.local/bin/
#   ./scripts/install-cli.sh /usr/local/bin   # 安装到指定目录
#   ./scripts/install-cli.sh --build     # 构建并安装
#━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

INSTALL_DIR="${HOME}/.local/bin"
DO_BUILD=false

for arg in "$@"; do
  case "$arg" in
    --build) DO_BUILD=true ;;
    --help|-h)
      echo "用法: $0 [--build] [INSTALL_DIR]"
      echo ""
      echo "  --build       先构建再安装 (cargo build --release)"
      echo "  INSTALL_DIR   安装目标目录 (默认: ~/.local/bin/)"
      exit 0
      ;;
    *)
      if [ -d "$arg" ] || [[ "$arg" == /* ]]; then
        INSTALL_DIR="$arg"
      fi
      ;;
  esac
done

log() { echo -e "\033[1;36m▸ $1\033[0m"; }
ok()  { echo -e "\033[1;32m✓ $1\033[0m"; }
err() { echo -e "\033[1;31m✗ $1\033[0m" >&2; }

BIN_PATH="$PROJECT_ROOT/target/release/fastclaw"

if [ "$DO_BUILD" = true ]; then
  log "构建 fastclaw CLI (release)..."
  (cd "$PROJECT_ROOT" && cargo build --release --package fastclaw-cli)
  ok "构建完成"
fi

if [ ! -f "$BIN_PATH" ]; then
  err "未找到编译产物: $BIN_PATH"
  echo "  请先运行: cargo build --release --package fastclaw-cli"
  echo "  或使用: $0 --build"
  exit 1
fi

mkdir -p "$INSTALL_DIR"
cp "$BIN_PATH" "$INSTALL_DIR/fastclaw"
chmod +x "$INSTALL_DIR/fastclaw"
ok "已安装: $INSTALL_DIR/fastclaw"

# Check if install dir is in PATH
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo ""
  echo "  ⚠ $INSTALL_DIR 不在 PATH 中"
  echo "  请添加到 shell 配置文件:"
  echo ""
  echo "    echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc"
  echo ""
  echo "  然后重新加载: source ~/.bashrc"
else
  echo ""
  INSTALLED_VERSION=$("$INSTALL_DIR/fastclaw" --version 2>/dev/null || echo "unknown")
  ok "fastclaw 已可用: $INSTALLED_VERSION"
fi
