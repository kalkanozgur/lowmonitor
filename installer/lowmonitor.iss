; LowMonitor tek setup exe (ADR-0038). Derleme (repo kökünden, release build'den sonra):
;   ISCC /DMyVersion=1.1.0 installer\lowmonitor.iss   → target\setup\LowMonitor-Setup-1.1.0.exe
; scripts\install.ps1 ile aynı adımlar; %ProgramData%\LowMonitor verisi (key, log, config) hiçbir zaman silinmez.
#ifndef MyVersion
  #define MyVersion "0.0.0"
#endif

[Setup]
AppId={{6C0E5B9A-4F1D-4E43-9B3E-1A7D2C0B9F51}
AppName=LowMonitor
AppVersion={#MyVersion}
AppPublisher=kalkanozgur
DefaultDirName={autopf}\LowMonitor
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\setup
OutputBaseFilename=LowMonitor-Setup-{#MyVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Süreçleri PrepareToInstall kendisi kapatır.
CloseApplications=no
UninstallDisplayIcon={app}\lowmon-tray.exe

[Languages]
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
turkish.CTray=Tray + taskbar göstergeleri (zorunlu): bildirim simgesi, telefon sunucusu, CPU/RAM/AI göstergeleri
english.CTray=Tray + taskbar indicators (required): notification icon, phone server, CPU/RAM/AI indicators
turkish.CSvc=Servis (lowmon-svc): kaynak toplayıcı, şifreli log, alarmlar
english.CSvc=Service (lowmon-svc): resource collector, encrypted log, alarms
turkish.CUi=Arayüz (lowmon-ui): canlı, geçmiş, süreçler, ayarlar
english.CUi=Interface (lowmon-ui): live, history, processes, settings
turkish.CNet=LAN API (lowmon-net): ağdan canlı veri (WebSocket, eşleştirme)
english.CNet=LAN API (lowmon-net): live data over the network (WebSocket, pairing)
turkish.CHook=Claude Code hook (lowmon-hook): etkinlik balonu; kaydı tray menüsünden
english.CHook=Claude Code hook (lowmon-hook): activity bubble; register from the tray menu
turkish.CDump=lwm-dump: log dökümü / dışa aktarma aracı
english.CDump=lwm-dump: log dump / export tool
turkish.CFw=Telefon güvenlik duvarı kuralı (TCP 47810 + UDP 47811, yalnız özel ağ, yerel alt ağ)
english.CFw=Phone firewall rule (TCP 47810 + UDP 47811, private network, local subnet only)
turkish.CRunTray=LowMonitor tray'i başlat
english.CRunTray=Start LowMonitor tray

[Types]
Name: "full"; Description: "Full"
Name: "custom"; Description: "Custom"; Flags: iscustom

[Components]
Name: "tray"; Description: "{cm:CTray}"; Types: full custom; Flags: fixed
Name: "svc"; Description: "{cm:CSvc}"; Types: full
Name: "ui"; Description: "{cm:CUi}"; Types: full
Name: "net"; Description: "{cm:CNet}"; Types: full
Name: "hook"; Description: "{cm:CHook}"; Types: full
Name: "dump"; Description: "{cm:CDump}"; Types: full
Name: "phonefw"; Description: "{cm:CFw}"; Types: full

[Files]
Source: "..\target\release\lowmon-tray.exe"; DestDir: "{app}"; Components: tray; Flags: ignoreversion
Source: "..\target\release\lowmon-svc.exe"; DestDir: "{app}"; Components: svc; Flags: ignoreversion
Source: "..\target\release-ui\lowmon-ui.exe"; DestDir: "{app}"; Components: ui; Flags: ignoreversion
Source: "..\target\release\lowmon-net.exe"; DestDir: "{app}"; Components: net; Flags: ignoreversion
Source: "..\target\release\lowmon-hook.exe"; DestDir: "{app}"; Components: hook; Flags: ignoreversion
Source: "..\target\release\lwm-dump.exe"; DestDir: "{app}"; Components: dump; Flags: ignoreversion

[Run]
; Servis: kurar (DACL, LocalService, "LowMonitor net" firewall kuralı) + başlatır.
Filename: "{app}\lowmon-svc.exe"; Parameters: "install"; Components: svc; Flags: runhidden waituntilterminated
Filename: "{app}\lowmon-svc.exe"; Parameters: "start"; Components: svc; Flags: runhidden waituntilterminated
; Telefon (lowmon-deck tray içinde): tray menüsündeki "Firewall kuralı ekle" ile aynı. shortcut: port sabit 47810 (deck.toml'da değiştiyse menüden).
Filename: "netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LowMonitor deck"""; Components: phonefw; Flags: runhidden waituntilterminated
Filename: "netsh.exe"; Parameters: "advfirewall firewall add rule name=""LowMonitor deck"" dir=in action=allow protocol=TCP localport=47810 profile=private remoteip=localsubnet program=""{app}\lowmon-tray.exe"""; Components: phonefw; Flags: runhidden waituntilterminated
Filename: "netsh.exe"; Parameters: "advfirewall firewall add rule name=""LowMonitor deck"" dir=in action=allow protocol=UDP localport=47811 profile=private remoteip=localsubnet program=""{app}\lowmon-tray.exe"""; Components: phonefw; Flags: runhidden waituntilterminated
; HKCU kayıtları kurulumu başlatan (yükseltilmemiş) kullanıcıya yazılır.
Filename: "{app}\lowmon-tray.exe"; Parameters: "--autostart on"; Flags: runasoriginaluser runhidden waituntilterminated
Filename: "{app}\lowmon-tray.exe"; Description: "{cm:CRunTray}"; Flags: runasoriginaluser nowait postinstall

[UninstallRun]
Filename: "taskkill.exe"; Parameters: "/F /IM lowmon-tray.exe /IM lowmon-ui.exe /IM lowmon-net.exe"; Flags: runhidden waituntilterminated; RunOnceId: "kill"
Filename: "{app}\lowmon-tray.exe"; Parameters: "--autostart off"; Flags: runhidden waituntilterminated; RunOnceId: "autostart"
Filename: "{app}\lowmon-svc.exe"; Parameters: "uninstall"; Flags: runhidden waituntilterminated skipifdoesntexist; RunOnceId: "svc"
Filename: "netsh.exe"; Parameters: "advfirewall firewall delete rule name=""LowMonitor deck"""; Flags: runhidden waituntilterminated; RunOnceId: "fw"

[Code]
procedure Run(const Exe, Params: String);
var Code: Integer;
begin
  Exec(Exe, Params, '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

// Kopyalamadan önce: kilitleyen süreçler + eski servis (seal yazılır, veri kalır) + eski LowDeck / ai-usage-bar izleri.
// shortcut: install.ps1'deki ai-usage-bar CSV/session.bin taşıma yok (v1.0 kurulumları zaten taşıdı).
function PrepareToInstall(var NeedsRestart: Boolean): String;
var Code, I: Integer;
begin
  Result := '';
  Run('taskkill.exe', '/F /IM lowmon-tray.exe /IM lowmon-ui.exe /IM lowmon-net.exe /IM lowmon-hook.exe /IM lowdeck-agent.exe /IM lowdeck-desktop.exe /IM claude-usage-bar.exe');
  if FileExists(ExpandConstant('{app}\lowmon-svc.exe')) then
  begin
    Run(ExpandConstant('{app}\lowmon-svc.exe'), 'uninstall');
    // SCM silmeyi süreç kapanınca tamamlar; exe kilidi kalkana kadar bekle (≤ 10 s)
    for I := 1 to 50 do
    begin
      Exec('cmd.exe', '/c tasklist /FI "IMAGENAME eq lowmon-svc.exe" | find /I "lowmon-svc.exe" >nul', '', SW_HIDE, ewWaitUntilTerminated, Code);
      if Code <> 0 then Break;
      Sleep(200);
    end;
  end;
  ExecAsOriginalUser('reg.exe', 'delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v LowDeckAgent /f', '', SW_HIDE, ewWaitUntilTerminated, Code);
  ExecAsOriginalUser('reg.exe', 'delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v ClaudeUsageBar /f', '', SW_HIDE, ewWaitUntilTerminated, Code);
  ExecAsOriginalUser('reg.exe', 'delete HKCU\Software\LowMonitor /v UiExe /f', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Run('netsh.exe', 'advfirewall firewall delete rule name="LowDeck agent"');
end;
