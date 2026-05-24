#!/usr/bin/env pwsh
# publish_release.ps1 — sign + upload the Windows auto-update release to R2.
#
# Windows half of the release. macOS publishes independently from a Mac via
# publish_release.sh — the two write DIFFERENT manifest files on R2
# (windows-x86_64.json here, darwin-*.json there), so independent, async uploads
# never clobber each other.
#
# Prerequisite:  .\build_release.ps1           (NSIS installer + updater payload)
#
# Usage:
#   .\publish_release.ps1                       # sign + upload windows artifacts
#   .\publish_release.ps1 -DryRun               # print what would happen, upload nothing
#
# What it does:
#   1. Reads version from src-tauri/tauri.conf.json
#   2. Locates the auto-update payload (.nsis.zip), reads/creates its .sig
#      (signed at build time via TAURI_SIGNING_PRIVATE_KEY), writes a manifest
#      JSON ({version, notes, pub_date, signature, url}).
#   3. rclone uploads to R2:
#      - .nsis.zip + .sig + -setup.exe -> release/v<version>/
#      - manifest -> update/windows-x86_64.json
#
# R2 credentials come from .env (R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY /
# R2_ENDPOINT / R2_BUCKET). The same .env that hosts TAURI_SIGNING_PRIVATE_KEY
# also hosts these.

param([switch]$DryRun)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

function Info($m) { Write-Host "-> $m" }
function Ok($m)   { Write-Host "  [ok] $m" -ForegroundColor Green }
function Warn($m) { Write-Host "  [!] $m"  -ForegroundColor Yellow }
function Die($m)  { Write-Host "  [x] $m"  -ForegroundColor Red; exit 1 }

$Target = 'x86_64-pc-windows-msvc'

# -- load .env (KEY=VALUE per line) ------------------------------------------
function Import-DotEnv($path) {
  if (-not (Test-Path $path)) { Die ".env not found - needed for R2 creds and signing key" }
  Info "loading .env"
  Get-Content $path | ForEach-Object {
    $line = $_.Trim()
    if ($line -eq '' -or $line.StartsWith('#')) { return }
    $idx = $line.IndexOf('=')
    if ($idx -lt 1) { return }
    $k = $line.Substring(0, $idx).Trim()
    $v = $line.Substring($idx + 1).Trim().Trim('"').Trim("'")
    [Environment]::SetEnvironmentVariable($k, $v, 'Process')
  }
}
Import-DotEnv (Join-Path $PSScriptRoot '.env')

foreach ($v in 'R2_ACCESS_KEY_ID', 'R2_SECRET_ACCESS_KEY', 'R2_ENDPOINT', 'R2_BUCKET') {
  if (-not [Environment]::GetEnvironmentVariable($v)) { Die "$v missing from .env" }
}
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  Warn "TAURI_SIGNING_PRIVATE_KEY not set - manifest will reference an unsigned payload (clients will refuse it)"
}
if (-not (Get-Command rclone -ErrorAction SilentlyContinue)) {
  Die "rclone not on PATH - install from https://rclone.org/downloads/"
}

# -- version -----------------------------------------------------------------
$Version = (Get-Content (Join-Path $PSScriptRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
if (-not $Version) { Die "could not read version from tauri.conf.json" }
Ok "version: $Version"
$PubDate = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$Notes = "Cameo v$Version"
$DownloadBase = "https://r.cameo.ink/release/v$Version"

# -- locate artifacts --------------------------------------------------------
$nsisDir = Join-Path $PSScriptRoot "src-tauri\target\$Target\release\bundle\nsis"
if (-not (Test-Path $nsisDir)) { Die "no Windows bundle at $nsisDir - run .\build_release.ps1 first" }

# Manifests are written into a temp dir then uploaded.
$manifestDir = Join-Path ([System.IO.Path]::GetTempPath()) ("cameo-pub-" + [System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $manifestDir -Force | Out-Null

# Track files we'll upload as { Src; Dst } records.
$uploads = New-Object System.Collections.ArrayList
function Queue($src, $dst) { [void]$uploads.Add([pscustomobject]@{ Src = $src; Dst = $dst }) }

# Sign a payload if .sig is missing. Best-effort - returns $null on failure.
function Ensure-Sig($payload) {
  $sig = "$payload.sig"
  if (Test-Path $sig) { return $sig }
  if (-not $env:TAURI_SIGNING_PRIVATE_KEY) { return $null }
  Info "  signing $(Split-Path $payload -Leaf)"
  $keyfile = New-TemporaryFile
  Set-Content -Path $keyfile.FullName -Value $env:TAURI_SIGNING_PRIVATE_KEY -NoNewline
  try {
    if ($env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
      pnpm tauri signer sign -k $keyfile.FullName -p $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD $payload | Out-Null
    } else {
      pnpm tauri signer sign -k $keyfile.FullName $payload | Out-Null
    }
  } finally { Remove-Item $keyfile.FullName -ErrorAction SilentlyContinue }
  if (Test-Path $sig) { return $sig } else { return $null }
}

function Write-Manifest($name, $payloadFilename, $sigFile) {
  $signature = ''
  if ($sigFile -and (Test-Path $sigFile)) { $signature = (Get-Content $sigFile -Raw).Trim() }
  $out = Join-Path $manifestDir "$name.json"
  $json = @"
{
  "version": "$Version",
  "notes": "$Notes",
  "pub_date": "$PubDate",
  "signature": "$signature",
  "url": "$DownloadBase/$payloadFilename"
}
"@
  # UTF-8 WITHOUT BOM - a BOM breaks serde_json parsing on the client.
  [System.IO.File]::WriteAllText($out, $json, (New-Object System.Text.UTF8Encoding $false))
  Ok "manifest: $name.json"
  if (-not $signature) { Warn "  signature empty - clients will REJECT this update" }
  Queue $out "update/$name.json"
}

# -- Windows scan (x64) ------------------------------------------------------
$zip = Get-ChildItem -Path $nsisDir -Filter '*.nsis.zip' -ErrorAction SilentlyContinue | Select-Object -First 1
$exe = Get-ChildItem -Path $nsisDir -Filter '*-setup.exe' -ErrorAction SilentlyContinue | Select-Object -First 1

if (-not $zip) {
  Warn "no .nsis.zip updater payload in $nsisDir - installer-only release (no auto-update)"
} else {
  $sig = Ensure-Sig $zip.FullName
  $zipName = $zip.Name
  Queue $zip.FullName "release/v$Version/$zipName"
  if ($sig) { Queue $sig "release/v$Version/$zipName.sig" }
  Write-Manifest "windows-x86_64" $zipName $sig
}
if ($exe) {
  Queue $exe.FullName "release/v$Version/$($exe.Name)"
  Ok "  installer: $($exe.Name)"
}

# -- upload ------------------------------------------------------------------
if ($uploads.Count -eq 0) { Die "no artifacts to upload - did you run .\build_release.ps1 first?" }

Write-Host ""
Info "preparing to upload $($uploads.Count) file(s) to R2 bucket '$($env:R2_BUCKET)':"
foreach ($u in $uploads) {
  Write-Host "    $(Split-Path $u.Src -Leaf)"
  Write-Host "         -> r2:$($env:R2_BUCKET)/$($u.Dst)"
}
Write-Host ""

if ($DryRun) { Ok "DRY RUN - no upload performed."; exit 0 }

$yn = Read-Host "Proceed with upload? [y/N]"
if ($yn -notmatch '^[Yy]$') { Warn "aborted by user"; exit 0 }

# Configure rclone's ad-hoc `:s3:` backend via env vars. Do NOT use an inline
# connection string (`:s3,endpoint=…:`) — R2_ENDPOINT contains "://" and rclone
# truncates an unquoted value at the first colon, yielding endpoint="https".
$env:RCLONE_S3_PROVIDER          = "Cloudflare"
$env:RCLONE_S3_ACCESS_KEY_ID     = $env:R2_ACCESS_KEY_ID
$env:RCLONE_S3_SECRET_ACCESS_KEY = $env:R2_SECRET_ACCESS_KEY
$env:RCLONE_S3_ENDPOINT          = $env:R2_ENDPOINT
$env:RCLONE_S3_REGION            = "auto"
# R2 tokens can't create buckets; skip rclone's pre-upload bucket check/create
# (the bucket already exists) - otherwise the CreateBucket probe 403s.
$env:RCLONE_S3_NO_CHECK_BUCKET   = "true"
$emptyConf = New-TemporaryFile
try {
  foreach ($u in $uploads) {
    $remote = ":s3:$($env:R2_BUCKET)/$($u.Dst)"
    Info "rclone copyto $(Split-Path $u.Src -Leaf) -> r2:$($env:R2_BUCKET)/$($u.Dst)"
    & rclone --config $emptyConf.FullName copyto $u.Src $remote
    if ($LASTEXITCODE -ne 0) { Die "rclone upload failed for $($u.Src)" }
  }
} finally { Remove-Item $emptyConf.FullName -ErrorAction SilentlyContinue }

Write-Host ""
Ok "Windows release v$Version published"
Write-Host ""
Write-Host "  +- live manifest URL ------------------------------------------"
foreach ($u in $uploads) {
  if ($u.Dst -like 'update/*') { Write-Host "     https://r.cameo.ink/$($u.Dst)" }
}
Write-Host "  +--------------------------------------------------------------"
Write-Host ""
Warn "REMINDER: bump src-tauri/tauri.conf.json's version BEFORE running this"
Warn "script for the next release, or you'll re-publish v$Version."
Warn "macOS publishes separately - run ./publish_release.sh on the Mac."
