//! Native SCM integration shared by the ID and relay executables.
use flexi_logger::{Cleanup, Criterion, FileSpec, Logger, Naming};
use hbb_common::{anyhow::{bail, Context}, tokio, ResultType};
use std::{ffi::OsString, path::PathBuf, sync::{Mutex, OnceLock}, time::Duration};
use ::windows_service::{
    define_windows_service,
    service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};

type ServerMain = fn() -> ResultType<()>;
static SERVER: OnceLock<(&'static str, ServerMain)> = OnceLock::new();
static STATUS: OnceLock<Mutex<(ServiceStatusHandle, ServiceState)>> = OnceLock::new();
static STOP: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();
static FAILURE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

pub fn is_service() -> bool {
    SERVER.get().is_some()
}

/// Return false for the existing foreground CLI; SCM mode is explicit.
pub fn dispatch(name: &'static str, run: ServerMain) -> ResultType<bool> {
    if !std::env::args_os().any(|arg| arg == "--service") {
        return Ok(false);
    }
    SERVER.set((name, run)).map_err(|_| hbb_common::anyhow::anyhow!("Service already initialized"))?;
    STOP.get_or_init(|| tokio::sync::watch::channel(false).0);
    FAILURE.get_or_init(|| Mutex::new(None));
    service_dispatcher::start(name, ffi_service_main)?;
    if let Some(failure) = FAILURE.get() {
        if let Some(message) = failure.lock().map_err(|_| hbb_common::anyhow::anyhow!("Service failure lock poisoned"))?.take() {
            bail!(message);
        }
    }
    Ok(true)
}

fn data_directory(args: impl Iterator<Item = OsString>) -> ResultType<PathBuf> {
    let mut args = args;
    let mut directory = None;
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            directory = Some(PathBuf::from(args.next().context("--data-dir requires an absolute path")?));
        } else if let Some(value) = arg.to_str().and_then(|value| value.strip_prefix("--data-dir=")) {
            directory = Some(PathBuf::from(value));
        }
    }
    let directory = match directory {
        Some(directory) => directory,
        None => PathBuf::from(std::env::var_os("ProgramData").context("ProgramData is not set")?).join("RustDeskServer"),
    };
    if !directory.is_absolute() {
        bail!("The service data directory must be an absolute path");
    }
    Ok(directory)
}

fn status(state: ServiceState, failed: bool) -> ServiceStatus {
    let pending = matches!(state, ServiceState::StartPending | ServiceState::StopPending);
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else { ServiceControlAccept::empty() },
        exit_code: if failed { ServiceExitCode::ServiceSpecific(1) } else { ServiceExitCode::Win32(0) },
        checkpoint: if pending { 1 } else { 0 },
        wait_hint: if pending { Duration::from_secs(30) } else { Duration::ZERO },
        process_id: None,
    }
}

fn report(state: ServiceState, failed: bool) -> ResultType<()> {
    if let Some(status_handle) = STATUS.get() {
        let mut current = status_handle.lock().map_err(|_| hbb_common::anyhow::anyhow!("Service status lock poisoned"))?;
        // A stop racing with listener initialization must never restore Running.
        if state == ServiceState::Running && current.1 != ServiceState::StartPending {
            return Ok(());
        }
        current.0.set_service_status(status(state, failed))?;
        current.1 = state;
    }
    Ok(())
}

/// Called only after configuration, database and listening sockets are ready.
pub fn ready() -> ResultType<()> {
    report(ServiceState::Running, false)
}

fn control(event: ServiceControl) -> ServiceControlHandlerResult {
    match event {
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Err(error) = report(ServiceState::StopPending, false) {
                hbb_common::log::error!("Cannot report stop pending: {error}");
            }
            if let Some(stop) = STOP.get() {
                // Retain a stop that arrives before the async receiver exists.
                stop.send_replace(true);
            }
            ServiceControlHandlerResult::NoError
        }
        _ => ServiceControlHandlerResult::NotImplemented,
    }
}

fn service_main(_: Vec<OsString>) {
    let Some((name, run)) = SERVER.get() else { return; };
    let result = (|| -> ResultType<()> {
        let handle = service_control_handler::register(name, control)?;
        STATUS.set(Mutex::new((handle, ServiceState::StartPending)))
            .map_err(|_| hbb_common::anyhow::anyhow!("Service status already initialized"))?;
        report(ServiceState::StartPending, false)?;
        let directory = data_directory(std::env::args_os().skip(1))?;
        std::fs::create_dir_all(directory.join("logs").join(name))?;
        std::env::set_current_dir(&directory)?;
        run()
    })();
    if let Err(error) = &result {
        // Include errors that happened before the application logger started.
        if let Ok(directory) = data_directory(std::env::args_os().skip(1)) {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true)
                .open(directory.join("logs").join(name).join("startup.log")) {
                let _ = writeln!(file, "{:?}: {error:#}", std::time::SystemTime::now());
            }
        }
        if let Some(failure) = FAILURE.get() {
            if let Ok(mut failure) = failure.lock() { *failure = Some(format!("{error:#}")); }
        }
    }
    if let Err(error) = report(ServiceState::Stopped, result.is_err()) {
        eprintln!("Cannot report service stopped: {error}");
    }
}

pub fn configure_logger(logger: Logger, name: &str) -> Logger {
    if is_service() {
        logger.log_to_file(FileSpec::default().directory(std::path::Path::new("logs").join(name)).basename(name))
            .rotate(Criterion::Size(10 * 1024 * 1024), Naming::Numbers, Cleanup::KeepLogFiles(5))
    } else { logger }
}

pub async fn wait_for_stop() -> ResultType<()> {
    if let Some(stop) = STOP.get() {
        let mut receiver = stop.subscribe();
        loop {
            if *receiver.borrow_and_update() { return Ok(()); }
            receiver.changed().await?;
        }
    } else {
        let mut interrupt = tokio::signal::windows::ctrl_c()?;
        let mut interrupt_break = tokio::signal::windows::ctrl_break()?;
        let mut close = tokio::signal::windows::ctrl_close()?;
        let mut shutdown = tokio::signal::windows::ctrl_shutdown()?;
        tokio::select! {
            _ = interrupt.recv() => {},
            _ = interrupt_break.recv() => {},
            _ = close.recv() => {},
            _ = shutdown.recv() => {},
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_data_path_must_be_absolute() {
        assert!(data_directory(["--data-dir", "relative"].map(OsString::from).into_iter()).is_err());
        assert!(data_directory(["--data-dir"].map(OsString::from).into_iter()).is_err());
        let path = data_directory(["--data-dir=C:\\Service Data"].map(OsString::from).into_iter()).unwrap();
        assert_eq!(path, PathBuf::from("C:\\Service Data"));
    }

    #[test]
    fn only_running_service_accepts_stop_and_failure_is_nonzero() {
        assert!(status(ServiceState::StartPending, false).controls_accepted.is_empty());
        assert!(status(ServiceState::Running, false).controls_accepted.contains(ServiceControlAccept::STOP));
        assert!(status(ServiceState::StopPending, false).controls_accepted.is_empty());
        assert_eq!(status(ServiceState::Stopped, true).exit_code, ServiceExitCode::ServiceSpecific(1));
    }

    #[test]
    fn stop_before_receiver_is_not_lost() {
        let (sender, _) = tokio::sync::watch::channel(false);
        sender.send_replace(true);
        assert!(*sender.subscribe().borrow());
    }

    #[test]
    fn pending_stop_completes_async_shutdown() {
        STOP.get_or_init(|| tokio::sync::watch::channel(false).0).send_replace(true);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(100), wait_for_stop()).await.unwrap().unwrap();
        });
    }
}
