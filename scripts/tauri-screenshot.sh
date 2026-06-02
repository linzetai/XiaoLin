#!/bin/bash
# Tauri window screenshot helper for Linux
# Uses ImageMagick's import to capture the XiaoLin Tauri window
# Bypasses the broken native Linux screenshot in tauri-plugin-mcp-bridge

set -euo pipefail

OUTPUT="${1:-/tmp/xiaolin-screenshot.png}"

# Find the XiaoLin window with the expected size (1100x720)
WIN_ID=""
for WID in $(xdotool search --name "XiaoLin" 2>/dev/null); do
    GEOM=$(xdotool getwindowgeometry "$WID" 2>/dev/null | grep "Geometry" | awk '{print $2}')
    if [ "$GEOM" = "1100x720" ]; then
        WIN_ID="$WID"
        break
    fi
done

if [ -z "$WIN_ID" ]; then
    echo "ERROR: XiaoLin window (1100x720) not found" >&2
    exit 1
fi

import -window "$WIN_ID" "$OUTPUT" 2>&1
if [ $? -eq 0 ]; then
    echo "Screenshot saved: $OUTPUT"
    identify "$OUTPUT"
else
    echo "ERROR: import failed" >&2
    exit 1
fi
