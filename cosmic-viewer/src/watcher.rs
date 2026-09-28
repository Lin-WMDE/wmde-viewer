// SPDX-License-Identifier: GPL-3.0-only

use cosmic::iced::Subscription;
use cosmic::iced::futures::SinkExt;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum WatcherEvent {
    Created(PathBuf),
    Removed(PathBuf),
    Modified(PathBuf),
    Error(String),
}

// WMDE: every directory the album draws on, not one: an album of files given on the command
// line may span several, and a picture in a directory nobody watched went stale unseen.
pub fn watch_directories(dirs: Vec<PathBuf>) -> Subscription<WatcherEvent> {
    Subscription::run_with(dirs, |dirs| {
        cosmic::iced::stream::channel(100, {
            let dirs = dirs.clone();
            move |mut output: cosmic::iced::futures::channel::mpsc::Sender<WatcherEvent>| async move {
                if dirs.is_empty() {
                    std::future::pending::<()>().await;
                    unreachable!()
                }

                let (tx, mut rx) = mpsc::channel(100);

                let watcher_result = RecommendedWatcher::new(
                    move |res: Result<Event, notify::Error>| {
                        let _ = tx.blocking_send(res);
                    },
                    Config::default(),
                );

                let mut watcher = match watcher_result {
                    Ok(w) => w,
                    Err(e) => {
                        let _ = output.send(WatcherEvent::Error(e.to_string())).await;
                        std::future::pending::<()>().await;
                        unreachable!();
                    }
                };

                // A directory that cannot be watched is reported; the others still are.
                for dir in &dirs {
                    if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                        let _ = output
                            .send(WatcherEvent::Error(format!("{}: {e}", dir.display())))
                            .await;
                    }
                }

                while let Some(result) = rx.recv().await {
                    match result {
                        Ok(event) => {
                            for path in event.paths {
                                let msg = match event.kind {
                                    EventKind::Create(_) => Some(WatcherEvent::Created(path)),
                                    EventKind::Remove(_) => Some(WatcherEvent::Removed(path)),
                                    EventKind::Modify(_) => Some(WatcherEvent::Modified(path)),
                                    _ => None,
                                };

                                if let Some(msg) = msg {
                                    let _ = output.send(msg).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = output.send(WatcherEvent::Error(e.to_string())).await;
                        }
                    }
                }

                std::future::pending::<()>().await;
                unreachable!()
            }
        })
    })
}
