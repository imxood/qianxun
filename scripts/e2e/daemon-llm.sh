#!/usr/bin/env bash
# Stage 8a Daemon E2E — bash orchestrator (PowerShell 兼容 + Git-Bash 兼容)
#
# 用法:
#   bash scripts/e2e/daemon-llm.sh
#
# 平台行为:
#   - Linux/macOS/WSL bash → 直接调用本脚本, 但 daemon 是 Windows .exe 时
#     需要改用 scripts/e2e/daemon-llm.ps1
#   - Git-Bash on Windows → bash 可直接 exec qx.exe
#   - PowerShell (推荐 Windows 平台) → powershell -File scripts/e2e/daemon-llm.ps1
#
# 推荐在 Windows 上跑 PowerShell 版: scripts/e2e/daemon-llm.ps1
# 推荐在 macOS/Linux 上跑: 直接调 daemon 二进制 (Linux build)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 检测平台
IS_WINDOWS=0
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" || "$OSTYPE" == "cygwin" ]]; then
    IS_WINDOWS=1
fi
if [[ "$(uname -r 2>/dev/null)" == *microsoft* ]]; then
    # WSL bash: 不能直接 exec .exe, 推荐走 PowerShell 版
    IS_WINDOWS=0
    ON_WSL=1
else
    ON_WSL=0
fi

if [[ $IS_WINDOWS -eq 1 ]]; then
    # Git-Bash on Windows: 直接 exec daemon .exe
    exec bash "$SCRIPT_DIR/daemon-llm-direct.sh"
elif [[ $ON_WSL -eq 1 ]]; then
    # WSL bash: 不能 exec .exe, 委托给 PowerShell 版
    echo "[daemon-llm.sh] WSL detected, delegating to PowerShell version..."
    WIN_PS_PATH="$(echo "$SCRIPT_DIR" | sed 's|^/mnt/\([a-z]\)/|\1:\\|; s|/|\\|g')"
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${WIN_PS_PATH}\\daemon-llm.ps1"
else
    echo "[daemon-llm.sh] This is the bash variant for Git-Bash on Windows or native Linux/macOS."
    echo "[daemon-llm.sh] On Windows, use:  powershell -File $SCRIPT_DIR/daemon-llm.ps1"
    echo "[daemon-llm.sh] On WSL,         this script auto-delegates to PowerShell."
    echo "[daemon-llm.sh] Trying direct execution anyway (assumes daemon is buildable for this platform)..."
    exec bash "$SCRIPT_DIR/daemon-llm-direct.sh"
fi
