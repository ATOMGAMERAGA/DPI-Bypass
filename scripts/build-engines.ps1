# Fetch the Windows DPI engine (GoodbyeDPI + WinDivert) bundled with DPI-Bypass.
#
# Mirrors scripts/build-engines.sh for Windows. Output goes to
# src-tauri\binaries\ so the installer / release packaging can pick it up.
#
#   pwsh scripts/build-engines.ps1
$ErrorActionPreference = "Stop"

$Root  = Split-Path -Parent $PSScriptRoot
$Out   = Join-Path $Root "src-tauri\binaries"
$Build = Join-Path $Root "build"
New-Item -ItemType Directory -Force -Path $Out, $Build | Out-Null

$Ver = if ($env:GOODBYEDPI_VER) { $env:GOODBYEDPI_VER } else { "0.2.3rc3" }
# The release asset name does not follow a clean pattern (e.g.
# goodbyedpi-0.2.3rc3-2.zip), so default to the verified direct URL and allow an
# override via $env:GOODBYEDPI_URL.
$Url = if ($env:GOODBYEDPI_URL) { $env:GOODBYEDPI_URL } else {
  "https://github.com/ValdikSS/GoodbyeDPI/releases/download/0.2.3rc3/goodbyedpi-0.2.3rc3-2.zip"
}
$Zip = Join-Path $Build "goodbyedpi.zip"
$Ext = Join-Path $Build "goodbyedpi"

Write-Host "==> Fetching GoodbyeDPI $Ver" -ForegroundColor Cyan
Invoke-WebRequest -Uri $Url -OutFile $Zip
if (Test-Path $Ext) { Remove-Item -Recurse -Force $Ext }
Expand-Archive -Path $Zip -DestinationPath $Ext -Force

# The release ships x86 and x86_64 subfolders; prefer 64-bit.
$exe = Get-ChildItem -Path $Ext -Recurse -Filter "goodbyedpi.exe" |
       Where-Object { $_.FullName -match "x86_64" } | Select-Object -First 1
if (-not $exe) {
  $exe = Get-ChildItem -Path $Ext -Recurse -Filter "goodbyedpi.exe" | Select-Object -First 1
}
if (-not $exe) { throw "goodbyedpi.exe not found in archive" }

$engineDir = $exe.Directory.FullName
Copy-Item $exe.FullName (Join-Path $Out "goodbyedpi.exe") -Force
# WinDivert driver + dll must sit next to the exe.
Get-ChildItem -Path $engineDir -Filter "WinDivert*" | ForEach-Object {
  Copy-Item $_.FullName (Join-Path $Out $_.Name) -Force
}

Write-Host "Engines ready in $Out" -ForegroundColor Green
Get-ChildItem $Out | Select-Object Name, Length | Format-Table
