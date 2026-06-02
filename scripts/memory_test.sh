#!/usr/bin/env bash
set -euo pipefail

# Memory usage test for XiaoLin Gateway
# Validates RSS memory usage under different load conditions.
#
# Requirements:
#   - XiaoLin gateway binary built (target/release/xiaolin-gateway or cargo build first)
#   - curl, jq available
#   - Linux or macOS (uses /proc or ps for RSS measurement)
#
# Usage:
#   ./scripts/memory_test.sh [--release]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PORT=18799  # Use non-default port to avoid conflicts
GATEWAY_PID=""
ALL_PASS=true

BINARY="$PROJECT_ROOT/target/release/xiaolin-gateway"
if [[ "${1:-}" == "--debug" ]]; then
    BINARY="$PROJECT_ROOT/target/debug/xiaolin-gateway"
fi

# ─── Helpers ─────────────────────────────────────────────────────────────────

cleanup() {
    if [[ -n "$GATEWAY_PID" ]]; then
        kill "$GATEWAY_PID" 2>/dev/null || true
        wait "$GATEWAY_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

get_rss_kb() {
    local pid=$1
    if [[ -f "/proc/$pid/status" ]]; then
        # Linux: read VmRSS from /proc
        grep VmRSS "/proc/$pid/status" | awk '{print $2}'
    else
        # macOS: use ps
        ps -o rss= -p "$pid" | tr -d ' '
    fi
}

get_rss_mb() {
    local pid=$1
    local rss_kb
    rss_kb=$(get_rss_kb "$pid")
    echo "scale=1; $rss_kb / 1024" | bc
}

wait_for_health() {
    local max_wait=10
    local i=0
    while [[ $i -lt $max_wait ]]; do
        if curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.5
        i=$((i + 1))
    done
    echo "ERROR: Gateway failed to start within ${max_wait}s"
    return 1
}

check_threshold() {
    local label="$1"
    local actual_mb="$2"
    local limit_mb="$3"
    local pass
    pass=$(echo "$actual_mb < $limit_mb" | bc -l)
    if [[ "$pass" == "1" ]]; then
        printf "  [PASS] %-40s %6s MB (limit: %s MB)\n" "$label" "$actual_mb" "$limit_mb"
    else
        printf "  [FAIL] %-40s %6s MB (limit: %s MB)\n" "$label" "$actual_mb" "$limit_mb"
        ALL_PASS=false
    fi
}

send_message() {
    local session_id="$1"
    local content="$2"
    curl -sf -X POST "http://127.0.0.1:$PORT/api/v1/chat" \
        -H "Content-Type: application/json" \
        -d "{\"session_id\": \"$session_id\", \"message\": \"$content\"}" \
        >/dev/null 2>&1 || true
}

# ─── Build if needed ─────────────────────────────────────────────────────────

if [[ ! -f "$BINARY" ]]; then
    echo "Building xiaolin-gateway (release)..."
    cargo build --release -p xiaolin-gateway
fi

echo "═══════════════════════════════════════════════════════════════"
echo "  XiaoLin Memory Usage Test"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# ─── Start Gateway ───────────────────────────────────────────────────────────

export XIAOLIN_GATEWAY_PORT="$PORT"
export XIAOLIN_LOG_LEVEL="warn"

"$BINARY" &
GATEWAY_PID=$!
wait_for_health

echo "Gateway started (PID: $GATEWAY_PID, port: $PORT)"
echo ""

# Allow process to settle
sleep 2

# ─── Test 1: Idle State RSS ──────────────────────────────────────────────────

echo "── Test 1: Idle State ──"
IDLE_RSS=$(get_rss_mb "$GATEWAY_PID")
check_threshold "Idle gateway RSS" "$IDLE_RSS" "30"
echo ""

# ─── Test 2: 10 Active Sessions ─────────────────────────────────────────────

echo "── Test 2: 10 Active Sessions ──"
for i in $(seq 1 10); do
    session_id="mem-test-session-$i"
    send_message "$session_id" "Hello, this is test message $i for memory measurement."
    send_message "$session_id" "Another message with some content to simulate an active conversation turn $i."
done

sleep 2
ACTIVE_RSS=$(get_rss_mb "$GATEWAY_PID")
check_threshold "10 active sessions RSS" "$ACTIVE_RSS" "80"
echo ""

# ─── Test 3: Long Session (200 turns) ───────────────────────────────────────

echo "── Test 3: Long Session (200 turns, simulated) ──"
long_session="mem-test-long-session"
for i in $(seq 1 200); do
    msg="Turn $i: This is a moderately long message to simulate real conversation content with enough tokens to trigger context compression after accumulation. $(date +%s%N)"
    send_message "$long_session" "$msg"
done

sleep 3
LONG_RSS=$(get_rss_mb "$GATEWAY_PID")
check_threshold "200-turn long session RSS" "$LONG_RSS" "100"
echo ""

# ─── Summary ─────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════"
echo "  Summary"
echo "═══════════════════════════════════════════════════════════════"
printf "  Idle:           %6s MB\n" "$IDLE_RSS"
printf "  10 sessions:    %6s MB\n" "$ACTIVE_RSS"
printf "  200-turn long:  %6s MB\n" "$LONG_RSS"
echo ""

if $ALL_PASS; then
    echo "  ✓ All memory usage checks passed."
    exit 0
else
    echo "  ✗ Some memory usage checks FAILED!"
    exit 1
fi
