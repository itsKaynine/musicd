use clap::{Parser, Subcommand};
use serde_json::json;

use crate::downloader::DownloaderKind;
use crate::player::SetPlaylistMode;

const DEFAULT_HOST: &str = "http://127.0.0.1:8371";

#[derive(Parser, Debug)]
#[command(name = "musicd")]
#[command(about = "Simple headless music player daemon for devs with web UI")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the daemon (web + player)
    Start,
    /// Print current status via HTTP API
    Status {
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Print jobs via HTTP API
    Jobs {
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Publish a playlist via HTTP API
    Publish {
        name: String,
        #[arg(long, value_enum, default_value_t = DownloaderKind::YtDlp)]
        downloader: DownloaderKind,
        #[arg(last = true)]
        source_urls: Vec<String>,
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Clean unused files and directories
    Clean {
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Play command via HTTP API
    Play {
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Pause command track via HTTP API
    Pause {
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Skip to previous track via HTTP API
    Prev {
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Skip to next track via HTTP API
    Next {
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Seek to position via HTTP API
    Seek {
        secs: u64,
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Set volume via HTTP API
    Volume {
        value: f32,
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Switch to a playlist id via HTTP API
    Playlist {
        id: String,
        #[arg(long, value_enum, default_value_t = SetPlaylistMode::Queue)]
        mode: SetPlaylistMode,
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
    /// Switch to a track index via HTTP API
    Track {
        idx: String,
        #[arg(long, default_value = DEFAULT_HOST)]
        host: String,
    },
}

impl Command {
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            Command::Start => Ok(()),
            Command::Status { host } => {
                let url = format!("{host}/status");
                let s = reqwest::get(url).await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Jobs { host } => {
                let url = format!("{host}/jobs");
                let s = reqwest::get(url).await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Publish {
                name,
                source_urls,
                downloader,
                host,
            } => {
                let url = format!("{host}/publish");
                let c = reqwest::Client::new();
                let b = json!({"name": name, "source_urls": source_urls, "downloader": downloader});
                let s = c.post(url).json(&b).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Clean { host } => {
                let url = format!("{host}/clean");
                let c = reqwest::Client::new();
                let s = c.post(url).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Play { host } => {
                let url = format!("{host}/control/play");
                let c = reqwest::Client::new();
                let s = c.post(url).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Pause { host } => {
                let url = format!("{host}/control/pause");
                let c = reqwest::Client::new();
                let s = c.post(url).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Prev { host } => {
                let url = format!("{host}/control/prev");
                let c = reqwest::Client::new();
                let s = c.post(url).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Next { host } => {
                let url = format!("{host}/control/next");
                let c = reqwest::Client::new();
                let s = c.post(url).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Seek { secs, host } => {
                let url = format!("{host}/control/seek");
                let c = reqwest::Client::new();
                let b = &json!({"secs": secs});
                let s = c.post(url).json(&b).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Volume { value, host } => {
                let url = format!("{host}/control/volume");
                let c = reqwest::Client::new();
                let b = json!({"value": value});
                let s = c.post(url).json(&b).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Playlist { id, mode, host } => {
                let url = format!("{host}/control/playlist/{id}");
                let c = reqwest::Client::new();
                let b = json!({"mode": mode});
                let s = c.post(url).json(&b).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
            Command::Track { idx, host } => {
                let url = format!("{host}/control/track/{idx}");
                let c = reqwest::Client::new();
                let s = c.post(url).send().await?.text().await?;
                println!("{s}");
                Ok(())
            }
        }
    }
}
