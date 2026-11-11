//! Native event wakeups plus bounded debounce and periodic reconciliation.
#[cfg(test)]
mod tests;
use kortex_core::{Error, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};
pub struct Events {
    receiver: Receiver<()>,
    _watcher: RecommendedWatcher,
}
impl Events {
    pub fn new(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let watched = root.clone();
        let (sender, receiver) = mpsc::sync_channel(128);
        let mut watcher = RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| {
                let relevant = match event {
                    Err(_) => true,
                    Ok(event) => {
                        event.paths.is_empty()
                            || event.paths.iter().any(|p| {
                                if !p.starts_with(&root)
                                    || p.starts_with(root.join(".kortex"))
                                    || p.starts_with(root.join(".git"))
                                {
                                    return false;
                                }
                                super::language(&p.to_string_lossy()).is_some()
                                    || p.file_name().is_some_and(|n| {
                                        [
                                            ".kortex.toml",
                                            ".gitignore",
                                            ".ignore",
                                            ".kortexignore",
                                            "go.mod",
                                            "Cargo.toml",
                                            "package.json",
                                            "tsconfig.json",
                                            "jsconfig.json",
                                            "pyproject.toml",
                                        ]
                                        .iter()
                                        .any(|name| n == *name)
                                    })
                                    || p.extension().is_none()
                            })
                    }
                };
                // A full queue already guarantees a wakeup/reconciliation; never allocate unbounded events.
                if relevant {
                    let _ = sender.try_send(());
                }
            },
            notify::Config::default().with_follow_symlinks(false),
        )
        .map_err(|e| Error::new("WATCH_ERROR", e.to_string(), 7))?;
        // Some backends open symlink entries even with recursive link traversal
        // disabled. Register only individually verified confined targets instead.
        for (count, entry) in ignore::WalkBuilder::new(&watched)
            .hidden(false)
            .follow_links(false)
            .max_depth(Some(64))
            .filter_entry(|entry| {
                entry.depth() == 0
                    || !matches!(
                        entry.file_name().to_str(),
                        Some(".git" | ".kortex" | "node_modules" | "target" | ".venv")
                    )
            })
            .build()
            .enumerate()
        {
            if count >= 100000 {
                return Err(Error::new(
                    "WATCH_LIMIT",
                    "native watch target scan limit reached",
                    7,
                ));
            }
            let entry = entry.map_err(|e| Error::new("WATCH_ERROR", e.to_string(), 7))?;
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
                continue;
            }
            let path = entry.path().canonicalize()?;
            if !path.starts_with(&watched) {
                continue;
            }
            if kind.is_dir()
                || super::language(&path.to_string_lossy()).is_some()
                || path.extension().is_none()
                || path
                    .extension()
                    .is_some_and(|ext| ext == "json" || ext == "toml")
            {
                watcher
                    .watch(&path, RecursiveMode::NonRecursive)
                    .map_err(|e| Error::new("WATCH_ERROR", e.to_string(), 7))?;
            }
        }
        Ok(Self {
            receiver,
            _watcher: watcher,
        })
    }
    pub fn wait(&self, debounce: Duration) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        match self.receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(()) => loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match self.receiver.recv_timeout(debounce.min(remaining)) {
                    Ok(()) => {}
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(Error::new("WATCH_ERROR", "watch channel disconnected", 7));
                    }
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Error::new("WATCH_ERROR", "watch channel disconnected", 7));
            }
        }
        // Even without an event, reconcile every two seconds for overflow/missed events.
        Ok(())
    }
}
