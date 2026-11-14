# lowmon-svc soak ölçümü (spek §12 Performans). Admin olarak çalıştır: servis sürecinin CPU zamanını okumak için.
# Kullanım: pwsh -File scripts\soak.ps1 -Out docs\olcumler\YYYY-MM-DD_soak.csv [-Hours 24] [-IntervalSec 300]
param(
    [Parameter(Mandatory)][string]$Out,
    [double]$Hours = 24,
    [int]$IntervalSec = 300
)
"ts,cpu_s,priv_ws_kb,private_kb,ws_kb,handles,threads" | Set-Content -Encoding utf8 $Out
$end = (Get-Date).AddHours($Hours)
while ((Get-Date) -lt $end) {
    $p = Get-Process lowmon-svc -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($p) {
        $pws = (Get-Counter "\Process(lowmon-svc)\Working Set - Private" -ErrorAction SilentlyContinue).CounterSamples[0].CookedValue
        # Invariant: tr-TR ondalık virgülü CSV sütunlarını kaydırır
        [string]::Format([cultureinfo]::InvariantCulture, "{0},{1:F3},{2},{3},{4},{5},{6}", (Get-Date -Format s), $p.TotalProcessorTime.TotalSeconds,
            [int]($pws / 1KB), [int]($p.PrivateMemorySize64 / 1KB), [int]($p.WorkingSet64 / 1KB),
            $p.HandleCount, $p.Threads.Count) | Add-Content $Out
    } else {
        "$(Get-Date -Format s),NA,,,,," | Add-Content $Out  # servis çalışmıyor → çökme/yeniden başlatma izi
    }
    Start-Sleep $IntervalSec
}
