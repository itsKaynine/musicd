mod api;
mod cli;
mod downloader;
mod job;
mod notifier;
mod player;
mod playlist;
mod publisher;
mod settings;
mod state;
mod utils;

use crate::{
    notifier::Notifier,
    player::PlayerConfig,
    settings::{DownloaderSettings, Settings},
};
use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Init .env
    dotenvy::dotenv().ok();

    // Init logging
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into());
    fmt().with_env_filter(filter).init();

    // Init CLI
    let cli = cli::Cli::parse();

    // Init settings
    let settings = Settings::load_or_init()?;
    let paths = settings.ensure_dirs()?;

    // Other commands use HTTP API and exit
    if let cli::Command::Start = cli.cmd {
    } else {
        return cli.cmd.run().await;
    }

    tracing::info!(
        "App environment: {}, Data dir: {}",
        settings.environment.as_str(),
        paths.root.display()
    );

    let notifier = Notifier::new();

    let kv = Arc::new(state::State::open(&paths.db)?);
    let player = player::PlayerHandle::new(
        notifier.clone(),
        PlayerConfig {
            auto_play: settings.player.auto_play,
            default_audio_effects: settings.player.default_audio_effects,
        },
    )?;

    // Job manager
    let job_manager = job::JobManager::new(notifier.clone(), &paths.jobs, settings.job.max_late_secs);
    job_manager.schedule_jobs();
    job_manager.watch();

    // Publisher
    let publisher = publisher::Publisher::new(
        paths.clone(),
        kv.clone(),
        notifier.clone(),
        player.clone(),
        settings.publish.clone(),
        settings.downloader.clone(),
    );

    // On boot, try to restore last playlist
    if let Some(id) = kv.get_current_playlist_id()? {
        if let Ok(items) = playlist::get_playlists(&paths.playlists)
            && let Some((folder, _meta)) = items.into_iter().find(|(_, m)| m.id == id)
        {
            player.set_playlist_dir(paths.playlists.join(folder), player::SetPlaylistMode::Queue);
        }
    } else {
        // Otherwise pick latest if exists
        if let Ok(items) = playlist::get_playlists(&paths.playlists)
            && let Some((folder, meta)) = items.into_iter().next()
        {
            kv.set_current_playlist_id(&meta.id).ok();
            player.set_playlist_dir(paths.playlists.join(folder), player::SetPlaylistMode::Queue);
        }
    }

    // Periodic (optional) manifest checker — if manifest url provided, and it indicates a new playlist,
    // your own service can return a JSON { "id": "...", "name": "...", "source_urls": "..." }.
    if settings.manifest.enable
        && let Some(url) = settings.manifest.url.clone()
    {
        let paths2 = paths.clone();
        let kv2 = kv.clone();
        let player2 = player.clone();
        let downloader_settings = settings.downloader.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) = check_manifest_once(&url, &paths2, &kv2, &player2, &downloader_settings).await {
                    tracing::warn!("manifest check failed: {error:#}");
                }
                tokio::time::sleep(Duration::from_secs(settings.manifest.check_interval_secs)).await;
            }
        });
    }

    // Web API
    let app = api::router(api::AppCtx {
        paths: paths.clone(),
        kv: kv.clone(),
        notifier: notifier.clone(),
        publisher: publisher.clone(),
        player: player.clone(),
        job_manager: job_manager.clone(),
    });

    let host = &settings.server.host;
    let port = settings.server.port;

    // Start server
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .unwrap_or_else(|_| panic!("Failed to parse address (host: {host}, port: {port})"));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("Listening on http://{addr}");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}

#[derive(serde::Deserialize)]
struct RemoteManifest {
    id: String,
    name: String,
    source_urls: Vec<String>,
}

async fn check_manifest_once(
    url: &str,
    paths: &settings::Paths,
    kv: &state::State,
    player: &player::PlayerHandle,
    downloader_settings: &DownloaderSettings,
) -> anyhow::Result<()> {
    let m: RemoteManifest = reqwest::get(url).await?.json().await?;
    // If id differs from current, fetch new
    if kv.get_current_playlist_id()? != Some(m.id.clone()) {
        use downloader::yt_dlp::YtDlpDownloader;
        use downloader::{Downloader, DownloaderKind};
        let dl: Box<dyn Downloader> = match DownloaderKind::YtDlp {
            DownloaderKind::YtDlp => Box::new(YtDlpDownloader),
        };
        let tmp_dir = paths.tmp.join(format!("remote_{}", m.id));
        tokio::fs::create_dir_all(&tmp_dir).await?;
        dl.download_playlist(&m.source_urls, &tmp_dir, downloader_settings)
            .await?;
        // fix meta
        let meta_path = tmp_dir.join("playlist.json");
        let mut meta: crate::playlist::PlaylistMeta = serde_json::from_slice(&tokio::fs::read(&meta_path).await?)?;
        meta.id = m.id.clone();
        meta.name = m.name.clone();
        tokio::fs::write(&meta_path, serde_json::to_vec_pretty(&meta)?).await?;
        let final_path = paths.playlists.join(meta.dir_name());
        tokio::fs::rename(&tmp_dir, &final_path).await?;
        kv.set_current_playlist_id(&meta.id)?;
        player.set_playlist_dir(final_path, player::SetPlaylistMode::Queue);
        tracing::info!("updated from manifest to '{}'", meta.name);
    }
    Ok(())
}
