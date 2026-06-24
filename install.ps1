# DPI-Bypass — Windows tek satır kurulum.
#
# Yönetici PowerShell'de:
#   irm https://raw.githubusercontent.com/ATOMGAMERAGA/DPI-Bypass/main/install.ps1 | iex
#
# En son GitHub sürümünü (windows .zip) indirir, Program Files'a kurar ve Başlat
# menüsü kısayolu oluşturur. Yönetici değilseniz kendini yükseltilmiş (UAC) bir
# pencerede yeniden başlatır — WinDivert sürücüsü yönetici hakkı gerektirir.
$ErrorActionPreference = "Stop"

$Repo     = "ATOMGAMERAGA/DPI-Bypass"
$OneLiner = "irm https://raw.githubusercontent.com/$Repo/main/install.ps1 | iex"

function Test-Admin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $p  = New-Object Security.Principal.WindowsPrincipal($id)
  return $p.IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)
}

# Self-elevate: re-run the same one-liner in an elevated PowerShell, then exit.
if (-not (Test-Admin)) {
  Write-Host "==> Yönetici hakkı isteniyor (UAC)…" -ForegroundColor Yellow
  Start-Process -FilePath "powershell.exe" -Verb RunAs -ArgumentList @(
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", $OneLiner
  )
  return
}

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$InstallDir = Join-Path $env:ProgramFiles "DPI-Bypass"

Write-Host "==> DPI-Bypass Windows kurulumu" -ForegroundColor Cyan

# Resolve the latest release's Windows zip asset.
Write-Host "    En son sürüm bulunuyor…"
$rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
  -Headers @{ "User-Agent" = "dpi-bypass-installer" }
$asset = $rel.assets | Where-Object { $_.name -like "*windows*x86_64.zip" } | Select-Object -First 1
if (-not $asset) { throw "Sürümde Windows .zip varlığı bulunamadı." }

$tmp = Join-Path $env:TEMP ("dpi-bypass-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$zip = Join-Path $tmp "bundle.zip"

Write-Host "    İndiriliyor: $($asset.name) ($($rel.tag_name))"
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip `
  -Headers @{ "User-Agent" = "dpi-bypass-installer" }
Expand-Archive -Path $zip -DestinationPath $tmp -Force

$payload = Get-ChildItem -Path $tmp -Recurse -Filter "dpi-bypass.exe" | Select-Object -First 1
if (-not $payload) { throw "Arşivde dpi-bypass.exe bulunamadı." }
$src = $payload.Directory.FullName

# Stop a running instance so files aren't locked, then install.
Get-Process -Name "dpi-bypass", "dpi-bypass-helper", "goodbyedpi" -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Get-ChildItem -Path $src -Include *.exe, WinDivert*, *.svg, *.sys, *.dll -Recurse -File |
  ForEach-Object { Copy-Item $_.FullName (Join-Path $InstallDir $_.Name) -Force }

# Start Menu shortcut (the GUI requests elevation itself).
$startMenu = Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs"
$lnk = Join-Path $startMenu "DPI-Bypass.lnk"
$ws = New-Object -ComObject WScript.Shell
$sc = $ws.CreateShortcut($lnk)
$sc.TargetPath = Join-Path $InstallDir "dpi-bypass.exe"
$sc.WorkingDirectory = $InstallDir
$sc.IconLocation = Join-Path $InstallDir "dpi-bypass.exe"
$sc.Description = "Domain bazlı DPI atlatma"
$sc.Save()

Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue

Write-Host "==> Kuruldu: $InstallDir" -ForegroundColor Green
Write-Host "    Başlat menüsünden 'DPI-Bypass' ile açın."
