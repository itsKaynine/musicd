use std::path::Path;

use serde::{Deserialize, Serialize};
use sled::Db;

const KEY_CURRENT: &str = "current_playlist_id";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentStatus {
    pub playlist_id: Option<String>,
    pub track_path: Option<String>,
    pub track_index: usize,
}

pub struct State {
    db: Db,
}

impl State {
    pub fn open<P: AsRef<Path>>(p: P) -> anyhow::Result<Self> {
        Ok(Self { db: sled::open(p)? })
    }

    pub fn get_current_playlist_id(&self) -> anyhow::Result<Option<String>> {
        Ok(self
            .db
            .get(KEY_CURRENT)?
            .and_then(|ivec| String::from_utf8(ivec.to_vec()).ok()))
    }

    pub fn set_current_playlist_id(&self, id: &str) -> anyhow::Result<()> {
        self.db.insert(KEY_CURRENT, id.as_bytes())?;
        self.db.flush()?;
        Ok(())
    }
}
