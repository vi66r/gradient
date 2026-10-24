//! Page zoom remembered per site, as in Safari: ⌘+ and ⌘− step through
//! Safari's levels, and the level sticks to the site (host and port) across
//! its tabs and across launches. Private tabs start from the saved levels, but
//! their own changes last only for the session and never reach disk.
use anyhow::Result;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Safari's zoom levels.
const LEVELS: [f64; 11] = [0.5, 0.75, 0.85, 1., 1.15, 1.25, 1.5, 1.75, 2., 2.5, 3.];
/// Keeps a corrupt or hostile file from growing without bound in memory.
const MAX_SITES: usize = 10_000;

/// The next level up or down from `zoom`, clamped to the ends.
pub fn step(zoom: f64, larger: bool) -> f64 {
    let next = if larger {
        LEVELS.iter().find(|level| **level > zoom + 1e-3)
    } else {
        LEVELS.iter().rev().find(|level| **level < zoom - 1e-3)
    };
    next.copied()
        .unwrap_or(if larger { LEVELS[10] } else { LEVELS[0] })
}

pub fn is_actual_size(zoom: f64) -> bool {
    (zoom - 1.).abs() < 1e-3
}

/// The site a zoom level belongs to: host and port of a web page. Local files
/// and the start page have none and always open at actual size.
pub fn site(url: &str) -> Option<String> {
    crate::favicon::host_key(url)
}

pub struct Zooms {
    path: PathBuf,
    saved: HashMap<String, f64>,
    /// Private tabs' changes, kept only in memory.
    private: HashMap<String, f64>,
}

impl Zooms {
    /// A missing or unreadable file starts empty; the next change replaces it.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("zoom.json");
        let saved = fs::read(&path)
            .ok()
            .filter(|bytes| bytes.len() <= 1024 * 1024)
            .and_then(|bytes| serde_json::from_slice::<HashMap<String, f64>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, zoom)| LEVELS.iter().any(|level| (level - zoom).abs() < 1e-3))
            .filter(|(_, zoom)| !is_actual_size(*zoom))
            .take(MAX_SITES)
            .collect();
        Self {
            path,
            saved,
            private: HashMap::new(),
        }
    }

    pub fn get(&self, site: &str, private: bool) -> f64 {
        private
            .then(|| self.private.get(site))
            .flatten()
            .or_else(|| self.saved.get(site))
            .copied()
            .unwrap_or(1.)
    }

    /// Remembers `zoom` for `site`. Normal tabs write it to disk at once.
    pub fn set(&mut self, site: String, zoom: f64, private: bool) -> Result<()> {
        if private {
            self.private.insert(site, zoom);
            return Ok(());
        }
        if is_actual_size(zoom) {
            self.saved.remove(&site);
        } else if self.saved.len() < MAX_SITES || self.saved.contains_key(&site) {
            self.saved.insert(site, zoom);
        }
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&serde_json::to_vec_pretty(&self.saved)?)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_safaris_levels_and_stop_at_the_ends() {
        assert_eq!(step(1., true), 1.15);
        assert_eq!(step(1., false), 0.85);
        assert_eq!(step(3., true), 3.);
        assert_eq!(step(0.5, false), 0.5);
        // A level between steps (another browser's) snaps to the next one.
        assert_eq!(step(1.1, true), 1.15);
        assert_eq!(step(1.1, false), 1.);
    }

    #[test]
    fn sites_are_host_and_port_and_files_have_none() {
        assert_eq!(
            site("https://www.example.com/a?b").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            site("http://localhost:5173/").as_deref(),
            Some("localhost:5173")
        );
        assert_eq!(site("file:///tmp/a.html"), None);
        assert_eq!(site(crate::model::START), None);
    }

    #[test]
    fn private_changes_never_reach_disk_or_normal_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut zooms = Zooms::load(dir.path());
        zooms.set("example.com".into(), 1.25, false).unwrap();
        zooms.set("example.com".into(), 2., true).unwrap();
        zooms.set("private.test".into(), 1.5, true).unwrap();
        assert_eq!(zooms.get("example.com", false), 1.25);
        assert_eq!(zooms.get("example.com", true), 2.);
        assert_eq!(zooms.get("private.test", false), 1.);
        let reloaded = Zooms::load(dir.path());
        assert_eq!(reloaded.get("example.com", false), 1.25);
        assert_eq!(reloaded.get("example.com", true), 1.25);
        assert_eq!(reloaded.get("private.test", true), 1.);
        // Back to actual size forgets the site.
        zooms.set("example.com".into(), 1., false).unwrap();
        assert!(Zooms::load(dir.path()).saved.is_empty());
    }

    #[test]
    fn unknown_levels_in_the_file_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("zoom.json"),
            br#"{"a.test": 1.5, "b.test": 40, "c.test": -1, "d.test": 1}"#,
        )
        .unwrap();
        let zooms = Zooms::load(dir.path());
        assert_eq!(zooms.get("a.test", false), 1.5);
        assert_eq!(zooms.saved.len(), 1);
        fs::write(dir.path().join("zoom.json"), b"not json").unwrap();
        assert!(Zooms::load(dir.path()).saved.is_empty());
    }
}
