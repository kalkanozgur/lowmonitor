# LowMonitor kurulum / güncelleme (spek §11). Yönetici PowerShell'de, repo kökünden:
#   cargo build --release --workspace
#   cargo build --profile release-ui -p lowmon-ui      # ui "s" profiliyle (Cargo.toml)
#   pwsh -File scripts\install.ps1 [-Src target\release] [-UiSrc target\release-ui]
# Veri (%ProgramData%\LowMonitor: key.bin, logs, config) korunur; uninstall --purge çağrılmaz.
param([string]$Src = "target\release", [string]$UiSrc = "target\release-ui")
$ErrorActionPreference = 'Stop'

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw "Yönetici PowerShell gerekli." }

$dst = Join-Path $env:ProgramFiles 'LowMonitor'
$exes = 'lowmon-svc.exe', 'lowmon-net.exe', 'lowmon-tray.exe', 'lowmon-hook.exe', 'lwm-dump.exe'
foreach ($e in $exes) { if (-not (Test-Path (Join-Path $Src $e))) { throw "$Src\$e yok; önce release build." } }
$ui = Join-Path $UiSrc 'lowmon-ui.exe'
if (-not (Test-Path $ui)) { throw "$ui yok; önce cargo build --profile release-ui -p lowmon-ui." }

# Kopyalanacak exe'leri kilitleyen kullanıcı süreçleri + eski LowDeck PC bileşenleri (ADR-0031: deck tray'de)
Get-Process lowmon-tray, lowmon-ui, lowmon-net, lowmon-hook, lowdeck-agent, lowdeck-desktop -ErrorAction SilentlyContinue | Stop-Process -Force

# Eski LowDeck izleri: agent autostart'ı, UiExe kaydı, agent firewall kuralı. agent.cfg kalır (deck.toml göçü okur).
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' LowDeckAgent -ErrorAction SilentlyContinue
Remove-ItemProperty 'HKCU:\Software\LowMonitor' UiExe -ErrorAction SilentlyContinue
netsh advfirewall firewall delete rule name="LowDeck agent" | Out-Null

# Eski C++ ai-usage-bar (ADR-0032 Faz 4): süreç + autostart kapanır, limit CSV'leri ve session.bin
# %LOCALAPPDATA%\LowMonitor\ai'ye taşınır, eski klasör silinir. config.ini taşınmaz (varsayılanlar aynı).
Get-Process claude-usage-bar -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' ClaudeUsageBar -ErrorAction SilentlyContinue
$oldAi = Join-Path $env:LOCALAPPDATA 'ClaudeUsageBar'
if (Test-Path $oldAi) {
    $newAi = Join-Path $env:LOCALAPPDATA 'LowMonitor\ai'
    New-Item -ItemType Directory -Force $newAi | Out-Null
    foreach ($f in Get-ChildItem $oldAi -Filter 'usage-*.csv') {
        $to = Join-Path $newAi $f.Name
        $rows = @(Get-Content $f.FullName | Select-Object -Skip 1)
        if (Test-Path $to) { $rows += @(Get-Content $to | Select-Object -Skip 1) }
        # Aynı ts iki dosyada da olabilir: ts'e göre tekilleştir + sırala
        $rows = $rows | Where-Object { $_ -match '^\d+,' } | Group-Object { $_.Split(',')[0] } |
            ForEach-Object { $_.Group[0] } | Sort-Object { [long]$_.Split(',')[0] }
        $text = (@('ts,session_pct,session_reset,week_pct,week_reset') + $rows) -join "`r`n"
        [IO.File]::WriteAllText($to, "$text`r`n")
    }
    $sb = Join-Path $oldAi 'session.bin'
    if ((Test-Path $sb) -and -not (Test-Path (Join-Path $newAi 'session.bin'))) { Copy-Item $sb $newAi }
    Remove-Item $oldAi -Recurse -Force
}

# Eski servis: durdur + sil (seal yazılır, veri kalır)
if (Get-Service LowMonitor -ErrorAction SilentlyContinue) {
    & (Join-Path $dst 'lowmon-svc.exe') uninstall
    if ($LASTEXITCODE -ne 0) { throw "uninstall başarısız ($LASTEXITCODE)" }
    # SCM silmeyi süreç kapanınca tamamlar; exe kilidi kalkana kadar bekle
    for ($i = 0; $i -lt 50 -and (Get-Process lowmon-svc -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 200 }
}

New-Item -ItemType Directory -Force $dst | Out-Null
foreach ($e in $exes) { Copy-Item (Join-Path $Src $e) $dst -Force }
Copy-Item $ui $dst -Force

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
