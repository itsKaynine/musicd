use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::settings::DownloaderSettings;

pub mod yt_dlp;

#[async_trait]
pub trait Downloader: Send + Sync {
    /// Download a playlist into dest dir atomically (write into tmp then rename).
    async fn download_playlist(
        &self,
        sources: &[String],
        dest_dir: &Path,
        settings: &DownloaderSettings,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum)]
pub enum DownloaderKind {
    #[serde(rename = "yt-dlp")]
    YtDlp,
}

impl DownloaderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DownloaderKind::YtDlp => "yt-dlp",
        }
    }
}

impl TryFrom<String> for DownloaderKind {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "yt-dlp" => Ok(Self::YtDlp),
            other => Err(format!("{} is not a supported downloader.", other)),
        }
    }
}
