# DPI-Bypass — Windows installer.
#
# Installs the extracted release bundle (this script sits next to dpi-bypass.exe,
# dpi-bypass-helper.exe and goodbyedpi.exe + WinDivert) into Program Files,
# creates a Start Menu shortcut, and registers the application. The GUI itself
# elevates (requireAdministrator) and manages the "Always On" scheduled task.
#
# Run from an elevated PowerShell, in the extracted bundle folder:
#   .\install.ps1
$ErrorActionPreference = "Stop"

function Assert-Admin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $p  = New-Object Security.Principal.WindowsPrincipal($id)
  if (-not $p.IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)) {
    throw "Lütfen bu kurulumu yönetici (Administrator) PowerShell'den çalıştırın."
  }
}

Assert-Admin

$Src = $PSScriptRoot
$InstallDir = Join-Path $env:ProgramFiles "DPI-Bypass"

Write-Host "==> DPI-Bypass Windows kurulumu" -ForegroundColor Cyan
Write-Host "    Hedef: $InstallDir"

# Required payload — fail early with a clear message if the bundle is incomplete.
$required = @("dpi-bypass.exe", "dpi-bypass-helper.exe", "goodbyedpi.exe")
foreach ($f in $required) {
  if (-not (Test-Path (Join-Path $Src $f))) {
    throw "Eksik dosya: $f — bu betiği çıkarılmış release arşivinin içinden çalıştırın."
  }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Copy the GUI, helper, engine and its WinDivert driver/dll.
Get-ChildItem -Path $Src -Include *.exe, WinDivert*, *.svg -Recurse -File |
  ForEach-Object { Copy-Item $_.FullName (Join-Path $InstallDir $_.Name) -Force }

# Start Menu shortcut (runs the GUI; the GUI requests elevation itself).
$startMenu = Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs"
$lnk = Join-Path $startMenu "DPI-Bypass.lnk"
$ws = New-Object -ComObject WScript.Shell
$sc = $ws.CreateShortcut($lnk)
$sc.TargetPath = Join-Path $InstallDir "dpi-bypass.exe"
$sc.WorkingDirectory = $InstallDir
$sc.IconLocation = Join-Path $InstallDir "dpi-bypass.exe"
$sc.Description = "Domain bazlı DPI atlatma"
$sc.Save()

Write-Host "==> Kuruldu." -ForegroundColor Green
Write-Host "    Başlat menüsünden 'DPI-Bypass' ile açın."
Write-Host "    'Her Zaman Açık' seçeneği uygulama içinden bir zamanlanmış görev olarak ayarlanır."
