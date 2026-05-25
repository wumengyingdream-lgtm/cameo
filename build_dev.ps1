# build_dev.ps1 — kill running instances, build a debug cameo.exe, print the path.
#
# Unsigned, fast iteration. Skips the NSIS installer (use build_release.ps1 for
# that) and produces the raw executable you can double-click.
#
# Usage (PowerShell):
#   .\build_dev.ps1            # kill running + build, then print the path
#   .\build_dev.ps1 -Open      # ...and launch it
#   .\build_dev.ps1 -Clean     # cargo clean first (full rebuild)
#
# Output: src-tauri\target\debug\cameo.exe
# macOS: use build_dev.sh instead.

param(
  [switch]$Open,
  [switch]$Clean
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

function Info($m) { Write-Host "-> $m" }
function Ok($m)   { Write-Host "  [ok] $m" -ForegroundColor Green }
function Die($m)  { Write-Host "  [x] $m"  -ForegroundColor Red; exit 1 }

if (-not (Get-Command pnpm  -ErrorAction SilentlyContinue)) { Die "pnpm not on PATH (run .\setup.ps1)" }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Die "cargo not on PATH (run .\setup.ps1)" }

# -- kill anything running ----------------------------------------------------
Info "stopping running Cameo / dev server / codex sidecar"
Get-Process -Name 'cameo','Cameo' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process -Name 'codex'         -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
try {
  Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue |
    Select-Object -ExpandProperty OwningProcess -Unique |
    ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
} catch {}
Start-Sleep -Milliseconds 500

if (-not (Test-Path node_modules)) {
  Info "pnpm install (first run)"
  pnpm install
  if ($LASTEXITCODE -ne 0) { Die "pnpm install failed" }
}

if ($Clean) {
  Info "cargo clean + rm dist"
  Push-Location src-tauri; cargo clean; Pop-Location
  if (Test-Path dist) { Remove-Item -Recurse -Force dist }
}

$start = Get-Date

# -- build debug exe (no installer) ------------------------------------------
Info "pnpm tauri build --debug --no-bundle"
pnpm tauri build --debug --no-bundle
if ($LASTEXITCODE -ne 0) { Die "tauri build failed" }

$exe = Join-Path $PSScriptRoot 'src-tauri\target\debug\cameo.exe'
if (-not (Test-Path $exe)) { Die "exe not produced at $exe" }

$secs = [int]((Get-Date) - $start).TotalSeconds
Write-Host ""
Ok "built in ${secs}s"
Write-Host ""
Write-Host "  +- double-click to launch ----------------------------------"
Write-Host "     $exe"
Write-Host "  +-----------------------------------------------------------"
Write-Host ""

if ($Open) { Info "launching cameo.exe"; Start-Process $exe }
