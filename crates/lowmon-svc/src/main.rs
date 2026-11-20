//! lowmon-svc — servis + install/start/stop/uninstall (spek §11), `run` = konsol modu (geliştirme).
mod alarm;
mod collect;
mod export;
mod firewall;
mod keystore;
mod pipe;
mod query;
mod retention;
mod security;
mod ticker;
mod uac;
mod writer;

use alarm::{Engine, Transition};
use collect::Collector;
use lowmon_core::{
    config::{Config, DEFAULT_TOML},
    metrics::{EventKind, Sample, TopEntry},
    proto::{CH_EVENT, CH_SAMPLE, CH_TOP, Control, ErrorCode, PIPE_NAME, ServerMsg},
};
use pipe::{Cmd, CmdReq, Publisher, Server};
use retention::Scheduler;
use std::{
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        mpsc::{Receiver, sync_channel},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use ticker::{StopHandle, Ticker};
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceSidType, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows_sys::Win32::System::{
    Console::SetConsoleCtrlHandler,
    Services::{
        ChangeServiceConfig2W, SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
        SERVICE_REQUIRED_PRIVILEGES_INFOW,
    },
    SystemInformation::GetTickCount64,
};
use writer::Writer;

const NAME: &str = "LowMonitor";
const USAGE: &str =
    "kullanım: lowmon-svc [install | uninstall [--purge] | start | stop | run [--dir <klasör>]]";

define_windows_service!(ffi_service_main, service_main);

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => service_dispatcher::start(NAME, ffi_service_main)?,
        Some("install") => install()?,
        Some("uninstall") => uninstall(args.get(1).is_some_and(|a| a == "--purge"))?,
        Some("run") => run_console(&args[1..])?,
        Some("start") => open(ServiceAccess::START)?.start::<&str>(&[])?,
        Some("stop") => {
            open(ServiceAccess::STOP)?.stop()?;
        }
        Some(other) => {
            eprintln!("bilinmeyen komut: {other}\n{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

/// Servis verisi: `%ProgramData%\LowMonitor`.
fn service_root() -> PathBuf {
    let base = std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into());
    PathBuf::from(base).join(NAME)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn uptime_ms() -> u64 {
    // SAFETY: parametresiz, yan etkisiz.
    unsafe { GetTickCount64() }
}

/// config.toml yoksa varsayılan; hatalıysa varsayılan + hata metni.
fn load_config(root: &Path) -> (Config, Option<String>) {
    match fs::read_to_string(root.join("config.toml")) {
        Err(_) => (Config::default(), None),
        Ok(text) => match Config::parse(&text) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e)),
        },
    }
}

fn manager(access: ServiceManagerAccess) -> windows_service::Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | access)
}

fn open(access: ServiceAccess) -> windows_service::Result<windows_service::service::Service> {
    manager(ServiceManagerAccess::empty())?.open_service(NAME, access)
}

fn install() -> Result<(), Box<dyn Error>> {
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: "LowMonitor".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: Some(r"NT AUTHORITY\LocalService".into()),
        account_password: None,
    };
    let s = manager(ServiceManagerAccess::CREATE_SERVICE)?
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
    s.set_description("Düşük tüketimli kaynak izleme ve loglama")?;
    s.set_delayed_auto_start(true)?;
    s.set_config_service_sid_info(ServiceSidType::Unrestricted)?;
    s.set_preshutdown_timeout(Duration::from_secs(5))?;
    // Gereksiz privilege'lar düşer (spek §11 adım 3); dizin geçişi için tek bir tane yeterli.
    let mut privs = security::wide("SeChangeNotifyPrivilege\0");
    let req = SERVICE_REQUIRED_PRIVILEGES_INFOW {
        pmszRequiredPrivileges: privs.as_mut_ptr(),
    };
    // SAFETY: SC handle canlı; multi-sz çift NUL ile bitiyor.
    if unsafe {
        ChangeServiceConfig2W(
            s.raw_handle(),
            SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
            (&raw const req).cast(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let restart = |secs| ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(secs),
    };
    s.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(5), restart(30), restart(300)]),
    })?;
    // Başlatma hatası (ör. key.bin okunamadı) da yeniden deneme tetiklesin
    s.set_failure_actions_on_non_crash_failures(true)?;

    // Klasörler + ACL (spek §11 adım 5). key.bin'i servis ilk açılışta kendisi üretir (ADR-0016).
    let root = service_root();
    fs::create_dir_all(root.join("logs"))?;
    fs::create_dir_all(root.join("state"))?;
    let sid = security::service_sid(NAME)?;
    let config = root.join("config.toml");
    if !config.exists() {
        fs::write(&config, DEFAULT_TOML)?;
    }
    security::set_dacl(
        &config,
        &format!("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;{sid})"),
    )?;
    // Root en son: kalıtılabilir ACE'ler logs/ ve state/'e yayılır. Users: erişim yok.
    // BA = okuma + FILE_DELETE_CHILD (0x40): `uninstall --purge` key.bin'i silebilsin.
    security::set_dacl(
        &root,
        &format!("D:P(A;OICI;FA;;;SY)(A;OICI;0x1301bf;;;{sid})(A;OICI;0x1200c9;;;BA)"),
    )?;
    // lowmon-net LAN erişimi (spek §16.2); başarısızlık servis kurulumunu bozmaz
    let net = std::env::current_exe()?.with_file_name("lowmon-net.exe");
    match firewall::set(Some(&net)) {
        Ok(()) => println!(
            "firewall: \"{}\" (Private, yerel alt ağ, TCP {})",
            firewall::RULE,
            firewall::PORT
        ),
        Err(e) => {
            eprintln!("uyarı: firewall kuralı eklenemedi ({e}); LAN erişimi için elle ekleyin")
        }
    }
    println!("{NAME} kuruldu: {}", root.display());
    Ok(())
}

fn uninstall(purge: bool) -> Result<(), Box<dyn Error>> {
    let s = open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?;
    if s.query_status()?.current_state != ServiceState::Stopped {
        s.stop()?;
        // Seal yazılsın ve süreç key.bin'i bıraksın
        for _ in 0..50 {
            if s.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    s.delete()?;
    let _ = firewall::set(None);
    if purge {
        let root = service_root();
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        println!("{NAME} kaldırıldı, veri silindi: {}", root.display());
    } else {
        println!("{NAME} kaldırıldı (veri duruyor; silmek için --purge)");
    }
    Ok(())
}

fn service_main(_args: Vec<OsString>) {
    let _ = run_service();
}

fn run_service() -> windows_service::Result<()> {
    let handle = service_control_handler::register(NAME, move |ctrl| match ctrl {
        ServiceControl::Stop | ServiceControl::Preshutdown => {
            signal_stop();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let status = |state, accept, code| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::ServiceSpecific(code),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    let root = service_root();
    let sid = security::service_sid(NAME).ok();
    let (cfg, cfg_err) = load_config(&root);
    let init = (|| {
        let key = keystore::load_or_create(&root.join("key.bin"), sid.as_deref())?;
        let ticker = Ticker::new(cfg.sampling.interval_ms)?;
        let writer = Writer::open(&root, key, cfg.sampling.interval_ms, now_ms(), uptime_ms())?;
        Ok::<_, std::io::Error>((ticker, writer, key))
    })();
    let Ok((ticker, mut writer, key)) = init else {
        // ServiceSpecific(1) → SCM failure actions (non-crash) yeniden dener
        return handle.set_service_status(status(
            ServiceState::Stopped,
            ServiceControlAccept::empty(),
            1,
        ));
    };
    let _ = STOP.set(ticker.stop_handle());
    handle.set_service_status(status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::PRESHUTDOWN,
        0,
    ))?;
    let mut sched = Scheduler::new(&root, key, cfg.retention.raw_days, cfg.retention.total_days);
    let publ = Publisher::default();
    // Pipe açılamazsa servis pipe'sız toplamaya devam eder (pipe_unavailable olayı)
    let (srv, cmds) = server(&publ, &cfg, &root, key);
    let start = start_events(cfg_err.is_some(), pipe::spawn(PIPE_NAME, srv).is_ok());
    collect_loop(
        &ticker,
        &cfg,
        &root,
        &mut writer,
        &mut sched,
        &publ,
        &cmds,
        &start,
        false,
    );
    handle.set_service_status(status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        0,
    ))
}

/// Durdurma: servis kontrol handler'ı ve konsol Ctrl+C aynı yolu kullanır.
static STOP: OnceLock<StopHandle> = OnceLock::new();

fn signal_stop() {
    if let Some(s) = STOP.get() {
        s.signal();
    }
}

unsafe extern "system" fn on_ctrl(_ctrl_type: u32) -> windows_sys::core::BOOL {
    signal_stop();
    1 // işlendi: süreç öldürülmez, döngü Seal yazıp çıkar
}

/// `run [--dir <klasör>]`: kullanıcı oturumunda, ekrana basar + `%LOCALAPPDATA%\LowMonitor-dev`'e yazar.
fn run_console(args: &[String]) -> Result<(), Box<dyn Error>> {
    let root = match args {
        [flag, dir] if flag == "--dir" => PathBuf::from(dir),
        [] => PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA yok")?)
            .join("LowMonitor-dev"),
        _ => return Err(USAGE.into()),
    };
    fs::create_dir_all(&root)?;
    let (cfg, err) = load_config(&root);
    if let Some(e) = &err {
        eprintln!("config.toml hatalı, varsayılanlar kullanılıyor: {e}");
    }
    let key = keystore::load_or_create(&root.join("key.bin"), None)?;
    let ticker = Ticker::new(cfg.sampling.interval_ms)?;
    let mut writer = Writer::open(&root, key, cfg.sampling.interval_ms, now_ms(), uptime_ms())?;
    let _ = STOP.set(ticker.stop_handle());
    // SAFETY: on_ctrl statik fonksiyon, süreç boyunca geçerli.
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
    eprintln!("veri: {} (çıkış: Ctrl+C)", root.display());
    let mut sched = Scheduler::new(&root, key, cfg.retention.raw_days, cfg.retention.total_days);
    let publ = Publisher::default();
    let (srv, cmds) = server(&publ, &cfg, &root, key);
    let pipe = pipe::spawn(PIPE_NAME, srv);
    if let Err(e) = &pipe {
        eprintln!("pipe açılamadı (servis çalışıyor olabilir), pipe'sız devam: {e}");
    }
    let start = start_events(err.is_some(), pipe.is_ok());
    collect_loop(
        &ticker,
        &cfg,
        &root,
        &mut writer,
        &mut sched,
        &publ,
        &cmds,
        &start,
        true,
    );
    Ok(())
}

fn server(
    publ: &Publisher,
    cfg: &Config,
    root: &Path,
    key: [u8; 32],
) -> (Arc<Server>, Receiver<CmdReq>) {
    // Kapasite = eşzamanlı istek; dolarsa Export flush'sız devam eder, Control `Io` alır.
    let (tx, rx) = sync_channel(8);
    publ.set_interval(cfg.sampling.interval_ms);
    let srv = Arc::new(Server {
        publ: publ.clone(),
        root: root.to_path_buf(),
        key,
        cmd: tx,
    });
    (srv, rx)
}

/// ReloadConfig: hatalı/okunamayan dosya reddedilir (`Io`, mevcut ayar kalır).
fn reload_config(root: &Path) -> Result<Config, ErrorCode> {
    let text = fs::read_to_string(root.join("config.toml")).map_err(|_| ErrorCode::Io)?;
    Config::parse(&text).map_err(|_| ErrorCode::Io)
}

/// Açılışta bilinen sistem olayları (ilk örnekten sonra yazılır).
fn start_events(config_invalid: bool, pipe_ok: bool) -> Vec<&'static str> {
    let mut v = vec!["service_start"];
    if config_invalid {
        v.push("config_invalid");
    }
    if !pipe_ok {
        v.push("pipe_unavailable");
    }
    v
}

/// System olayları (ADR-0028): aynı ad dakikada ≤ 1 (disk dolu sırasında taşkın yok).
/// Kuyrukta bekler, örnekten sonra yazılır → zamanı o örneğin ts'i.
#[derive(Default)]
struct SysEvents {
    pending: Vec<(&'static str, i64)>,
    last: Vec<(&'static str, u64)>,
}

impl SysEvents {
    fn push(&mut self, name: &'static str, value: i64, now_ms: u64) {
        match self.last.iter_mut().find(|l| l.0 == name) {
            Some(l) if now_ms < l.1 + 60_000 => return,
            Some(l) => l.1 = now_ms,
            None => self.last.push((name, now_ms)),
        }
        self.pending.push((name, value));
    }

    fn drain(&mut self, w: &mut Writer, publ: &Publisher, ts_ms: u64) {
        for (name, value) in self.pending.drain(..) {
            emit(w, publ, ts_ms, EventKind::System, name, value);
        }
    }
}

/// Olay: log'a yazılır + `CH_EVENT` abonelerine gider.
fn emit(w: &mut Writer, publ: &Publisher, ts_ms: u64, kind: EventKind, name: &str, value: i64) {
    w.event(kind, name, value);
    publ.publish(CH_EVENT, || ServerMsg::Event {
        ts_ms,
        kind,
        name: name.to_owned(),
        value,
    });
}

/// Servis ve `run` konsol modu aynı döngüyü kullanır; durdurulunca Seal yazılır.
#[allow(clippy::too_many_arguments)]
fn collect_loop(
    ticker: &Ticker,
    cfg: &Config,
    root: &Path,
    writer: &mut Writer,
    sched: &mut Scheduler,
    publ: &Publisher,
    cmds: &Receiver<CmdReq>,
    start: &[&'static str],
    print: bool,
) {
    ticker::enter_eco_mode();
    let mut cfg = cfg.clone();
    let new_collector = |c: &Config| Collector::new(c.sampling.topn_every, c.sampling.topn_count);
    let mut collector = new_collector(&cfg);
    let (mut s, mut top) = (Sample::default(), Vec::new());
    let mut paused = false;
    let (mut engine, mut fired) = (Engine::new(&cfg.alarm), Vec::new());
    let mut sys = SysEvents::default();
    for &name in start {
        sys.push(name, 0, now_ms());
    }
    if writer.integrity {
        sys.push("integrity", 0, now_ms());
    }
    // UAC istemi (ADR-0037): config kapalıysa oturum/thread yok; ReloadConfig açar/kapatır.
    let start_uac = |sys: &mut SysEvents| {
        uac::start()
            .map_err(|code| sys.push("uac_unavailable", i64::from(code), now_ms()))
            .ok()
    };
    let mut uac = if cfg.events.uac {
        start_uac(&mut sys)
    } else {
        None
    };
    let mut uac_open = false;
    let active = |e: &Engine, open: bool| {
        let mut v = e.active();
        if open {
            v.push("uac".into());
        }
        v
    };
    let retention = |sys: &mut SysEvents, r: Option<std::io::Result<retention::Report>>| match r {
        Some(Ok(rep)) => sys.push("retention_ok", rep.deleted as i64, now_ms()),
        Some(Err(_)) => sys.push("retention_error", 0, now_ms()),
        None => {}
    };
    while ticker.wait() {
        // Pipe komutları (≤ 1 tick gecikme, ek uyanma yok)
        while let Ok((cmd, done)) = cmds.try_recv() {
            let r = match cmd {
                Cmd::Flush => writer.flush().map_err(|_| ErrorCode::Io),
                Cmd::Control(Control::Pause) => {
                    paused = true;
                    engine.gap();
                    sys.push("control_pause", 0, now_ms());
                    sys.drain(writer, publ, s.ts_unix_ms); // duraklamadan önce, son örneğin ts'iyle
                    writer.pause().map_err(|_| ErrorCode::Io)
                }
                Cmd::Control(Control::Resume) => {
                    if std::mem::take(&mut paused) {
                        collector = new_collector(&cfg); // eski delta'lar duraklamayı kapsar → ilk örnek atılır
                        sys.push("control_resume", 0, now_ms());
                    }
                    Ok(())
                }
                Cmd::Control(Control::ReloadConfig) => reload_config(root).and_then(|new| {
                    let ms = new.sampling.interval_ms;
                    // Aralık çalışırken (ADR-0027): önce timer — kurulamazsa hiçbir şey değişmez
                    if ms != cfg.sampling.interval_ms {
                        ticker.set_period(ms).map_err(|_| ErrorCode::Io)?;
                        writer.set_interval(ms);
                        publ.set_interval(ms);
                    }
                    sched.set_days(new.retention.raw_days, new.retention.total_days);
                    collector = new_collector(&new); // aralık/Top-N değişti → delta'lar sıfırdan
                    for id in engine.reload(&new.alarm) {
                        // kaldırılan/değişen Raised kural
                        emit(writer, publ, s.ts_unix_ms, EventKind::AlarmClear, &id, 0);
                    }
                    if new.events.uac != cfg.events.uac {
                        uac = None; // eski oturum durur (Drop)
                        if new.events.uac {
                            uac = start_uac(&mut sys);
                        }
                        if std::mem::take(&mut uac_open) {
                            emit(writer, publ, s.ts_unix_ms, EventKind::AlarmClear, "uac", 0);
                        }
                    }
                    publ.set_alarms(active(&engine, uac_open));
                    cfg = new;
                    Ok(())
                }),
            };
            if matches!(cmd, Cmd::Control(Control::ReloadConfig)) {
                sys.push("control_reload", r.is_ok() as i64, now_ms());
            }
            if print {
                eprintln!("komut {cmd:?}: {r:?}");
            }
            let _ = done.send(r);
        }
        if paused {
            retention(&mut sys, sched.tick(now_ms())); // retention duraklamada da çalışır
            continue;
        }
        let got = collector.collect(&mut s, &mut top);
        if got.sample {
            // Yazma hatası (disk dolu vb.): toplama sürer, write_error olayı dakikada ≤ 1
            let gap = writer.sample(&s).unwrap_or_else(|e| {
                if print {
                    eprintln!("log yazma hatası: {e}");
                }
                sys.push("write_error", 0, s.ts_unix_ms);
                false
            });
            if gap {
                engine.gap(); // uyku / duraklama: yarım süreler geçersiz
            }
            engine.step(&s, &collector.ctx, &mut fired);
            for (id, t, v) in &fired {
                let kind = match t {
                    Transition::Raise => EventKind::AlarmRaise,
                    Transition::Clear => EventKind::AlarmClear,
                };
                emit(writer, publ, s.ts_unix_ms, kind, id, (v * 100.0) as i64);
                if print {
                    eprintln!("alarm {kind:?} {id} = {v:.2}");
                }
            }
            let mut uac_changed = false;
            while let Some(open) = uac.as_ref().and_then(|u| u.rx.try_recv().ok()) {
                let kind = if open {
                    EventKind::AlarmRaise
                } else {
                    EventKind::AlarmClear
                };
                emit(writer, publ, s.ts_unix_ms, kind, "uac", 0);
                uac_open = open;
                uac_changed = true;
            }
            if !fired.is_empty() || uac_changed {
                publ.set_alarms(active(&engine, uac_open));
            }
            sys.drain(writer, publ, s.ts_unix_ms);
        }
        if got.top {
            writer.top(&top);
            publ.publish(CH_TOP, || ServerMsg::Top(top.clone()));
        }
        if got.sample {
            publ.publish(CH_SAMPLE, || ServerMsg::Sample(s.clone()));
        }
        retention(&mut sys, sched.tick(s.ts_unix_ms));
        if print && got.sample {
            print_sample(&s);
        }
        if print && got.top {
            print_top(&top);
        }
    }
    if let Err(e) = writer.close()
        && print
    {
        eprintln!("kapanışta log yazma hatası: {e}");
    }
}

fn print_top(top: &[TopEntry]) {
    let rows: Vec<String> = top
        .iter()
        .map(|e| {
            format!(
                "{} ({}) {:.2}% {} MB",
                e.name,
                e.pid,
                e.cpu as f32 / 100.0,
                e.ws_kb / 1024
            )
        })
        .collect();
    println!("top: {}", rows.join(" | "));
}

fn print_sample(s: &Sample) {
    let cores: Vec<String> = s
        .cpu_core
        .iter()
        .map(|&c| format!("{:.1}", c as f32 / 2.0))
        .collect();
    println!(
        "cpu {:6.2}% | ram {:6} MB | commit {:6} MB | cache {:6} MB | disk({}) r {:7} w {:7} KB/s | net({}) ↓{:7} ↑{:7} KB/s | çekirdek [{}]",
        s.cpu_total as f32 / 100.0,
        s.ram_used_kb / 1024,
        s.commit_kb / 1024,
        s.cache_kb / 1024,
        s.disk.len(),
        s.disk.iter().map(|d| d.r_bps).sum::<u64>() / 1024,
        s.disk.iter().map(|d| d.w_bps).sum::<u64>() / 1024,
        s.net.len(),
        s.net.iter().map(|n| n.rx_bps).sum::<u64>() / 1024,
        s.net.iter().map(|n| n.tx_bps).sum::<u64>() / 1024,
        cores.join(" ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sys_events_rate_limited_per_name() {
        let mut e = SysEvents::default();
        e.push("write_error", 0, 1_000);
        e.push("write_error", 0, 30_000); // aynı dakika → atılır
        e.push("retention_ok", 3, 30_000); // başka ad → geçer
        e.push("write_error", 0, 61_000);
        let names: Vec<_> = e.pending.iter().map(|p| p.0).collect();
        assert_eq!(names, ["write_error", "retention_ok", "write_error"]);
        assert_eq!(
            start_events(true, false),
            ["service_start", "config_invalid", "pipe_unavailable"]
        );
    }

    #[test]
    fn reload_config_accepts_changes_and_rejects_bad_file() {
        let dir = std::env::temp_dir().join(format!("lowmon-test-{}-reload", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let write = |s: &str| fs::write(dir.join("config.toml"), s).unwrap();
        assert_eq!(reload_config(&dir), Err(ErrorCode::Io)); // dosya yok
        write("[sampling]\ntopn_every = 10\n[retention]\nraw_days = 3\ntotal_days = 20\n");
        let new = reload_config(&dir).unwrap();
        assert_eq!(
            (
                new.sampling.topn_every,
                new.retention.raw_days,
                new.retention.total_days
            ),
            (10, 3, 20)
        );
        write("[retention]\nraw_days = 40\n");
        assert_eq!(reload_config(&dir), Err(ErrorCode::Io)); // doğrulama hatası
        write("[sampling]\ninterval_ms = 2000\n");
        assert_eq!(reload_config(&dir).unwrap().sampling.interval_ms, 2000); // ADR-0027: kabul
        let _ = fs::remove_dir_all(&dir);
    }
}
