use std::sync::Arc;

use tokio::fs;

use crate::downloader::yt_dlp::YtDlpDownloader;
use crate::downloader::{Downloader, DownloaderKind};
use crate::notifier::{Notification, Notifier};
use crate::player::{PlayerHandle, SetPlaylistMode};
use crate::playlist::PlaylistMeta;
use crate::settings::{DownloaderSettings, Paths, PublishSettings};
use crate::state::State as Kv;

#[derive(Clone)]
pub struct Publisher {
    pub paths: Paths,
    pub kv: Arc<Kv>,
    pub notifier: Notifier,
    pub player: PlayerHandle,
    pub publish_settings: PublishSettings,
    pub downloader_settings: DownloaderSettings,
}

impl Publisher {
    pub fn new(
        paths: Paths,
        kv: Arc<Kv>,
        notifier: Notifier,
        player: PlayerHandle,
        publish_settings: PublishSettings,
        downloader_settings: DownloaderSettings,
    ) -> Self {
        Self {
            paths,
            kv,
            notifier,
            player,
            publish_settings,
            downloader_settings,
        }
    }

    pub fn publish_in_background(&self, name: &str, source_urls: &[String], downloader_kind: Option<DownloaderKind>) {
        // Resolve downloader
        let downloader_kind = downloader_kind.unwrap_or(self.downloader_settings.default.clone());
        let downloader: Box<dyn Downloader> = match downloader_kind {
            DownloaderKind::YtDlp => Box::new(YtDlpDownloader),
        };

        // Temp dir for target; we’ll write to final folder after we have id/name
        let provisional_name = format!(
            "{}_{}",
            chrono::Utc::now().format("%Y%m%d%H%M%S"),
            name.replace(' ', "_")
        );

        // Will be renamed after meta is fixed
        let final_dir = self.paths.playlists.join(&provisional_name);

        tracing::info!(
            "Publishing playlist {} (downloader: {:?}) to {:?}",
            name,
            downloader_kind,
            final_dir
        );

        // Perform download in background (fire-and-forget)
        let name = name.to_string();
        let sources = source_urls.to_vec();
        let paths = self.paths.clone();
        let kv = self.kv.clone();
        let player = self.player.clone();
        let publish_settings = self.publish_settings.clone();
        let downloader_settings = self.downloader_settings.clone();
        let notifier = self.notifier.clone();
        tokio::spawn(async move {
            let tmp_dir = paths.tmp.join(&provisional_name);
            let _ = fs::remove_dir_all(&tmp_dir).await;
            let _ = fs::create_dir_all(&tmp_dir).await;

            let res = downloader
                .download_playlist(&sources, &tmp_dir, &downloader_settings)
                .await;
            if let Err(error) = res {
                tracing::error!("Download failed: {error:#}");
                let _ = fs::remove_dir_all(&tmp_dir).await;
                return;
            }

            // Fix playlist.json with id/name and move atomically into playlists/
            let meta_path = tmp_dir.join("playlist.json");
            let mut meta = match PlaylistMeta::load_async(&meta_path).await {
                Some(m) => m,
                None => {
                    tracing::error!("Missing playlist.json");
                    let _ = fs::remove_dir_all(&tmp_dir).await;
                    return;
                }
            };
            meta.id = uuid::Uuid::new_v4().to_string();
            meta.name = name.clone();
            meta.sources = sources.clone();
            if let Err(error) = meta.save_async(&meta_path).await {
                tracing::error!("Write meta failed: {error:#}");
            }

            let final_folder = meta.dir_name();
            let final_path = paths.playlists.join(&final_folder);
            if let Err(error) = tokio::fs::rename(&tmp_dir, &final_path).await {
                tracing::error!("Rename final failed: {error:#}");
                return;
            }

            tracing::info!("Published playlist '{}'", meta.name);

            // Notify
            notifier.notify(Notification::PlaylistPublished {
                id: meta.id.clone(),
                name: meta.name.clone(),
            });

            // Switch current to the new playlist
            if publish_settings.auto_set_playlist {
                tracing::info!("Setting playlist after publish");

                if let Err(error) = kv.set_current_playlist_id(&meta.id) {
                    tracing::warn!("Set current playlist failed: {error:#}");
                }
                player.set_playlist_dir(&final_path, SetPlaylistMode::Queue);
            }
        });
    }
}
