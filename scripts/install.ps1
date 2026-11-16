# LowMonitor kurulum / güncelleme (spek §11). Yönetici PowerShell'de, repo kökünden:
#   cargo build --release --workspace
#   pwsh -File scripts\install.ps1 [-Src target\release]
# Veri (%ProgramData%\LowMonitor: key.bin, logs, config) korunur; uninstall --purge çağrılmaz.
param([string]$Src = "target\release")
$ErrorActionPreference = 'Stop'

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw "Yönetici PowerShell gerekli." }

$dst = Join-Path $env:ProgramFiles 'LowMonitor'
$exes = 'lowmon-svc.exe', 'lowmon-net.exe', 'lowmon-tray.exe', 'lowmon-ui.exe', 'lwm-dump.exe'
foreach ($e in $exes) { if (-not (Test-Path (Join-Path $Src $e))) { throw "$Src\$e yok; önce release build." } }

# Kopyalanacak exe'leri kilitleyen kullanıcı süreçleri + eski LowDeck PC bileşenleri (ADR-0031: deck tray'de)
Get-Process lowmon-tray, lowmon-ui, lowmon-net, lowdeck-agent, lowdeck-desktop -ErrorAction SilentlyContinue | Stop-Process -Force

# Eski LowDeck izleri: agent autostart'ı, UiExe kaydı, agent firewall kuralı. agent.cfg kalır (deck.toml göçü okur).
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' LowDeckAgent -ErrorAction SilentlyContinue
Remove-ItemProperty 'HKCU:\Software\LowMonitor' UiExe -ErrorAction SilentlyContinue
netsh advfirewall firewall delete rule name="LowDeck agent" | Out-Null

# Eski servis: durdur + sil (seal yazılır, veri kalır)
if (Get-Service LowMonitor -ErrorAction SilentlyContinue) {
    & (Join-Path $dst 'lowmon-svc.exe') uninstall
    if ($LASTEXITCODE -ne 0) { throw "uninstall başarısız ($LASTEXITCODE)" }
    # SCM silmeyi süreç kapanınca tamamlar; exe kilidi kalkana kadar bekle
    for ($i = 0; $i -lt 50 -and (Get-Process lowmon-svc -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 200 }
}

New-Item -ItemType Directory -Force $dst | Out-Null
foreach ($e in $exes) { Copy-Item (Join-Path $Src $e) $dst -Force }

$svc = Join-Path $dst 'lowmon-svc.exe'
& $svc install; if ($LASTEXITCODE -ne 0) { throw "install başarısız ($LASTEXITCODE)" }
& $svc start; if ($LASTEXITCODE -ne 0) { throw "start başarısız ($LASTEXITCODE)" }

# Tray kullanıcı yetkisiyle çalışmalı: explorer üzerinden başlatınca yükseltilmemiş açılır.
# shortcut: farklı bir admin hesabıyla yükseltildiyse HKCU o hesabınki olur; o durumda autostart'ı normal kullanıcıyla ayrıca aç.
$tray = Join-Path $dst 'lowmon-tray.exe'

# Telefon sunucusu (lowmon-deck) firewall kuralı: tray menüsündeki "Firewall kuralı ekle" ile aynı.
# Port deck.toml'dan; dosya yoksa (ilk kurulum) varsayılan 47810.
$port = 47810
$deckToml = Join-Path $env:APPDATA 'LowMonitor\deck.toml'
if ((Test-Path $deckToml) -and ((Get-Content $deckToml -Raw) -match '(?m)^\s*port\s*=\s*(\d+)')) { $port = [int]$Matches[1] }
netsh advfirewall firewall delete rule name="LowMonitor deck" | Out-Null
netsh advfirewall firewall add rule name="LowMonitor deck" dir=in action=allow protocol=TCP localport=$port profile=private remoteip=localsubnet program="$tray" | Out-Null
netsh advfirewall firewall add rule name="LowMonitor deck" dir=in action=allow protocol=UDP localport=47811 profile=private remoteip=localsubnet program="$tray" | Out-Null

& $tray --autostart on | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Warning "tray autostart kaydı başarısız ($LASTEXITCODE)" }
Start-Process explorer.exe $tray

Get-Service LowMonitor | Format-Table Name, Status, StartType -AutoSize
netsh advfirewall firewall show rule name="LowMonitor net" | Select-String 'Enabled|Profiles|LocalPort|RemoteIP|Program'
netsh advfirewall firewall show rule name="LowMonitor deck" | Select-String 'Protocol|LocalPort|Program'
