# setup.ps1 — one-time dev environment setup for Cameo on Windows (x64).
#
# Checks the toolchain (Rust + MSVC + Node/pnpm + WebView2), adds the Rust
# target we ship for, and pulls JS + Cargo deps. Re-runnable any time.
#
# Usage (PowerShell):
#   .\setup.ps1
#
# macOS: use setup.sh instead.

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

function Info($m) { Write-Host "-> $m" }
function Ok($m)   { Write-Host "  [ok] $m"   -ForegroundColor Green }
function Warn($m) { Write-Host "  [!] $m"    -ForegroundColor Yellow }
function Die($m)  { Write-Host "  [x] $m"    -ForegroundColor Red; exit 1 }

function Have($cmd) { return [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

# -- toolchain ---------------------------------------------------------------
Info "checking toolchain"
if (-not (Have node))  { Die "Node.js not found - install Node 20+ (winget install OpenJS.NodeJS.LTS)" }
if (-not (Have pnpm))  { Die "pnpm not found - 'npm i -g pnpm' (or 'winget install pnpm.pnpm')" }
if (-not (Have rustc)) { Die "Rust not found - install via https://rustup.rs (choose the MSVC toolchain)" }
if (-not (Have cargo)) { Die "cargo not found - install Rust via https://rustup.rs" }
Ok ("node {0}  .  pnpm {1}  .  {2}" -f (node -v), (pnpm -v), (rustc --version))

# -- MSVC build tools (Tauri/Rust link with the MSVC toolchain) --------------
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $vswhere) {
  $vcPath = & $vswhere -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationPath 2>$null
  if ($vcPath) { Ok "Visual Studio C++ build tools: $vcPath" }
  else { Warn "VS found but the 'Desktop development with C++' (VC++) workload is missing - install it via the VS Installer" }
} else {
  Warn "Visual Studio Build Tools not detected. Install 'Build Tools for Visual Studio' with the C++ workload:"
  Warn "    winget install Microsoft.VisualStudio.2022.BuildTools --override `"--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended`""
}

# -- WebView2 runtime (Win11 ships it; Win10 may not) ------------------------
$wv2Key = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$wv2KeyUser = 'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
if ((Test-Path $wv2Key) -or (Test-Path $wv2KeyUser)) {
  Ok "WebView2 runtime present"
} else {
  Warn "WebView2 runtime not detected. The release installer downloads it automatically,"
  Warn "but for dev builds install it: https://developer.microsoft.com/microsoft-edge/webview2/"
}

# -- Rust target -------------------------------------------------------------
Info "ensuring Rust target x86_64-pc-windows-msvc"
$installed = (rustup target list --installed 2>$null)
if ($installed -contains 'x86_64-pc-windows-msvc') { Ok "x86_64-pc-windows-msvc" }
else { Info "adding x86_64-pc-windows-msvc"; rustup target add x86_64-pc-windows-msvc }

# -- dependencies ------------------------------------------------------------
Info "pnpm install"
pnpm install
Ok "JS dependencies installed"

Info "warming the Cargo cache (cargo fetch)"
Push-Location src-tauri; cargo fetch | Out-Null; Pop-Location
Ok "Cargo dependencies fetched"

# -- Codex CLI (not bundled - Cameo drives the user's own, authenticated copy)
if (Have codex) {
  Ok ("codex found: {0}" -f (Get-Command codex).Source)
} else {
  Warn "codex CLI not on PATH. Cameo needs it at runtime (it is NOT bundled):"
  Warn "    npm i -g @openai/codex"
  Warn "    codex login              # ChatGPT subscription auth (no API key)"
}

Write-Host ""
Ok "setup complete"
Write-Host ""
Write-Host "  next:"
Write-Host "    pnpm tauri dev          # live dev with hot reload"
Write-Host "    .\build_dev.ps1         # build an unsigned debug cameo.exe"
Write-Host "    .\build_release.ps1     # build the NSIS release installer"
Write-Host ""
