use std::convert::{TryFrom, TryInto};
use std::{fs, path::PathBuf};

use crate::downloader::DownloaderKind;

/// The possible runtime environment for our application.
#[derive(serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Local,
    Test,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Local => "local",
            Environment::Test => "test",
        }
    }
}

impl TryFrom<String> for Environment {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "test" => Ok(Self::Test),
            other => Err(format!("{} is not a supported environment.", other)),
        }
    }
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Settings {
    /// The app environment.
    pub environment: Environment,
    /// Data root directory.
    pub data_dir: PathBuf,
    /// Server settings.
    pub server: ServerSettings,
    /// Manifest settings.
    pub manifest: ManifestSettings,
    /// Player settings.
    pub player: PlayerSettings,
    /// Publish settings.
    pub publish: PublishSettings,
    /// Job settings.
    pub job: JobSettings,
    /// Downloader settings.
    pub downloader: DownloaderSettings,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct ManifestSettings {
    /// Enable remote manifest fetching
    pub enable: bool,
    /// Optional remote manifest URL that can signal newer playlist to fetch.
    pub url: Option<String>,
    /// How often to check for new manifest/downloads (seconds).    
    pub check_interval_secs: u64,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct PlayerSettings {
    /// Auto play on start.
    pub auto_play: bool,
    /// Use default audio effects.
    pub default_audio_effects: bool,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct PublishSettings {
    /// Set playlist after publish.
    pub auto_set_playlist: bool,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct JobSettings {
    /// Number of seconds before expire.
    pub max_late_secs: u64,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct DownloaderSettings {
    /// Name of default downloader.
    pub default: DownloaderKind,
    /// Override path to yt-dlp.
    pub yt_dlp: YtDlpSettings,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct YtDlpSettings {
    /// Override path to yt-dlp.
    pub path: Option<PathBuf>,
}

impl Settings {
    pub fn load_or_init() -> anyhow::Result<Self> {
        // Detect the running environment.
        // Default to `local` if unspecified.
        let environment: Environment = std::env::var("MUSICD_ENVIRONMENT")
            .unwrap_or_else(|_| "local".into())
            .try_into()
            .expect("Failed to parse MUSICD_ENVIRONMENT");

        let mut base_path = std::env::current_dir().expect("Failed to determine the current directory");

        // Redirect path for test environment
        if environment == Environment::Test {
            base_path = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        }

        let environment_filename = format!("{}.json", environment.as_str());
        let settings = config::Config::builder()
            .set_default("environment", environment.as_str())?
            .set_default("data_dir", "./data")?
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 8371)?
            .set_default("manifest.enable", false)?
            .set_default("manifest.url", None::<Option<String>>)?
            .set_default("manifest.check_interval_secs", 900)?
            .set_default("player.auto_play", true)?
            .set_default("player.default_audio_effects", true)?
            .set_default("publish.auto_set_playlist", false)?
            .set_default("job.max_late_secs", 10)?
            .set_default("downloader.default", DownloaderKind::YtDlp.as_str())?
            .set_default("downloader.yt_dlp.path", "yt-dlp")?
            .add_source(config::File::from(base_path.join("settings.json")).required(false))
            .add_source(config::File::from(base_path.join(environment_filename)).required(false))
            .add_source(
                config::Environment::with_prefix("MUSICD")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?;

        let settings = settings.try_deserialize::<Self>()?;
        Ok(settings)
    }

    pub fn ensure_dirs(&self) -> anyhow::Result<Paths> {
        let root = self.data_dir.clone();
        let playlists = root.join("playlists");
        let tmp = root.join("tmp");
        let db = root.join("db");

        fs::create_dir_all(&playlists)?;
        fs::create_dir_all(&tmp)?;
        fs::create_dir_all(&db)?;

        let jobs = root.join("jobs.json");

        Ok(Paths {
            root,
            playlists,
            tmp,
            db,
            jobs,
        })
    }
}

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub playlists: PathBuf,
    pub tmp: PathBuf,
    pub db: PathBuf,
    pub jobs: PathBuf,
}
