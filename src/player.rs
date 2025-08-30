use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};

use rodio::{OutputStreamBuilder, Sink, Source, decoder::DecoderBuilder, source::LimitSettings};
use serde::{Deserialize, Serialize};

use crate::notifier::{Notification, Notifier};
use crate::playlist::PlaylistMeta;

#[derive(Clone)]
pub struct PlayerHandle {
    inner: Arc<PlayerInner>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerConfig {
    pub auto_play: bool,
    pub default_audio_effects: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum)]
pub enum SetPlaylistMode {
    #[serde(rename = "queue")]
    Queue,
    #[serde(rename = "skip")]
    Skip,
}

enum PlayerCommand {
    Play,
    Pause,
    Prev,
    Next,
    Seek(u64),
    SetVolume(f32),
    SetIndex(usize),
}

struct PlayerInner {
    /// Path to active playlist dir
    playlist_dir: RwLock<Option<PathBuf>>,
    /// Current index state exposed for status
    status: Mutex<PlayerStatus>,
    /// Signal channels
    tx: crossbeam_channel::Sender<PlayerCommand>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerStatus {
    pub playlist_id: Option<String>,
    pub playlist_name: Option<String>,
    pub current_index: usize,
    pub current_track: Option<String>,
    pub current_pos: Option<Duration>,
    pub total_duration: Option<Duration>,
    pub is_paused: Option<bool>,
    pub volume: Option<f32>,
}

const RETRY_DURATION_S: u64 = 2;
const TICK_DURATION_MS: u64 = 100;
const POSITION_UPDATE_DURATION_MS: u64 = 500;

impl PlayerHandle {
    pub fn new(notifier: Notifier, config: PlayerConfig) -> anyhow::Result<Self> {
        let (_tx, _rx) = crossbeam_channel::unbounded::<PlayerCommand>();
        let inner = Arc::new(PlayerInner {
            playlist_dir: RwLock::new(None),
            status: Mutex::new(PlayerStatus::default()),
            tx: _tx.clone(),
        });

        let self_inner = inner.clone();
        thread::Builder::new().name("musicd-player".into()).spawn(move || {
            // Audio stream owns OS device; keep it inside the thread.
            let stream_handle = match OutputStreamBuilder::open_default_stream() {
                Ok(v) => v,
                Err(error) => {
                    eprintln!("Audio init error: {error:?}");
                    return;
                }
            };

            // Cache durations
            let retry_duration = Duration::from_secs(RETRY_DURATION_S);
            let position_update_duration = Duration::from_millis(POSITION_UPDATE_DURATION_MS);
            let tick_duration = Duration::from_millis(TICK_DURATION_MS);

            loop {
                // Reload playlist dir
                let pdir = {
                    match self_inner.playlist_dir.try_read() {
                        Ok(dir) => dir.clone(),
                        Err(error) => {
                            tracing::warn!("Failed to obtain playlist_dir lock: {:?}", error);
                            None
                        }
                    }
                };
                if let Some(dir) = pdir {
                    // Load meta
                    let meta_path = dir.join("playlist.json");
                    let meta = match std::fs::read_to_string(&meta_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<PlaylistMeta>(&s).ok())
                    {
                        Some(m) => m,
                        None => {
                            thread::sleep(retry_duration);
                            continue;
                        }
                    };

                    // Notify
                    notifier.notify(Notification::PlaylistChanged {
                        id: meta.id.clone(),
                        name: meta.name.clone(),
                    });

                    let mut idx = {
                        match self_inner.status.try_lock() {
                            Ok(mut s) => {
                                s.playlist_id = Some(meta.id.clone());
                                s.playlist_name = Some(meta.name.clone());
                                s.current_index = 0;
                                s.current_track = None;
                                s.current_pos = None;
                                s.total_duration = None;
                                s.is_paused = None;
                                s.volume = None;
                                s.current_index
                            }
                            Err(error) => {
                                tracing::warn!("Failed to obtain status lock: {:?}", error);
                                thread::sleep(retry_duration);
                                continue;
                            }
                        }
                    };

                    // Wait for retry if empty
                    if meta.tracks.is_empty() {
                        thread::sleep(retry_duration);
                        continue;
                    }

                    loop {
                        // Loop to first track
                        if idx >= meta.tracks.len() {
                            idx = 0;
                        }

                        let track = &meta.tracks[idx];
                        {
                            // Notify
                            notifier.notify(Notification::TrackChanged {
                                idx,
                                name: track.to_string(),
                            });

                            match self_inner.status.try_lock() {
                                Ok(mut s) => {
                                    s.current_index = idx;
                                    s.current_track = Some(track.clone());
                                }
                                Err(error) => {
                                    tracing::warn!("Failed to obtain status lock: {:?}", error);
                                    thread::sleep(retry_duration);
                                    continue;
                                }
                            }
                        }

                        let fp = dir.join(track);
                        let sink = Sink::connect_new(stream_handle.mixer());
                        if let Ok(file) = File::open(&fp)
                            && let Ok(source) = DecoderBuilder::new()
                                .with_data(BufReader::new(file))
                                .with_seekable(true)
                                .build()
                        {
                            match self_inner.status.try_lock() {
                                Ok(mut s) => {
                                    s.total_duration = source.total_duration();
                                }
                                Err(error) => {
                                    tracing::warn!("Failed to obtain status lock: {:?}", error);
                                    thread::sleep(retry_duration);
                                    continue;
                                }
                            }

                            // Notify
                            notifier.notify(Notification::TrackDurationChanged {
                                duration: source.total_duration(),
                            });

                            // Audio effects
                            if config.default_audio_effects {
                                let limit_settings = LimitSettings::default()
                                    .with_threshold(-1.0) // Higher threshold (less limiting)
                                    .with_knee_width(8.0) // Wide knee (softer)
                                    .with_attack(Duration::from_millis(20)) // Slower attack
                                    .with_release(Duration::from_millis(200)); // Slower release                            
                                let mixed_source =
                                    source.automatic_gain_control(1.0, 4.0, 0.1, 5.0).limit(limit_settings);
                                sink.append(mixed_source);
                            } else {
                                sink.append(source);
                            }

                            // Auto play
                            if !config.auto_play {
                                sink.pause();

                                // Notify
                                notifier.notify(Notification::Paused);
                            }
                        }

                        // Keep track of position updates for notification
                        let mut last_position_update_time = Instant::now();

                        // Ticks - Wait for end or skip signal
                        loop {
                            match self_inner.status.try_lock() {
                                Ok(mut s) => {
                                    s.current_pos = Some(sink.get_pos());
                                    s.is_paused = Some(sink.is_paused());
                                    s.volume = Some(sink.volume());
                                }
                                Err(error) => {
                                    tracing::warn!("Failed to obtain status lock: {:?}", error);
                                    thread::sleep(retry_duration);
                                    continue;
                                }
                            }

                            if last_position_update_time.elapsed() >= position_update_duration {
                                // Notify
                                notifier.notify(Notification::SeekPositionChanged {
                                    duration: sink.get_pos(),
                                });

                                // Update last update time
                                last_position_update_time = Instant::now();
                            }

                            // End
                            if sink.empty() {
                                tracing::info!("Seek empty");
                                idx += 1;
                                break;
                            }

                            // Commands
                            match _rx.try_recv() {
                                Ok(PlayerCommand::Play) => {
                                    tracing::info!("Play");
                                    sink.play();

                                    // Notify
                                    notifier.notify(Notification::Played);
                                }
                                Ok(PlayerCommand::Pause) => {
                                    tracing::info!("Pause");
                                    sink.pause();

                                    // Notify
                                    notifier.notify(Notification::Paused);
                                }
                                Ok(PlayerCommand::Seek(secs)) => {
                                    let duration = Duration::from_secs(secs);
                                    match sink.try_seek(duration) {
                                        Ok(()) => {
                                            tracing::info!("Seek to position: {:?}", secs);

                                            // Notify
                                            notifier.notify(Notification::SeekPositionChanged { duration });
                                        }
                                        Err(error) => tracing::warn!("Seek error: {:?}", error),
                                    }
                                }
                                Ok(PlayerCommand::Prev) => {
                                    tracing::info!("Prev");

                                    if idx == 0 {
                                        idx = meta.tracks.len() - 1;
                                    } else {
                                        idx -= 1;
                                    }
                                    sink.stop();
                                    break;
                                }
                                Ok(PlayerCommand::Next) => {
                                    tracing::info!("Next");

                                    idx += 1;
                                    sink.stop();
                                    break;
                                }
                                Ok(PlayerCommand::SetVolume(value)) => {
                                    let value = value.clamp(0.0, 1.0);
                                    tracing::info!("Volume: {:?}", value);
                                    sink.set_volume(value);

                                    // Notify
                                    notifier.notify(Notification::VolumeChanged { value });
                                }
                                Ok(PlayerCommand::SetIndex(index)) => {
                                    tracing::info!("Set Index: {:?}", index);
                                    if index != idx {
                                        idx = index;
                                        sink.stop();
                                        break;
                                    }
                                }
                                Err(error) => match error {
                                    crossbeam_channel::TryRecvError::Empty => {}
                                    _ => tracing::warn!("Player command channel recv error: {:?}", error),
                                },
                            }

                            thread::sleep(tick_duration);
                        }

                        // Check if playlist changed
                        let now_dir = {
                            match self_inner.playlist_dir.try_read() {
                                Ok(dir) => dir.clone(),
                                Err(error) => {
                                    tracing::warn!("Failed to obtain playlist_dir lock: {:?}", error);
                                    None
                                }
                            }
                        };
                        if now_dir.as_deref() != Some(&dir) {
                            // Reload
                            break;
                        }
                    }
                } else {
                    thread::sleep(retry_duration);
                }
            }
        })?;

        Ok(Self { inner })
    }

    pub fn set_playlist_dir(&self, p: impl AsRef<Path>, mode: SetPlaylistMode) {
        if let Ok(mut dir) = self.inner.playlist_dir.try_write() {
            *dir = Some(p.as_ref().to_path_buf());

            match mode {
                SetPlaylistMode::Queue => {}
                SetPlaylistMode::Skip => {
                    self.next();
                }
            };
        }
    }

    pub fn status(&self) -> anyhow::Result<PlayerStatus> {
        match self.inner.status.try_lock() {
            Ok(s) => Ok(s.clone()),
            Err(error) => {
                tracing::warn!("Failed to obtain status lock: {:?}", error);
                Err(anyhow::anyhow!("Failed to obtain status lock: {:?}", error))
            }
        }
    }

    pub fn play(&self) {
        let _ = self.inner.tx.send(PlayerCommand::Play);
    }

    pub fn pause(&self) {
        let _ = self.inner.tx.send(PlayerCommand::Pause);
    }

    pub fn prev(&self) {
        let _ = self.inner.tx.send(PlayerCommand::Prev);
    }

    pub fn next(&self) {
        let _ = self.inner.tx.send(PlayerCommand::Next);
    }

    pub fn seek(&self, secs: u64) {
        let _ = self.inner.tx.send(PlayerCommand::Seek(secs));
    }

    pub fn set_volume(&self, value: f32) {
        let _ = self.inner.tx.send(PlayerCommand::SetVolume(value));
    }

    pub fn set_index(&self, index: usize) {
        let _ = self.inner.tx.send(PlayerCommand::SetIndex(index));
    }
}
