use crate::adapter;

pub fn create() -> Option<Box<dyn IDesktopService + Send>> {
    if cfg!(target_os = "windows") {
        return Some(Box::new(adapter::WindowsDesktopService::new()));
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub enum DesktopServiceState {
    Paused,
    Started,
    Stopped,
    Unknown,
}

pub trait IDesktopService {
    fn start(&mut self);
    fn stop(&mut self);
    fn restart(&mut self);
    fn pause(&mut self);
    fn check(&mut self) -> DesktopServiceState;
}
