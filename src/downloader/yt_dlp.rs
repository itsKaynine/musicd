use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::Context;
use async_trait::async_trait;
use tokio::{fs, process::Command};

use super::Downloader;

use crate::settings::DownloaderSettings;

pub struct YtDlpDownloader;

#[async_trait]
impl Downloader for YtDlpDownloader {
    async fn download_playlist(
        &self,
        sources: &[String],
        dest_dir: &Path,
        settings: &DownloaderSettings,
    ) -> anyhow::Result<()> {
        if sources.is_empty() {
            return Ok(());
        }

        // Find path to yt-dlp
        let yt_dlp_path = settings.yt_dlp.path.clone().unwrap_or("yt-dlp".into());

        // We assume yt-dlp is installed & in PATH.
        // Strategy: use yt-dlp to extract audio files into dest_dir_tmp,
        // then move atomically to dest_dir (rename directory).
        let tmp = dest_dir.with_extension("tmp");
        if tmp.exists() {
            fs::remove_dir_all(&tmp).await.ok();
        }
        fs::create_dir_all(&tmp).await?;

        for (i, source) in sources.iter().enumerate() {
            // 001-song.m4a, 002-001-playlist-song.m4a
            let template = "%(playlist_index|)03d%(playlist_index&-|)s%(title).80s.%(ext)s";
            let out_template = tmp.join(format!("{:03}-{}", i + 1, template));
            let out_template_str = out_template.to_string_lossy().to_string();

            // Download audio
            let status = Command::new(&yt_dlp_path)
                .arg("-x")
                .arg("--audio-format")
                .arg("m4a")
                .arg("--yes-playlist")
                .arg("--no-progress")
                .arg("-o")
                .arg(&out_template_str)
                .arg(source)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await
                .context(format!("failed to spawn yt-dlp from path: {:?}", yt_dlp_path))?;

            if !status.success() {
                tracing::warn!("yt-dlp failed with status {}", status);
            }
        }

        // Build playlist.json
        let mut tracks: Vec<String> = vec![];
        let mut rd = tokio::fs::read_dir(&tmp).await?;
        while let Some(e) = rd.next_entry().await? {
            if e.file_type().await?.is_file() {
                let p = e.path();
                if let Some(ext) = p.extension().and_then(|s| s.to_str())
                    && matches!(ext, "m4a" | "mp3" | "ogg" | "flac" | "wav" | "aac" | "opus")
                {
                    let name = p.file_name().unwrap().to_string_lossy().to_string();
                    tracks.push(name);
                }
            }
        }
        tracks.sort();

        if tracks.is_empty() {
            anyhow::bail!("no audio tracks were downloaded");
        }

        // We don't know the friendly name here; caller should rewrite playlist.json after move.
        let meta = serde_json::json!({
            "id": "TBD",
            "name": "TBD",
            "created_at": chrono::Utc::now(),
            "sources": sources,
            "tracks": tracks
        });
        tokio::fs::write(tmp.join("playlist.json"), serde_json::to_vec_pretty(&meta)?).await?;

        // Atomic move into place (ensure parent exists)
        if dest_dir.exists() {
            // Should not normally exist; but if it does, keep both.
            let backup = unique_path(dest_dir)?;
            tokio::fs::rename(dest_dir, &backup).await?;
        }
        tokio::fs::rename(&tmp, dest_dir).await?;

        Ok(())
    }
}

fn unique_path(p: &Path) -> anyhow::Result<PathBuf> {
    let mut i = 1;
    loop {
        let cand = p.with_extension(format!("old{}", i));
        if !cand.exists() {
            return Ok(cand);
        }
        i += 1;
        if i > 9999 {
            anyhow::bail!("too many old folders");
        }
    }
}
