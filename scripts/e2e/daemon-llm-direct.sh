#!/usr/bin/env bash
# Stage 8a Daemon E2E — 直接 bash 版 (Git-Bash on Windows / Linux native)
#
# 注意: WSL bash 不支持 (WSL 不能 exec Windows .exe)
# WSL 上请跑: bash scripts/e2e/daemon-llm.sh (会委托给 PowerShell 版)
# Git-Bash on Windows 可直接跑这个文件.

set -euo pipefail

# ─── 配置 ─────────────────────────────────────────────────────
PORT="${DAEMON_PORT:-23910}"
JWT_SECRET="test-jwt-secret-2026-stage8a"
DAEMON_LOG_DIR="${DAEMON_LOG_DIR:-$SCRIPT_DIR/../.e2e-logs}"
if [[ "$DAEMON_LOG_DIR" != /* ]]; then
    DAEMON_LOG_DIR="$(pwd)/${DAEMON_LOG_DIR}"
fi
CONFIG_PATH="${HOME}/.qianxun/config.json"
DELIVERABLE="${DELIVERABLE:-qianxun/src/daemon/deliverable-8a-daemon.md}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "$DAEMON_LOG_DIR"
DAEMON_STDOUT="$DAEMON_LOG_DIR/daemon-stdout.log"
DAEMON_STDERR="$DAEMON_LOG_DIR/daemon-stderr.log"
SSE_MINIMAX="$DAEMON_LOG_DIR/sse-minimax.txt"
SSE_DEEPSEEK="$DAEMON_LOG_DIR/sse-deepseek.txt"
: > "$DAEMON_STDOUT"
: > "$DAEMON_STDERR"
: > "$SSE_MINIMAX"
: > "$SSE_DEEPSEEK"

# ─── 工具函数 ─────────────────────────────────────────────────
log()  { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }
fail() { echo "[FAIL] $*" >&2; exit 1; }
ok()   { echo "[OK]   $*"; }

# 平台路径解析
resolve_config_path() {
    if [[ -f "$HOME/.qianxun/config.json" ]]; then
        echo "$HOME/.qianxun/config.json"
        return
    fi
    if [[ -d /mnt/c/Users ]]; then
        for u in "${WIN_USER:-}" "${USERNAME:-}" "maxu" "${SUDO_USER:-}"; do
            if [[ -n "$u" && -f "/mnt/c/Users/${u}/.qianxun/config.json" ]]; then
                echo "/mnt/c/Users/${u}/.qianxun/config.json"
                return
            fi
        done
    fi
    if [[ -n "${USERPROFILE:-}" ]]; then
        local wpath="${USERPROFILE//\\//}/.qianxun/config.json"
        if [[ -f "$wpath" ]]; then
            echo "$wpath"
            return
        fi
    fi
    echo "$CONFIG_PATH"
}

find_daemon_binary() {
    local candidates=(
        "$DAEMON_BIN"
        "$DAEMON_BIN.exe"
        "target/release/qx"
        "target/release/qx.exe"
        "target/debug/qx"
        "target/debug/qx.exe"
    )
    for p in "${candidates[@]}"; do
        if [[ -f "$p" ]]; then
            echo "$p"
            return
        fi
    done
    echo ""
}

# ─── Step 0: 前置检查 ─────────────────────────────────────────
log "===== Stage 8a Daemon E2E (bash direct) ====="
log "Step 0: preflight"

CONFIG_PATH_RESOLVED=$(resolve_config_path)
[[ -f "$CONFIG_PATH_RESOLVED" ]] || fail "~/.qianxun/config.json not found"
CONFIG_PATH="$CONFIG_PATH_RESOLVED"
log "  - config: $CONFIG_PATH"

DAEMON_BIN=$(find_daemon_binary)
[[ -n "$DAEMON_BIN" ]] || fail "daemon binary not found (tried target/release/qx + .exe)"
log "  - binary: $DAEMON_BIN"
log "  - port:   $PORT"
log "  - log:    $DAEMON_LOG_DIR"

# ─── Step 1: 启 daemon ────────────────────────────────────────
log "Step 1: launching daemon on port $PORT"
export QIANXUN_JWT_SECRET="$JWT_SECRET"
"$DAEMON_BIN" --daemon --port "$PORT" \
    > "$DAEMON_STDOUT" 2> "$DAEMON_STDERR" &
DAEMON_PID=$!
log "  - daemon PID: $DAEMON_PID"
echo "$DAEMON_PID" > "$DAEMON_LOG_DIR/daemon.pid"
sleep 2

# ─── Step 2: health check ─────────────────────────────────────
log "Step 2: GET /v1/system/health"
HEALTH=$(curl -sS -w "\n%{http_code}" "http://127.0.0.1:${PORT}/v1/system/health" || true)
HEALTH_CODE=$(echo "$HEALTH" | tail -n1)
HEALTH_BODY=$(echo "$HEALTH" | head -n-1)
log "  - status: $HEALTH_CODE"
log "  - body:   $HEALTH_BODY"
[[ "$HEALTH_CODE" == "200" ]] || fail "health check failed (code=$HEALTH_CODE)"
ok "health 200"

# ─── Step 3: mint JWT ────────────────────────────────────────
log "Step 3: minting JWT"
JWT=$(python3 "$SCRIPT_DIR/mint_jwt.py" "test_e2e" "$JWT_SECRET" \
    || python "$SCRIPT_DIR/mint_jwt.py" "test_e2e" "$JWT_SECRET")
log "  - jwt: ${JWT:0:40}..."

# ─── Step 4: 列出 providers ──────────────────────────────────
log "Step 4: GET /v1/llm/providers"
PROVIDERS_RESP=$(curl -sS -H "authorization: Bearer $JWT" "http://127.0.0.1:${PORT}/v1/llm/providers")
log "  - response: $PROVIDERS_RESP"
PROVIDER_COUNT=$(echo "$PROVIDERS_RESP" | python -c "import json,sys; d=json.load(sys.stdin); print(len(d.get('providers',[])))")
[[ "$PROVIDER_COUNT" -ge 2 ]] || fail "expected ≥ 2 providers, got $PROVIDER_COUNT"
ok "got $PROVIDER_COUNT providers"

ACTIVE_ID=$(echo "$PROVIDERS_RESP" | python -c "import json,sys; d=json.load(sys.stdin); print(next(p['id'] for p in d['providers'] if p.get('is_active')))")
log "  - active: $ACTIVE_ID"

# ─── Step 5: test minimax ─────────────────────────────────────
log "Step 5: POST /v1/llm/providers/minimax/test"
TEST_START=$(date +%s%3N)
TEST_MINIMAX=$(curl -sS -X POST -H "authorization: Bearer $JWT" \
    "http://127.0.0.1:${PORT}/v1/llm/providers/minimax/test")
TEST_END=$(date +%s%3N)
log "  - response: $TEST_MINIMAX"
log "  - elapsed:  $((TEST_END - TEST_START))ms"
MINIMAX_OK=$(echo "$TEST_MINIMAX" | python -c "import json,sys; print(json.load(sys.stdin).get('ok'))")
[[ "$MINIMAX_OK" == "True" ]] || fail "minimax test failed: $TEST_MINIMAX"
ok "minimax ok"

# ─── Step 6: 创建 session ─────────────────────────────────────
log "Step 6: POST /v1/chat/session"
SESSION_RESP=$(curl -sS -X POST -H "authorization: Bearer $JWT" \
    "http://127.0.0.1:${PORT}/v1/chat/session")
log "  - response: $SESSION_RESP"
SESSION_ID=$(echo "$SESSION_RESP" | python -c "import json,sys; print(json.load(sys.stdin)['session_id'])")
log "  - session_id: $SESSION_ID"
[[ -n "$SESSION_ID" ]] || fail "session_id empty"
ok "session created"

# ─── Step 7: 跑 prompt, 收 SSE 流 (minimax) ──────────────────
log "Step 7: POST /v1/chat/session/$SESSION_ID/prompt (minimax)"
PROMPT_BODY='{"messages":[{"role":"user","content":"用一句话介绍 Rust 编程语言"}]}'
PROMPT_START=$(date +%s%3N)
curl -sS --no-buffer -N \
    -H "authorization: Bearer $JWT" \
    -H "content-type: application/json" \
    -H "accept: text/event-stream" \
    -X POST \
    --data "$PROMPT_BODY" \
    -o "$SSE_MINIMAX" \
    -w "HTTP_CODE=%{http_code} TIME=%{time_total}s SIZE=%{size_download}\n" \
    "http://127.0.0.1:${PORT}/v1/chat/session/${SESSION_ID}/prompt"
PROMPT_END=$(date +%s%3N)
log "  - elapsed: $((PROMPT_END - PROMPT_START))ms"
log "  - transcript: $SSE_MINIMAX"

log "  - parsing minimax SSE..."
python3 "$SCRIPT_DIR/parse_sse.py" "$SSE_MINIMAX" | tee "$DAEMON_LOG_DIR/minimax-summary.txt"

# ─── Step 8: 切到 deepseek, 重复 ─────────────────────────────
log "Step 8: switch active → deepseek + new session + prompt"

ACTIVATE_RESP=$(curl -sS -X POST -H "authorization: Bearer $JWT" \
    "http://127.0.0.1:${PORT}/v1/llm/providers/deepseek/activate")
log "  - activate: $ACTIVATE_RESP"

SESSION2=$(curl -sS -X POST -H "authorization: Bearer $JWT" \
    "http://127.0.0.1:${PORT}/v1/chat/session" | python -c "import json,sys; print(json.load(sys.stdin)['session_id'])")
log "  - session2: $SESSION2"

log "  - prompt (deepseek)..."
PROMPT_START2=$(date +%s%3N)
curl -sS --no-buffer -N \
    -H "authorization: Bearer $JWT" \
    -H "content-type: application/json" \
    -H "accept: text/event-stream" \
    -X POST \
    --data "$PROMPT_BODY" \
    -o "$SSE_DEEPSEEK" \
    -w "HTTP_CODE=%{http_code} TIME=%{time_total}s SIZE=%{size_download}\n" \
    "http://127.0.0.1:${PORT}/v1/chat/session/${SESSION2}/prompt"
PROMPT_END2=$(date +%s%3N)
log "  - elapsed: $((PROMPT_END2 - PROMPT_START2))ms"

log "  - parsing deepseek SSE..."
python3 "$SCRIPT_DIR/parse_sse.py" "$SSE_DEEPSEEK" | tee "$DAEMON_LOG_DIR/deepseek-summary.txt"

# 验证 active 切换
log "  - final list (verify active swap):"
curl -sS -H "authorization: Bearer $JWT" "http://127.0.0.1:${PORT}/v1/llm/providers" \
    | python -m json.tool

# ─── Step 9: cleanup ─────────────────────────────────────────
log "Step 9: killing daemon (PID $DAEMON_PID)"
kill "$DAEMON_PID" 2>/dev/null || true
sleep 1
kill -9 "$DAEMON_PID" 2>/dev/null || true

# ─── Final verdict ──────────────────────────────────────────
log "===== Verdict ====="
log "  - minimax SSE:  $SSE_MINIMAX"
log "  - deepseek SSE: $SSE_DEEPSEEK"
ok "E2E completed. Transcripts in $DAEMON_LOG_DIR"
