use std::{ffi::OsStr, process::Command};

use crate::{path, usecase::service::*};
use derive_new::new;
use windows_service::{
    service::ServiceAccess,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[derive(Debug, new)]
pub struct WindowsDesktopService {
    #[new(value = "DesktopServiceState::Stopped")]
    pub state: DesktopServiceState,
}

impl IDesktopService for WindowsDesktopService {
    fn start(&mut self) {
        call(
            [
                "echo.",
                "%nssm% stop hbbr",
                "%nssm% remove hbbr confirm",
                "%nssm% stop hbbs",
                "%nssm% remove hbbs confirm",
                "mkdir logs",
                "echo.",
                "service\\run.cmd hbbs",
                "echo.",
                "service\\run.cmd hbbr",
                "echo.",
                "@ping 127.1 -n 3 >nul",
            ]
            .join(" & "),
        );
        self.check();
    }
    fn stop(&mut self) {
        call(
            [
                "echo.",
                "%nssm% stop hbbr",
                "%nssm% remove hbbr confirm",
                "echo.",
                "%nssm% stop hbbs",
                "%nssm% remove hbbs confirm",
                "echo.",
                "@ping 127.1 -n 3 >nul",
            ]
            .join(" & "),
        );
        self.check();
    }
    fn restart(&mut self) {
        nssm(["restart", "hbbs"].map(|x| x.to_owned()));
        nssm(["restart", "hbbr"].map(|x| x.to_owned()));
        self.check();
    }
    fn pause(&mut self) {
        call(
            [
                "echo.",
                "%nssm% stop hbbr",
                "echo.",
                "%nssm% stop hbbs",
                "echo.",
                "@ping 127.1 -n 3 >nul",
            ]
            .join(" & "),
        );
        self.check();
    }
    fn check(&mut self) -> DesktopServiceState {
        self.state = match service_status("hbbs").as_str() {
            "Running" => DesktopServiceState::Started,
            // "Stopped" => DeskServerServiceState::Paused,
            _ => DesktopServiceState::Stopped,
        };
        self.state.to_owned()
    }
}

fn call(cmd: String) {
    Command::new("cmd")
        .current_dir(&path())
        .env("nssm", "service\\nssm.exe")
        .arg("/c")
        .arg("start")
        .arg("cmd")
        .arg("/c")
        .arg(cmd)
        .output()
        .expect("cmd exec error!");
}

fn exec<I, S>(program: S, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    match Command::new(program).args(args).output() {
        Ok(out) => String::from_utf8(out.stdout).unwrap_or("".to_owned()),
        Err(e) => e.to_string(),
    }
}

fn nssm<I>(args: I) -> String
where
    I: IntoIterator<Item = String>,
{
    exec(
        format!("{}\\service\\nssm.exe", path().to_str().unwrap_or_default()),
        args,
    )
    .replace("\0", "")
    .trim()
    .to_owned()
}

fn service_status(name: &str) -> String {
    match ServiceManager::local_computer(None::<&OsStr>, ServiceManagerAccess::CONNECT) {
        Ok(manager) => match manager.open_service(name, ServiceAccess::QUERY_STATUS) {
            Ok(service) => match service.query_status() {
                Ok(status) => format!("{:?}", status.current_state),
                Err(e) => e.to_string(),
            },
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    }
}
