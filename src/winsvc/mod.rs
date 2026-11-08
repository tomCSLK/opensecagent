//! The Windows service `opensecagent`: the service manager starts `opensecagent.exe run` as LocalSystem, with a
//! delayed automatic start after a boot, and starts it again after a failure. Stopping goes through the service
//! manager instead of SIGTERM. The lines of the service go to `opensecagent.log` in the agent's folder (`log`).
//!
//! `service-install` and `service-uninstall` (hidden, for the installer, as administrator) set the service up and
//! remove it.

pub mod folder;

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::agent::{Agent, Error};
use crate::store::Store;
use crate::{log, service};

/// The name of the service.
pub const NAME: &str = "opensecagent";
const DESCRIPTION: &str = "Reports the installed software of this computer to opensecmon.";
/// The log in the agent's folder.
const LOG: &str = "opensecagent.log";

/// `StartServiceCtrlDispatcher`: the program was not started by the service manager.
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

/// Exit codes of the service (service specific), shown by `sc query opensecagent`.
const EXIT_FOLDER: u32 = 1;
const EXIT_FAILED: u32 = 2;

/// Restarts after a failure: after 1 min, again after 1 min, then after 5 min each; counted anew after a day.
const RESTARTS_S: [u64; 3] = [60, 60, 300];
const RESET_S: u64 = 24 * 3600;
/// How long `service-uninstall` waits for the service to stop.
const STOP_WAIT: Duration = Duration::from_secs(30);

/// The agent's folder for the service thread, which the service manager starts without our arguments.
static STORE: OnceLock<Store> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// `opensecagent run`: as a service when the service manager started the program, otherwise in the terminal until
/// Ctrl+C (lines on standard output, as on Linux).
pub fn run(store: Store) -> ExitCode {
    let _ = STORE.set(store.clone());
    match service_dispatcher::start(NAME, ffi_service_main) {
        // The service has stopped.
        Ok(()) => ExitCode::SUCCESS,
        Err(windows_service::Error::Winapi(e))
            if e.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) =>
        {
            let result = runtime().and_then(|rt| {
                rt.block_on(service::run(Agent::new(store), service::stop_signal()))
                    .map_err(|e| e.to_string())
            });
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("error: the service could not be started: {e}");
            ExitCode::FAILURE
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

/// Called by the service manager in its own thread.
fn service_main(_arguments: Vec<OsString>) {
    if let Some(store) = STORE.get().cloned() {
        // Without a working status handle there is nobody to tell.
        let _ = serve(store);
    }
}

fn serve(store: Store) -> windows_service::Result<()> {
    let stop = Arc::new(Notify::new());
    let on_stop = Arc::clone(&stop);
    let handle = service_control_handler::register(NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            // Kept until the service waits for it, if it does not yet.
            on_stop.notify_one();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let status = |state, controls, exit_code| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: controls,
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    };
    // The folder first: the log goes there, and a folder someone else made (or a link) is not used.
    let exit = if store.secure().is_err() {
        ServiceExitCode::ServiceSpecific(EXIT_FOLDER)
    } else {
        log::to_file(store.dir().join(LOG));
        std::panic::set_hook(Box::new(|info| log::line(format_args!("panic: {info}"))));
        handle.set_service_status(status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            ServiceExitCode::NO_ERROR,
        ))?;
        let result = runtime().and_then(|rt| {
            rt.block_on(service::run(Agent::new(store), async move {
                stop.notified().await;
            }))
            .map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => ServiceExitCode::NO_ERROR,
            Err(e) => {
                log::line(format_args!("the service failed: {e}"));
                ServiceExitCode::ServiceSpecific(EXIT_FAILED)
            }
        }
    };
    handle.set_service_status(status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit,
    ))
}

/// `service-install`: creates the agent's folder with its access list, sets the service up (or updates it, e.g.
/// with a new path) and starts it.
pub fn install() -> Result<(), Error> {
    let store = Store::own();
    store
        .secure()
        .map_err(|e| Error::Files(store.dir().to_path_buf(), e))?;
    let exe = std::env::current_exe().map_err(|e| Error::Service(e.to_string()))?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec!["run".into()],
        dependencies: Vec::new(),
        // LocalSystem
        account_name: None,
        account_password: None,
    };
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(failed)?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::CHANGE_CONFIG;
    let service = match manager.open_service(NAME, access) {
        Ok(service) => {
            service.change_config(&info).map_err(failed)?;
            service
        }
        Err(e) if os_error(&e) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => {
            manager.create_service(&info, access).map_err(failed)?
        }
        Err(e) => return Err(failed(e)),
    };
    service.set_description(DESCRIPTION).map_err(failed)?;
    service.set_delayed_auto_start(true).map_err(failed)?;
    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(RESET_S)),
            reboot_msg: None,
            command: None,
            actions: Some(
                RESTARTS_S
                    .iter()
                    .map(|&s| ServiceAction {
                        action_type: ServiceActionType::Restart,
                        delay: Duration::from_secs(s),
                    })
                    .collect(),
            ),
        })
        .map_err(failed)?;
    // Also when the service stops with an error of its own, not only when it crashes.
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(failed)?;
    if service.query_status().map_err(failed)?.current_state == ServiceState::Stopped {
        service.start::<&str>(&[]).map_err(failed)?;
    }
    Ok(())
}

/// `service-uninstall`: stops and removes the service; whether there was one. The agent's files stay
/// (`uninstall` removes the enrollment).
pub fn uninstall() -> Result<bool, Error> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(failed)?;
    let service = match manager.open_service(
        NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        Ok(service) => service,
        Err(e) if os_error(&e) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => return Ok(false),
        Err(e) => return Err(failed(e)),
    };
    if service.query_status().map_err(failed)?.current_state != ServiceState::Stopped {
        // It may be stopping already.
        let _ = service.stop();
        let start = Instant::now();
        while service.query_status().map_err(failed)?.current_state != ServiceState::Stopped {
            if start.elapsed() > STOP_WAIT {
                return Err(Error::Service("the service did not stop".into()));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    service.delete().map_err(failed)?;
    Ok(true)
}

fn os_error(e: &windows_service::Error) -> Option<i32> {
    match e {
        windows_service::Error::Winapi(e) => e.raw_os_error(),
        _ => None,
    }
}

fn failed(e: windows_service::Error) -> Error {
    let hint = match os_error(&e) {
        // ERROR_ACCESS_DENIED
        Some(5) => " (run it as administrator)",
        _ => "",
    };
    match e {
        windows_service::Error::Winapi(io) => Error::Service(format!("{io}{hint}")),
        e => Error::Service(e.to_string()),
    }
}
