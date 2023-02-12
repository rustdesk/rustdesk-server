use std::{path::Path, time::Duration};

use super::Event;
use crate::path;
use async_std::task::{sleep, spawn_blocking};
use crossbeam_channel::{bounded, Sender};
use notify::{Config, RecommendedWatcher, RecursiveMode, Result, Watcher};

pub async fn create(sender: Sender<Event>) {
    loop {
        let watch_sender = sender.clone();
        match spawn_blocking(|| {
            watch(
                format!("{}/logs/", path().to_str().unwrap_or_default()),
                watch_sender,
            )
        })
        .await
        {
            Ok(_) => (),
            Err(e) => println!("error: {e}"),
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn watch<P: AsRef<Path>>(path: P, sender: Sender<Event>) -> Result<()> {
    let (tx, rx) = bounded(10);
    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
    watcher.watch(path.as_ref(), RecursiveMode::Recursive)?;
    for res in rx {
        let event = res?;
        for p in event.paths {
            let path = p
                .file_name()
                .unwrap_or_default()
                .to_str()
                .unwrap_or_default()
                .to_owned();
            if path.len() > 0 {
                sender.send(Event::FileChange(path)).unwrap_or_default();
            }
        }
    }
    Ok(())
}
