use std::{fs, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistMeta {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub sources: Vec<String>, // e.g., url, or "uploaded"
    pub tracks: Vec<String>,  // relative file names
}

impl PlaylistMeta {
    pub fn load(p: &Path) -> anyhow::Result<Self> {
        let s = fs::read_to_string(p)?;
        Ok(serde_json::from_str(&s)?)
    }

    pub async fn load_async(p: &Path) -> Option<Self> {
        tokio::fs::read_to_string(&p)
            .await
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    #[allow(dead_code)]
    pub fn save(&self, p: &Path) -> anyhow::Result<()> {
        fs::write(p, serde_json::to_vec_pretty(&self).unwrap())?;
        Ok(())
    }

    pub async fn save_async(&self, p: &Path) -> anyhow::Result<()> {
        tokio::fs::write(&p, serde_json::to_vec_pretty(&self).unwrap()).await?;
        Ok(())
    }

    pub fn dir_name(&self) -> String {
        // "2025-08-name_id"
        format!(
            "{}-{}_{:.8}",
            self.created_at.format("%Y-%m"),
            safe(&self.name),
            &self.id
        )
    }
}

fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub fn get_playlists(root: &Path) -> anyhow::Result<Vec<(String, PlaylistMeta)>> {
    let mut out = vec![];
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let meta = entry.path().join("playlist.json");
            if meta.exists()
                && let Ok(p) = PlaylistMeta::load(&meta)
            {
                out.push((entry.file_name().to_string_lossy().to_string(), p));
            }
        }
    }
    out.sort_by_key(|(_, m)| m.created_at);
    out.reverse();
    Ok(out)
}
