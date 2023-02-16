use std::{
    process::exit,
    time::{Duration, Instant},
};

use crate::{
    path,
    usecase::{view::Event, DesktopServiceState},
    BUFFER,
};
use async_std::task::sleep;
use crossbeam_channel::{Receiver, Sender};
use tauri::{
    CustomMenuItem, Manager, Menu, MenuItem, Submenu, SystemTray, SystemTrayEvent, SystemTrayMenu,
    SystemTrayMenuItem, WindowEvent,
};

pub async fn run(sender: Sender<Event>, receiver: Receiver<Event>) {
    let setup_sender = sender.clone();
    let menu_sender = sender.clone();
    let tray_sender = sender.clone();
    let menu = Menu::new()
        .add_submenu(Submenu::new(
            "Service",
            Menu::new()
                .add_item(CustomMenuItem::new("restart", "Restart"))
                .add_native_item(MenuItem::Separator)
                .add_item(CustomMenuItem::new("start", "Start"))
                .add_item(CustomMenuItem::new("stop", "Stop")),
        ))
        .add_submenu(Submenu::new(
            "Logs",
            Menu::new()
                .add_item(CustomMenuItem::new("hbbs.out", "hbbs.out"))
                .add_item(CustomMenuItem::new("hbbs.err", "hbbs.err"))
                .add_native_item(MenuItem::Separator)
                .add_item(CustomMenuItem::new("hbbr.out", "hbbr.out"))
                .add_item(CustomMenuItem::new("hbbr.err", "hbbr.err")),
        ))
        .add_submenu(Submenu::new(
            "Configuration",
            Menu::new().add_item(CustomMenuItem::new(".env", ".env")),
        ));
    let tray = SystemTray::new().with_menu(
        SystemTrayMenu::new()
            .add_item(CustomMenuItem::new("restart", "Restart"))
            .add_native_item(SystemTrayMenuItem::Separator)
            .add_item(CustomMenuItem::new("start", "Start"))
            .add_item(CustomMenuItem::new("stop", "Stop"))
            .add_native_item(SystemTrayMenuItem::Separator)
            .add_item(CustomMenuItem::new("exit", "Exit GUI")),
    );
    let mut app = tauri::Builder::default()
        .on_window_event(|event| match event.event() {
            // WindowEvent::Resized(size) => {
            //     if size.width == 0 && size.height == 0 {
            //         event.window().hide().unwrap();
            //     }
            // }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                event.window().minimize().unwrap();
                event.window().hide().unwrap();
            }
            _ => {}
        })
        .menu(menu)
        .on_menu_event(move |event| {
            // println!(
            //     "send {}: {}",
            //     std::time::SystemTime::now()
            //         .duration_since(std::time::UNIX_EPOCH)
            //         .unwrap_or_default()
            //         .as_millis(),
            //     event.menu_item_id()
            // );
            menu_sender
                .send(Event::ViewAction(event.menu_item_id().to_owned()))
                .unwrap_or_default()
        })
        .system_tray(tray)
        .on_system_tray_event(move |app, event| match event {
            SystemTrayEvent::LeftClick { .. } => {
                let main = app.get_window("main").unwrap();
                if main.is_visible().unwrap() {
                    main.hide().unwrap();
                } else {
                    main.show().unwrap();
                    main.unminimize().unwrap();
                    main.set_focus().unwrap();
                }
            }
            SystemTrayEvent::MenuItemClick { id, .. } => {
                tray_sender.send(Event::ViewAction(id)).unwrap_or_default();
            }
            _ => {}
        })
        .setup(move |app| {
            setup_sender.send(Event::ViewInit).unwrap_or_default();
            app.listen_global("__action__", move |msg| {
                match msg.payload().unwrap_or_default() {
                    r#""__init__""# => setup_sender.send(Event::BrowserInit).unwrap_or_default(),
                    r#""restart""# => setup_sender
                        .send(Event::BrowserAction("restart".to_owned()))
                        .unwrap_or_default(),
                    _ => (),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![root])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");
    let mut now = Instant::now();
    let mut blink = false;
    let mut span = 0;
    let mut title = "".to_owned();
    let product = "RustDesk Server";
    let buffer = BUFFER.get().unwrap().to_owned();
    loop {
        for _ in 1..buffer {
            match receiver.recv_timeout(Duration::from_nanos(1)) {
                Ok(event) => {
                    let main = app.get_window("main").unwrap();
                    let menu = main.menu_handle();
                    let tray = app.tray_handle();
                    match event {
                        Event::BrowserUpdate((action, data)) => match action.as_str() {
                            "file" => {
                                let list = ["hbbs.out", "hbbs.err", "hbbr.out", "hbbr.err", ".env"];
                                let id = data.as_str();
                                if list.contains(&id) {
                                    for file in list {
                                        menu.get_item(file)
                                            .set_selected(file == id)
                                            .unwrap_or_default();
                                    }
                                    // println!(
                                    //     "emit {}: {}",
                                    //     std::time::SystemTime::now()
                                    //         .duration_since(std::time::UNIX_EPOCH)
                                    //         .unwrap_or_default()
                                    //         .as_millis(),
                                    //     data
                                    // );
                                    app.emit_all("__update__", (action, data))
                                        .unwrap_or_default();
                                }
                            }
                            _ => (),
                        },
                        Event::ViewRenderAppExit => exit(0),
                        Event::ViewRenderServiceState(state) => {
                            let enabled = |id, enabled| {
                                menu.get_item(id).set_enabled(enabled).unwrap_or_default();
                                tray.get_item(id).set_enabled(enabled).unwrap_or_default();
                            };
                            title = format!("{} {:?}", product, state);
                            main.set_title(title.as_str()).unwrap_or_default();
                            match state {
                                DesktopServiceState::Started => {
                                    enabled("start", false);
                                    enabled("stop", true);
                                    enabled("restart", true);
                                    blink = false;
                                }
                                DesktopServiceState::Stopped => {
                                    enabled("start", true);
                                    enabled("stop", false);
                                    enabled("restart", false);
                                    blink = true;
                                }
                                _ => {
                                    enabled("start", false);
                                    enabled("stop", false);
                                    enabled("restart", false);
                                    blink = true;
                                }
                            }
                        }
                        _ => (),
                    }
                }
                Err(_) => break,
            }
        }
        let elapsed = now.elapsed().as_micros();
        if elapsed > 16666 {
            now = Instant::now();
            // println!("{}ms", elapsed as f64 * 0.001);
            let iteration = app.run_iteration();
            if iteration.window_count == 0 {
                break;
            }
            if blink {
                if span > 1000000 {
                    span = 0;
                    app.get_window("main")
                        .unwrap()
                        .set_title(title.as_str())
                        .unwrap_or_default();
                } else {
                    span += elapsed;
                    if span > 500000 {
                        app.get_window("main")
                            .unwrap()
                            .set_title(product)
                            .unwrap_or_default();
                    }
                }
            }
        } else {
            sleep(Duration::from_micros(999)).await;
        }
    }
}

#[tauri::command]
fn root() -> String {
    path().to_str().unwrap_or_default().to_owned()
}
