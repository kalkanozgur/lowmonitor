//! lowmon-svc — servis + install/start/stop/uninstall (spek §11), `run` = konsol modu (geliştirme).
mod collect;
mod export;
mod firewall;
mod keystore;
mod pipe;
mod query;
mod retention;
mod security;
mod ticker;
mod writer;

use collect::Collector;
use lowmon_core::{
    config::{Config, DEFAULT_TOML},
    metrics::{Sample, TopEntry},
    proto::{CH_SAMPLE, CH_TOP, PIPE_NAME, ServerMsg},
};
use pipe::{FlushAck, Publisher, Server};
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
            // shortcut: hatalı config sessizce varsayılana düşer; M5'te Event olarak loglanır
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
    let (cfg, _) = load_config(&root);
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
    // shortcut: pipe açılamazsa servis pipe'sız toplamaya devam eder; Event kaydı M5'te
    let (srv, flush) = server(&publ, &cfg, &root, key);
    let _ = pipe::spawn(PIPE_NAME, srv);
    collect_loop(&ticker, &cfg, &mut writer, &mut sched, &publ, &flush, false);
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
    if let Some(e) = err {
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
    let (srv, flush) = server(&publ, &cfg, &root, key);
    if let Err(e) = pipe::spawn(PIPE_NAME, srv) {
        eprintln!("pipe açılamadı (servis çalışıyor olabilir), pipe'sız devam: {e}");
    }
    collect_loop(&ticker, &cfg, &mut writer, &mut sched, &publ, &flush, true);
    Ok(())
}

fn server(
    publ: &Publisher,
    cfg: &Config,
    root: &Path,
    key: [u8; 32],
) -> (Arc<Server>, Receiver<FlushAck>) {
    // Kapasite = eşzamanlı Export sayısı; dolarsa Export flush beklemeden devam eder.
    let (tx, rx) = sync_channel(8);
    let srv = Arc::new(Server {
        publ: publ.clone(),
        interval_ms: cfg.sampling.interval_ms,
        root: root.to_path_buf(),
        key,
        flush: tx,
    });
    (srv, rx)
}

/// Servis ve `run` konsol modu aynı döngüyü kullanır; durdurulunca Seal yazılır.
fn collect_loop(
    ticker: &Ticker,
    cfg: &Config,
    writer: &mut Writer,
    sched: &mut Scheduler,
    publ: &Publisher,
    flush: &Receiver<FlushAck>,
    print: bool,
) {
    ticker::enter_eco_mode();
    let mut collector = Collector::new(cfg.sampling.topn_every, cfg.sampling.topn_count);
    let (mut s, mut top) = (Sample::default(), Vec::new());
    while ticker.wait() {
        let got = collector.collect(&mut s, &mut top);
        // shortcut: yazma hatası (disk dolu vb.) yalnızca konsolda görünür; servis toplamaya devam eder, Event M5'te
        if got.sample
            && let Err(e) = writer.sample(&s)
            && print
        {
            eprintln!("log yazma hatası: {e}");
        }
        if got.top {
            writer.top(&top);
            publ.publish(CH_TOP, || ServerMsg::Top(top.clone()));
        }
        if got.sample {
            publ.publish(CH_SAMPLE, || ServerMsg::Sample(s.clone()));
        }
        // Export öncesi: yarım blok da diske (yalnız Admin Export tetikler → ek yazma sınırlı)
        while let Ok(ack) = flush.try_recv() {
            if let Err(e) = writer.flush()
                && print
            {
                eprintln!("flush hatası: {e}");
            }
            let _ = ack.send(());
        }
        sched.tick(s.ts_unix_ms);
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
