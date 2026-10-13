use notify::{
    Event, EventKind,
    event::{ModifyKind, RenameMode},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::enums::{FsEvent, PathKind};
use crate::utils::file_util;

/// Time window for matching rename events (from/to pairs)
///
/// When a file is renamed, the filesystem generates separate events for the source (from) and destination (to) paths.
/// This constant defines the maximum time interval between these events for them to be considered as part of the same rename operation.
const RENAME_TIME_WINDOW: Duration = Duration::from_millis(300);

#[derive(Debug)]
struct PendingFrom {
    path: PathBuf,
    at: Instant,
}

pub struct FsEventNormalizer {
    pending_from: VecDeque<PendingFrom>,
}

impl FsEventNormalizer {
    pub fn new() -> Self {
        Self {
            pending_from: VecDeque::new(),
        }
    }

    /// Input: notify::Event，Output: merged FsEvent
    pub fn handle(&mut self, event: Event) -> Vec<FsEvent> {
        let mut out = Vec::new();
        let now = Instant::now();

        // Clear expired [ From ] events
        // Rename downgrade to Remove if no [ To ] event is received within the time window
        while let Some(p) = self.pending_from.front() {
            if now.duration_since(p.at) > RENAME_TIME_WINDOW {
                if let Some(p) = self.pending_from.pop_front() {
                    let is_file = match file_util::guess_path_kind(
                        p.path.to_string_lossy().to_string().as_str(),
                    ) {
                        PathKind::File => true,
                        _ => false,
                    };
                    out.push(FsEvent::Remove {
                        path: p.path,
                        is_file: is_file,
                    });
                }
            } else {
                break;
            }
        }
        match event.kind {
            // rename（Both）
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                if event.paths.len() >= 2 {
                    out.push(FsEvent::Rename {
                        from: event.paths[0].clone(),
                        to: event.paths[1].clone(),
                    });
                }
            }

            // rename（From）
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                if let Some(path) = event.paths.get(0) {
                    self.pending_from.push_back(PendingFrom {
                        path: path.clone(),
                        at: now,
                    });
                }
            }

            // rename（To）
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                if let Some(to_path) = event.paths.get(0) {
                    if let Some(p) = self.pending_from.pop_front() {
                        out.push(FsEvent::Rename {
                            from: p.path,
                            to: to_path.clone(),
                        });
                    } else {
                        // Upgrade to Create if no [ From ] event
                        out.push(FsEvent::Create(to_path.clone()));
                    }
                }
            }

            // Create / Remove / Modify
            EventKind::Create(_) => {
                for p in event.paths {
                    out.push(FsEvent::Create(p));
                }
            }

            EventKind::Remove(remove_kind) => {
                log::debug!("remove_kind: {:?}", remove_kind);
                for p in event.paths {
                    let is_file = match remove_kind {
                        notify::event::RemoveKind::File => true,
                        _ => {
                            file_util::guess_path_kind(p.to_str().unwrap_or_default())
                                == PathKind::File
                        }
                    };
                    out.push(FsEvent::Remove { path: p, is_file });
                }
            }

            EventKind::Modify(_) => {
                for p in event.paths {
                    out.push(FsEvent::Modify(p));
                }
            }

            _ => {
                out.push(FsEvent::Other);
            }
        }

        out
    }
}
