#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use async_std::{
    prelude::FutureExt,
    task::{spawn, spawn_local},
};
use crossbeam_channel::bounded;
use rustdesk_server::{
    usecase::{presenter, view, watcher},
    BUFFER,
};

#[async_std::main]
async fn main() {
    let buffer = BUFFER.get_or_init(|| 10).to_owned();
    let (view_sender, presenter_receiver) = bounded(buffer);
    let (presenter_sender, view_receiver) = bounded(buffer);
    spawn_local(view::create(presenter_sender.clone(), presenter_receiver))
        .join(spawn(presenter::create(view_sender, view_receiver)))
        .join(spawn(watcher::create(presenter_sender)))
        .await;
}
