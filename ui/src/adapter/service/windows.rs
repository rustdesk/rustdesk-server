use std::{ffi::OsStr, process::Command, time::Duration};
use crate::usecase::service::*;
use derive_new::new;
use windows_service::{service::ServiceAccess, service_manager::{ServiceManager, ServiceManagerAccess}};

#[derive(Debug, new)]
pub struct WindowsDesktopService {
    #[new(value = "DesktopServiceState::Stopped")]
    pub state: DesktopServiceState,
}

impl IDesktopService for WindowsDesktopService {
    fn start(&mut self) {
        for name in ["hbbs", "hbbr"] { control("start", name); }
        self.check();
    }
    fn stop(&mut self) {
        for name in ["hbbr", "hbbs"] { control("stop", name); }
        self.check();
    }
    fn restart(&mut self) {
        self.stop();
        for _ in 0..100 {
            if ["hbbs", "hbbr"].iter().all(|name| service_status(name) == "Stopped") { break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.start();
    }
    fn pause(&mut self) { self.stop(); }
    fn check(&mut self) -> DesktopServiceState {
        self.state = if ["hbbs", "hbbr"].iter().all(|name| service_status(name) == "Running") {
            DesktopServiceState::Started
        } else { DesktopServiceState::Stopped };
        self.state.to_owned()
    }
}

fn control(action: &str, name: &str) {
    // Registration is owned by the native installer. The GUI never recreates services.
    match Command::new("sc.exe").args([action, name]).output() {
        Ok(output) if !output.status.success() => eprintln!("Service {action} {name}: {}", String::from_utf8_lossy(&output.stdout)),
        Err(error) => eprintln!("Service {action} {name}: {error}"),
        _ => {}
    }
}

fn service_status(name: &str) -> String {
    match ServiceManager::local_computer(None::<&OsStr>, ServiceManagerAccess::CONNECT) {
        Ok(manager) => match manager.open_service(name, ServiceAccess::QUERY_STATUS) {
            Ok(service) => match service.query_status() {
                Ok(status) => format!("{:?}", status.current_state),
                Err(error) => error.to_string(),
            },
            Err(error) => error.to_string(),
        },
        Err(error) => error.to_string(),
    }
}
