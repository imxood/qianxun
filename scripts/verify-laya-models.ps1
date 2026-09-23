# Verify/restore Laya model bundles (per-bundle manifest.json, sha256)
# Usage:
#   powershell -File scripts/verify-laya-models.ps1              # all bundles
#   powershell -File scripts/verify-laya-models.ps1 -Bundle laya-onnx
#
# Model weights are NOT in git (models/ is ignored). Integrity & versioning
# live in each bundle's manifest.json. Restore via manifest.source:
#   - local-export: regenerate via the export pipeline (docs/09 §2)
#   - hf:<repo>   : future fetch script pulling from HuggingFace

param([string]$Bundle = '')

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$bundlesRoot = Join-Path $root 'models'
$targets = if ($Bundle) { @(Join-Path $bundlesRoot $Bundle) } else { Get-ChildItem $bundlesRoot -Directory | ForEach-Object { $_.FullName } }

$failed = 0
foreach ($dir in $targets) {
    $manifest = Join-Path $dir 'manifest.json'
    if (-not (Test-Path $manifest)) {
        Write-Host "SKIP (no manifest): $dir" -ForegroundColor Yellow
        continue
    }
    $m = Get-Content $manifest -Raw | ConvertFrom-Json
    Write-Host "-- $($m.bundle)  source=$($m.source)"
    foreach ($f in $m.files) {
        $p = Join-Path $dir ($f.path -replace '/', '\')
        if (-not (Test-Path $p)) {
            Write-Host "  MISSING  $($f.path)" -ForegroundColor Red
            $failed += 1
            continue
        }
        $len = (Get-Item $p).Length
        if ($len -ne $f.bytes) {
            Write-Host "  SIZE!=   $($f.path) ($len != $($f.bytes))" -ForegroundColor Red
            $failed += 1
            continue
        }
        $h = (Get-FileHash $p -Algorithm SHA256).Hash.ToLower()
        if ($h -ne $f.sha256) {
            Write-Host "  SHA!=    $($f.path)" -ForegroundColor Red
            $failed += 1
        }
        else {
            Write-Host ("  ok       {0}  ({1:N1} MB)" -f $f.path, ($f.bytes / 1MB)) -ForegroundColor Green
        }
    }
}
if ($failed -gt 0) {
    Write-Host ""
    Write-Host "$failed file(s) failed/missing. See manifest.source and docs/09." -ForegroundColor Red
    exit 1
}
Write-Host ""
Write-Host "All bundles verified." -ForegroundColor Green
