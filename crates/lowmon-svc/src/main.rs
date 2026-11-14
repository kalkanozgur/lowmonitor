//! lowmon-svc — servis + install/start/stop/uninstall (spek §11), `run` = konsol modu (geliştirme).
mod ticker;

use std::{error::Error, ffi::OsString, time::Duration};
use ticker::Ticker;
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

const NAME: &str = "LowMonitor";
const INTERVAL_MS: u32 = 1000; // shortcut: sabit; config.toml [sampling] M2'de

define_windows_service!(ffi_service_main, service_main);

fn main() -> Result<(), Box<dyn Error>> {
    match std::env::args().nth(1).as_deref() {
        None => service_dispatcher::start(NAME, ffi_service_main)?,
        Some("install") => install()?,
        Some("uninstall") => uninstall()?,
        Some("run") => collect_loop(&Ticker::new(INTERVAL_MS)?), // Ctrl+C ile çıkılır
        Some("start") => open(ServiceAccess::START)?.start::<&str>(&[])?,
        Some("stop") => {
            open(ServiceAccess::STOP)?.stop()?;
        }
        Some(other) => {
            eprintln!(
                "bilinmeyen komut: {other}\nkullanım: lowmon-svc [install|uninstall|start|stop|run]"
            );
            std::process::exit(2);
        }
    }
    Ok(())
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
    // shortcut: ACL, ProgramData klasörleri, key.bin, config.toml ve required privileges M2'de (spek §11 adım 3, 5–7)
    println!("{NAME} kuruldu");
    Ok(())
}

fn uninstall() -> Result<(), Box<dyn Error>> {
    let s = open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?;
    if s.query_status()?.current_state != ServiceState::Stopped {
        s.stop()?;
    }
    s.delete()?;
    println!("{NAME} kaldırıldı");
    Ok(())
}

fn service_main(_args: Vec<OsString>) {
    // shortcut: hata henüz loglanmıyor; M2'de log/Event altyapısı gelince yazılır
    let _ = run();
}

fn run() -> windows_service::Result<()> {
    // shortcut: Ticker hatası servisi Running'e geçmeden sonlandırır; SCM failure actions yeniden dener
    let Ok(ticker) = Ticker::new(INTERVAL_MS) else {
        return Ok(());
    };
    let stop = ticker.stop_handle();
    let handle = service_control_handler::register(NAME, move |ctrl| match ctrl {
        ServiceControl::Stop | ServiceControl::Preshutdown => {
            stop.signal();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let status = |state, accept| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    handle.set_service_status(status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::PRESHUTDOWN,
    ))?;
    collect_loop(&ticker);
    handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))
}

/// Servis ve `run` konsol modu aynı döngüyü kullanır.
fn collect_loop(ticker: &Ticker) {
    ticker::enter_eco_mode();
    while ticker.wait() {}
}
