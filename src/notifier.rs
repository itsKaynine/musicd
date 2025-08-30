use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Notification {
    Played,
    Paused,
    TrackChanged { idx: usize, name: String },
    TrackDurationChanged { duration: Option<Duration> },
    PlaylistChanged { id: String, name: String },
    PlaylistPublished { id: String },
    SeekPositionChanged { duration: Duration },
    VolumeChanged { value: f32 },
    JobsUpdated,
    RunningJob { id: String },
}

/// Wrapper around a broadcast channel
#[derive(Clone)]
pub struct Notifier {
    pub tx: broadcast::Sender<Notification>,
}

impl Notifier {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(1000);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Notification> {
        self.tx.subscribe()
    }

    pub fn notify(&self, notification: Notification) {
        // Ignore error if there are no active subscribers
        let _ = self.tx.send(notification);
    }
}
