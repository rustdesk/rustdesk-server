use super::DesktopServiceState;
use crate::adapter::desktop;
use crossbeam_channel::{Receiver, Sender};

pub async fn create(sender: Sender<Event>, receiver: Receiver<Event>) {
    desktop::run(sender, receiver).await;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    BrowserAction(String),
    BrowserInit,
    BrowserUpdate((String, String)),
    BrowserRender(String),
    FileChange(String),
    ViewAction(String),
    ViewInit,
    ViewUpdate(String),
    ViewRender(String),
    ViewRenderAppExit,
    ViewRenderServiceState(DesktopServiceState),
}
