//! Config file watcher for hot reload.

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// Config file change events.
#[derive(Debug)]
pub enum ConfigEvent {
    /// Config file was modified.
    Modified,
    /// Error watching config file.
    Error(String),
}

/// Watches the config file for changes.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    receiver: Receiver<Result<Event, notify::Error>>,
    config_path: PathBuf,
}

impl ConfigWatcher {
    /// Create a new config watcher for the given path.
    pub fn new(config_path: PathBuf) -> Result<Self, notify::Error> {
        let (tx, rx) = channel();

        let config = Config::default()
            .with_poll_interval(Duration::from_secs(2));

        let mut watcher = RecommendedWatcher::new(tx, config)?;

        // Watch the config file's parent directory
        if let Some(parent) = config_path.parent() {
            if parent.exists() {
                watcher.watch(parent, RecursiveMode::NonRecursive)?;
                tracing::info!("Watching config directory: {:?}", parent);
            } else {
                tracing::warn!("Config directory does not exist: {:?}", parent);
            }
        }

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
            config_path,
        })
    }

    /// Poll for config file changes (non-blocking).
    pub fn poll(&self) -> Option<ConfigEvent> {
        match self.receiver.try_recv() {
            Ok(Ok(event)) => {
                // Check if the event is for our config file
                let is_config_file = event.paths.iter().any(|p| {
                    p.file_name() == self.config_path.file_name()
                });

                if is_config_file {
                    use notify::EventKind;
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            tracing::debug!("Config file changed: {:?}", event);
                            Some(ConfigEvent::Modified)
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("Config watch error: {}", e);
                Some(ConfigEvent::Error(e.to_string()))
            }
            Err(_) => None, // No events
        }
    }

    /// Get the watched config path.
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }
}
