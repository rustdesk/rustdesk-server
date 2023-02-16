use std::time::{Duration, Instant};

use super::{service, DesktopServiceState, Event};
use crate::BUFFER;
use async_std::task::sleep;
use crossbeam_channel::{Receiver, Sender};

pub async fn create(sender: Sender<Event>, receiver: Receiver<Event>) {
    let mut now = Instant::now();
    let buffer = BUFFER.get().unwrap().to_owned();
    let send = |event| sender.send(event).unwrap_or_default();
    if let Some(mut service) = service::create() {
        let mut service_state = DesktopServiceState::Unknown;
        let mut file = "hbbs.out".to_owned();
        send(Event::ViewRenderServiceState(service_state.to_owned()));
        loop {
            for _ in 1..buffer {
                match receiver.recv_timeout(Duration::from_nanos(1)) {
                    Ok(event) => match event {
                        Event::BrowserInit => {
                            send(Event::BrowserUpdate(("file".to_owned(), file.to_owned())));
                        }
                        Event::BrowserAction(action) => match action.as_str() {
                            "restart" => service.restart(),
                            _ => (),
                        },
                        Event::FileChange(path) => {
                            if path == file {
                                send(Event::BrowserUpdate(("file".to_owned(), file.to_owned())));
                            }
                        }
                        Event::ViewAction(action) => match action.as_str() {
                            "start" => service.start(),
                            "stop" => service.stop(),
                            "restart" => service.restart(),
                            "pause" => service.pause(),
                            "exit" => send(Event::ViewRenderAppExit),
                            _ => {
                                file = action;
                                send(Event::BrowserUpdate(("file".to_owned(), file.to_owned())));
                            }
                        },
                        _ => (),
                    },
                    Err(_) => break,
                }
            }
            sleep(Duration::from_micros(999)).await;
            if now.elapsed().as_millis() > 999 {
                let state = service.check();
                if state != service_state {
                    service_state = state.to_owned();
                    send(Event::ViewRenderServiceState(state));
                }
                now = Instant::now();
            }
        }
    }
}
