# Stage 8a Daemon E2E — PowerShell 兼容版
#
# 用法:
#   powershell -ExecutionPolicy Bypass -File scripts/e2e/daemon-llm.ps1
#
# 前置:
# - 编译好的 qx: target/release/qx.exe
# - ~/.qianxun/config.json 含 minimax + deepseek
# - env QIANXUN_JWT_SECRET=test-jwt-secret-2026-stage8a
#
# 输出:
# - 控制台逐步打印
# - transcript: qianxun/src/daemon/deliverable-8a-daemon.md

$ErrorActionPreference = "Stop"

# ─── Config ───────────────────────────────────────────────────
$Port = if ($env:DAEMON_PORT) { $env:DAEMON_PORT } else { 23910 }
$JwtSecret = "test-jwt-secret-2026-stage8a"
$DaemonBin = if ($env:DAEMON_BIN) { $env:DAEMON_BIN } else { "target\release\qx.exe" }
$LogDir = if ($env:DAEMON_LOG_DIR) { $env:DAEMON_LOG_DIR } else { "$env:TEMP\daemon-e2e" }
$ConfigPath = Join-Path $env:USERPROFILE ".qianxun\config.json"
$Deliverable = "qianxun\src\daemon\deliverable-8a-daemon.md"

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$Stdout = "$LogDir\daemon-stdout.log"
$Stderr = "$LogDir\daemon-stderr.log"
$SseMinimax = "$LogDir\sse-minimax.txt"
$SseDeepseek = "$LogDir\sse-deepseek.txt"
"" | Set-Content $Stdout
"" | Set-Content $Stderr
"" | Set-Content $SseMinimax
"" | Set-Content $SseDeepseek

# ─── Helpers ──────────────────────────────────────────────────
function Log($msg) { Write-Host "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] $msg" }
function Fail($msg) { Write-Host "[FAIL] $msg" -ForegroundColor Red; exit 1 }
function Ok($msg) { Write-Host "[OK]   $msg" -ForegroundColor Green }

function Mint-Jwt {
    param([string]$Sub = "test_e2e")
    $now = [int][double]::Parse((Get-Date -UFormat %s))
    $exp = $now + 3600
    $header = [System.Text.StringBuilder]::new().Append('{').Append('"alg":"HS256","typ":"JWT"').Append('}').ToString()
    $payloadObj = @{
        sub = $Sub
        exp = $exp
        iat = $now
    }
    $payload = $payloadObj | ConvertTo-Json -Compress
    $b64 = {
        param($s)
        [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($s)).Replace('+','-').Replace('/','_').TrimEnd('=')
    }
    $h = & $b64 $header
    $p = & $b64 $payload
    $hmac = New-Object System.Security.Cryptography.HMACSHA256
    $hmac.Key = [Text.Encoding]::UTF8.GetBytes($JwtSecret)
    $hash = $hmac.ComputeHash([Text.Encoding]::UTF8.GetBytes("$h.$p"))
    $sig = [Convert]::ToBase64String($hash).Replace('+','-').Replace('/','_').TrimEnd('=')
    return "$h.$p.$sig"
}

# ─── Step 0: preflight ────────────────────────────────────────
Log "===== Stage 8a Daemon E2E (PowerShell) ====="
Log "Step 0: preflight"
if (-not (Test-Path $ConfigPath)) { Fail "config not found: $ConfigPath" }
if (-not (Test-Path $DaemonBin)) { Fail "daemon binary not found: $DaemonBin" }
Log "  - config: $ConfigPath"
Log "  - binary: $DaemonBin"
Log "  - port:   $Port"
Log "  - log:    $LogDir"

# ─── Step 1: 启 daemon ────────────────────────────────────────
Log "Step 1: launching daemon"
$env:QIANXUN_JWT_SECRET = $JwtSecret
$proc = Start-Process -FilePath $DaemonBin `
    -ArgumentList "--daemon","--port","$Port" `
    -RedirectStandardOutput $Stdout `
    -RedirectStandardError $Stderr `
    -NoNewWindow -PassThru
Log "  - daemon PID: $($proc.Id)"
Start-Sleep -Seconds 2

# ─── Step 2: health ──────────────────────────────────────────
Log "Step 2: GET /v1/system/health"
$h = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/v1/system/health" -Method Get
Log "  - body: $h"
Ok "health 200"

# ─── Step 3: JWT ─────────────────────────────────────────────
Log "Step 3: minting JWT"
$Jwt = Mint-Jwt
Log "  - jwt: $($Jwt.Substring(0, 40))..."

# ─── Step 4: list providers ──────────────────────────────────
Log "Step 4: GET /v1/llm/providers"
$hdrs = @{ authorization = "Bearer $Jwt" }
$providers = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/v1/llm/providers" -Headers $hdrs -Method Get
Log "  - count: $($providers.providers.Count)"
Log "  - providers: $($providers.providers | ConvertTo-Json -Compress)"
$activeId = ($providers.providers | Where-Object { $_.is_active })[0].id
Log "  - active: $activeId"
if ($providers.providers.Count -lt 2) { Fail "expected ≥ 2 providers" }
Ok "got $($providers.providers.Count) providers"

# ─── Step 5: test minimax ────────────────────────────────────
Log "Step 5: POST /v1/llm/providers/minimax/test"
$sw = [Diagnostics.Stopwatch]::StartNew()
$test = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/v1/llm/providers/minimax/test" -Headers $hdrs -Method Post
$sw.Stop()
Log "  - response: $($test | ConvertTo-Json -Compress)"
Log "  - elapsed: $($sw.ElapsedMilliseconds)ms"
if (-not $test.ok) { Fail "minimax test failed: $($test | ConvertTo-Json)" }
Ok "minimax ok"

# ─── Step 6: create session ─────────────────────────────────
Log "Step 6: POST /v1/chat/session"
$session = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/v1/chat/session" -Headers $hdrs -Method Post
$SessionId = $session.session_id
Log "  - session_id: $SessionId"
Ok "session created"

# ─── Step 7: prompt minimax ──────────────────────────────────
Log "Step 7: POST /v1/chat/session/$SessionId/prompt (minimax)"
$body = '{"messages":[{"role":"user","content":"用一句话介绍 Rust 编程语言"}]}'
$promptUri = "http://127.0.0.1:$Port/v1/chat/session/$SessionId/prompt"

$sw.Reset()
$sw.Start()
try {
    $sseResp = Invoke-WebRequest -Uri $promptUri -Headers $hdrs -Method Post `
        -ContentType "application/json" -Body $body `
        -TimeoutSec 60
    $sseText = $sseResp.Content
    $sw.Stop()
    Log "  - elapsed: $($sw.ElapsedMilliseconds)ms"
    Log "  - status:  $($sseResp.StatusCode)"
    Set-Content -Path $SseMinimax -Value $sseText
} catch {
    $sw.Stop()
    Log "  - elapsed: $($sw.ElapsedMilliseconds)ms"
    # 503/200 with stream — try direct
    Fail "prompt request failed: $_"
}
Log "  - transcript: $SseMinimax"

# 解析 SSE
$minimaxEvents = @()
$minimaxText = ""
foreach ($line in (Get-Content $SseMinimax)) {
    if ($line -like "data: *") {
        $payload = $line.Substring(6).Trim()
        if ($payload) {
            try {
                $ev = $payload | ConvertFrom-Json
                $minimaxEvents += $ev.type
                if ($ev.type -eq "text_delta") { $minimaxText += $ev.text }
            } catch { }
        }
    }
}
$minimaxDistinct = ($minimaxEvents | Sort-Object -Unique) -join ","
Log "  - distinct types: $minimaxDistinct"
Log "  - text length:    $($minimaxText.Length)"
Log "  - preview:        $($minimaxText.Substring(0, [Math]::Min(120, $minimaxText.Length)))"

# ─── Step 8: 切 deepseek, 重复 ─────────────────────────────
Log "Step 8: switch active → deepseek + new session + prompt"
$activate = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/v1/llm/providers/deepseek/activate" -Headers $hdrs -Method Post
Log "  - activate: $($activate | ConvertTo-Json -Compress)"

$session2 = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/v1/chat/session" -Headers $hdrs -Method Post
$Session2 = $session2.session_id
Log "  - session2: $Session2"

$promptUri2 = "http://127.0.0.1:$Port/v1/chat/session/$Session2/prompt"
$sw.Reset()
$sw.Start()
try {
    $sseResp2 = Invoke-WebRequest -Uri $promptUri2 -Headers $hdrs -Method Post `
        -ContentType "application/json" -Body $body `
        -TimeoutSec 60
    $sseText2 = $sseResp2.Content
    $sw.Stop()
    Log "  - elapsed: $($sw.ElapsedMilliseconds)ms"
    Set-Content -Path $SseDeepseek -Value $sseText2
} catch {
    $sw.Stop()
    Fail "deepseek prompt failed: $_"
}

$deepseekEvents = @()
$deepseekText = ""
foreach ($line in (Get-Content $SseDeepseek)) {
    if ($line -like "data: *") {
        $payload = $line.Substring(6).Trim()
        if ($payload) {
            try {
                $ev = $payload | ConvertFrom-Json
                $deepseekEvents += $ev.type
                if ($ev.type -eq "text_delta") { $deepseekText += $ev.text }
            } catch { }
        }
    }
}
$deepseekDistinct = ($deepseekEvents | Sort-Object -Unique) -join ","
Log "  - distinct types: $deepseekDistinct"
Log "  - text length:    $($deepseekText.Length)"
Log "  - preview:        $($deepseekText.Substring(0, [Math]::Min(120, $deepseekText.Length)))"

# ─── Step 9: cleanup ─────────────────────────────────────────
Log "Step 9: killing daemon (PID $($proc.Id))"
try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch {}

# ─── Final verdict ──────────────────────────────────────────
Log "===== Verdict ====="
Log "  - minimax events:  $minimaxDistinct"
Log "  - minimax text:    $($minimaxText.Length) chars"
Log "  - deepseek events: $deepseekDistinct"
Log "  - deepseek text:   $($deepseekText.Length) chars"
Ok "E2E completed. Transcripts in $LogDir"
Log "Tip: cat $Stdout $Stderr $SseMinimax $SseDeepseek > $Deliverable"
