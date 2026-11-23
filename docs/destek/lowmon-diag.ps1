# LowMonitor tanılama — yalnız OKUR, hiçbir şeyi değiştirmez. Çıktı: Masaüstü\lowmon-diag-<tarih>.txt
# Çalıştır: powershell -ExecutionPolicy Bypass -File .\lowmon-diag.ps1
$out = Join-Path ([Environment]::GetFolderPath('Desktop')) ("lowmon-diag-{0}.txt" -f (Get-Date -Format 'yyyy-MM-dd_HHmm'))
function Section($t, [scriptblock]$b) {
    "`n=== $t ===" | Out-File $out -Append -Encoding utf8
    try { & $b 2>&1 | Out-String -Width 220 | Out-File $out -Append -Encoding utf8 }
    catch { "HATA: $_" | Out-File $out -Append -Encoding utf8 }
}
"LowMonitor diag $(Get-Date -Format s)" | Out-File $out -Encoding utf8

Section 'Windows' { Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' | Select ProductName,DisplayVersion,CurrentBuild }
Section 'Taskbar ayarlari (TaskbarAl 0=sol, TaskbarDa bos=widget acik)' {
    Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced' | Select TaskbarAl,TaskbarDa,TaskbarSi
}
Section 'Monitorler' {
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.Screen]::AllScreens | Select DeviceName,Primary,Bounds
}
Section 'Taskbar ogeleri (UIA)' {
    Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
    Add-Type 'using System;using System.Runtime.InteropServices;public static class W{[DllImport("user32.dll")]public static extern IntPtr FindWindow(string c,string n);}'
    [System.Windows.Automation.AutomationElement]::FromHandle([W]::FindWindow("Shell_TrayWnd",$null)).FindAll(4,[System.Windows.Automation.Condition]::TrueCondition) |
        % { "{0} | {1} | {2} | {3}" -f $_.Current.AutomationId,$_.Current.ClassName,($_.Current.Name -replace "\s+"," "),$_.Current.BoundingRectangle }
}
Section 'Servis ve surecler' {
    Get-Service LowMonitor
    Get-Process lowmon* | Select Name,Id,StartTime,@{n='MB';e={[int]($_.PrivateMemorySize64/1MB)}}
}
Section 'CPU sicaklik sayaci' { Get-Counter '\Thermal Zone Information(*)\Temperature' }
Section 'GPU' { Get-CimInstance Win32_VideoController | Select Name,DriverVersion }
Section 'Diskler' { Get-PhysicalDisk | Select FriendlyName,BusType,MediaType }
Section 'taskbar.toml' { Get-Content (Join-Path $env:APPDATA 'LowMonitor\taskbar.toml') }
Section 'Log dosyalari (son 80 satir; gondermeden once goz at)' {
    foreach ($d in $env:APPDATA,$env:LOCALAPPDATA,$env:ProgramData) {
        Get-ChildItem (Join-Path $d 'LowMonitor') -Filter *.log -ErrorAction SilentlyContinue | % {
            "--- $($_.FullName) ($($_.Length) bayt)"; Get-Content $_.FullName -Tail 80
        }
    }
}
Section 'Uygulama hatalari (son 24 saat, LowMonitor/lowmon)' {
    Get-WinEvent -FilterHashtable @{LogName='Application'; StartTime=(Get-Date).AddDays(-1)} -ErrorAction SilentlyContinue |
        ? { $_.Message -match 'lowmon|LowMonitor' } | Select -First 15 TimeCreated,ProviderName,Id,@{n='Msg';e={$_.Message.Split("`n")[0]}}
}
"Yazildi: $out"
