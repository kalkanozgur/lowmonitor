# LowMonitor
Very low-overhead resource monitoring, logging and intervention tool for Windows (Rust, `x86_64-pc-windows-msvc`), with a desktop UI, taskbar meters, Claude usage tracking and a phone companion ([LowDeck](https://github.com/kalkanozgur/lowdeck)).
Spec: [docs/spesifikasyon.md](docs/spesifikasyon.md) (Turkish, single source of truth)

Türkçe: [README.tr.md](README.tr.md)

| Field | Value |
|---|---|
| Status | Prototype: M0/M3 done, M1–M6 code largely done; final acceptance soak pending |
| Modules | software (Rust workspace) |
| Started | 2026-11-14 |
| Budget | CPU < 0.05 % total, Private WS / Bytes < 3 MB (24 h) |

## Components
| Process / crate | Role |
|---|---|
| `lowmon-svc` | Windows service (SYSTEM): CPU / RAM / disk / network / Top-N collector, encrypted + hash-chained log, retention, alarms, named pipe `\\.\pipe\LowMonitor` |
| `lowmon-tray` | User session: tray icon, taskbar meters, Claude usage, phone server (`lowmon-deck`), sensor thread |
| `lowmon-ui` | Desktop UI (Slint): Live, Events, History, Processes, AI, Settings; data from the tray over loopback |
| `lowmon-net` | Optional LAN HTTP / WebSocket API (`/v1/info`, live JSON), token auth |
| `lowmon-deck` | Phone / UI server inside the tray: TCP 47810 + UDP 47811 discovery, audio / media / lock, processes, history, AI report |
| `lowmon-taskbar` | Two-block meter embedded in the taskbar (system grid + AI), per monitor |
| `lowmon-ai` | Claude subscription usage (5 h session + week), OAuth / sessionKey, CSV history, Claude Code activity |
| `lowmon-sensors` | GPU % / °C (D3DKMT), disk °C, CPU °C (ACPI thermal zone) |
| `lowmon-core`, `lowmon-proto` | Record format, crypto, config; pipe protocol and metric types |
| `tools/lwm-dump`, `tools/lowmon-hook` | Log export / inspection; Claude Code hook command |

## Features
- Live and historical graphs (CPU, RAM, disk, network, GPU, temperatures), Top-N processes, process kill
- Alarm rules → events, tray balloons, phone alarm band; optional UAC / firewall prompt alarms (`[events] uac = true`, ETW, notification only); tray temperature alarms (`Temp.CPU/GPU/Disk`) from `taskbar.toml` thresholds
- Taskbar: system grid (CPU / GPU / MEM / ↑, °C / ↓) and Claude usage meter; each can be toggled from the tray menu
- Claude Code activity bubble over the AI meter (hooks installed from the tray menu)
- Claude Code activity bubble: project, safe summary (Bash description, file name), `Claude.Input` alarm when Claude waits for you
- Phone (LowDeck): discovery, auto-reconnect when the PC IP changes, screen blackout that wakes on alarms
- Tray follows IP changes (DHCP) and re-binds the phone listener automatically
- English / Turkish UI (`ai.toml` `language`)

## Folder structure
```
lowmonitor/
├─ Cargo.toml              # workspace, shared dependency versions
├─ crates/                 # lowmon-core, -proto, -svc, -tray, -ui, -net, -deck, -taskbar, -ai, -sensors
├─ tools/                  # lwm-dump, lowmon-hook
├─ scripts/                # install.ps1, soak.ps1
├─ .githooks/pre-push      # local quality gate
├─ .github/workflows/      # release build on v* tags only
└─ docs/
   ├─ spesifikasyon.md     # single source of truth
   ├─ gereksinimler.md
   ├─ devlog/YYYY-MM.md
   ├─ kararlar/ADR-*.md    # decisions (latest: ADR-0035)
   ├─ olcumler/            # performance measurements
   └─ referans/
```

## Quick start
Build (repo root):
```powershell
cargo build --release --workspace
cargo build --profile release-ui -p lowmon-ui
```
Install / update from an **administrator PowerShell** (copies to `C:\Program Files\LowMonitor`, installs the service, firewall rules, tray autostart; data in `%ProgramData%\LowMonitor` is kept):
```powershell
pwsh -File scripts\install.ps1 -Src target\release -UiSrc target\release-ui
```
Install: download `LowMonitor-Setup-<version>.exe` from GitHub Releases (components selectable; tray + taskbar required). Developer path: `scripts\install.ps1`.

User settings: `%APPDATA%\LowMonitor\` (`deck.toml`, `taskbar.toml`, `ai.toml`, `net.toml`); service settings: `%ProgramData%\LowMonitor\config.toml`.

## Development flow
| Stage | Where | What |
|---|---|---|
| Build + test | Local | `cargo build`, `cargo test --workspace` |
| Before push | Local hook | `fmt --check` + `clippy -D warnings` + `test` |
| Change | GitHub | branch → PR → squash merge (ADR-0021); no direct push to main |
| Release | GitHub Actions | `git tag vX.Y.Z && git push --tags` → release build, exes attached to the GitHub Release |

Enable the hook once after cloning:
```powershell
git config core.hooksPath .githooks
```

## AI agent handoff
See `HANDOFF.md` for current state and next steps. Rules live in `CLAUDE.md`.

## License
MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

`lowmon-ui` is built with [Slint](https://slint.dev) ([Royalty-free 2.0](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md) license; attribution required):

<a href="https://slint.dev"><img alt="Made with Slint" src="https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg" height="60"></a>
