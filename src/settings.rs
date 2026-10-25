//! App-wide preferences in `settings.json`, chosen from menus until a
//! Settings window exists. A missing or unreadable file means the defaults;
//! the next change replaces it.
use crate::model::SearchEngine;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// An engine this build doesn't know (a newer version's) reads as the
    /// default instead of discarding the file's other settings.
    #[serde(default, deserialize_with = "lenient")]
    pub search: SearchEngine,
}

fn lenient<'de, D: serde::Deserializer<'de>>(value: D) -> Result<SearchEngine, D::Error> {
    let value = serde_json::Value::deserialize(value)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

impl Settings {
    pub fn load(dir: &Path) -> Self {
        fs::read(path(dir))
            .ok()
            .filter(|bytes| bytes.len() <= 64 * 1024)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(path(dir))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_engine_persists_and_unknown_ones_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()).search, SearchEngine::DuckDuckGo);
        let settings = Settings {
            search: SearchEngine::Kagi,
        };
        settings.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), settings);
        fs::write(path(dir.path()), br#"{"search": "altavista"}"#).unwrap();
        assert_eq!(Settings::load(dir.path()).search, SearchEngine::DuckDuckGo);
        fs::write(path(dir.path()), b"not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }
}
