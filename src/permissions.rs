//! Remembered site permissions, by profile and then by site (origin). A profile
//! is one WebKit data store, so this becomes per Space once Spaces land.
//! Private tabs never read or write it. Only "Allow" is remembered: "Don't
//! Allow" is the prompt's default answer, so a stray Return must not block a
//! site for good; it stays quiet until the page's next document instead.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Camera,
    Microphone,
    Screen,
    Location,
}

impl Kind {
    /// Screen sharing is granted per request, as in every other browser.
    pub fn rememberable(self) -> bool {
        self != Kind::Screen
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Camera => "camera",
            Kind::Microphone => "microphone",
            Kind::Screen => "screen",
            Kind::Location => "location",
        }
    }
}

/// The prompt's title for a request, naming the site.
pub fn question(site: &str, kinds: &[Kind]) -> String {
    let host = crate::model::display_host(site);
    match kinds {
        [Kind::Screen] => format!("Allow “{host}” to see your screen?"),
        [Kind::Camera, Kind::Microphone] => {
            format!("Allow “{host}” to use your camera and microphone?")
        }
        [kind] => format!("Allow “{host}” to use your {}?", kind.label()),
        _ => format!("Allow “{host}” to use your devices?"),
    }
}

/// A site's saved grants, for the reset prompt ("Camera, Location").
pub fn summary(kinds: &[Kind]) -> String {
    let mut names: Vec<String> = kinds.iter().map(|kind| kind.label().to_string()).collect();
    if let Some(first) = names.first_mut()
        && let Some(letter) = first.get(..1)
    {
        *first = letter.to_uppercase() + &first[1..];
    }
    names.join(", ")
}

/// The site a permission belongs to: the requesting origin, web only.
pub fn site(protocol: &str, host: &str, port: u16) -> Option<String> {
    if !matches!(protocol, "http" | "https") || host.is_empty() {
        return None;
    }
    let address = if port == 0 {
        format!("{protocol}://{host}/")
    } else {
        format!("{protocol}://{host}:{port}/")
    };
    let url = url::Url::parse(&address).ok()?;
    Some(url.origin().ascii_serialization())
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    version: u32,
    /// Profile UUID → site origin → granted kinds.
    profiles: BTreeMap<String, BTreeMap<String, Vec<Kind>>>,
}

pub struct Store {
    /// `None` when the saved file couldn't be read: grants then last only
    /// this session and the original file is preserved.
    path: Option<PathBuf>,
    saved: Saved,
}

fn profile_key(profile: &[u8; 16]) -> String {
    uuid::Uuid::from_bytes(*profile).to_string()
}

impl Store {
    pub fn open(dir: &std::path::Path) -> Self {
        let path = dir.join("permissions.json");
        let read = || -> Result<Saved> {
            if !path.exists() {
                return Ok(Saved {
                    version: 1,
                    ..Saved::default()
                });
            }
            anyhow::ensure!(
                std::fs::metadata(&path)?.len() <= 1024 * 1024,
                "Permissions file is too large."
            );
            let saved: Saved = serde_json::from_slice(&std::fs::read(&path)?)?;
            anyhow::ensure!(saved.version == 1, "Unknown permissions version.");
            Ok(saved)
        };
        match read() {
            Ok(saved) => Self {
                path: Some(path),
                saved,
            },
            Err(error) => {
                log::error!("Site permissions unavailable: {error}");
                Self {
                    path: None,
                    saved: Saved {
                        version: 1,
                        ..Saved::default()
                    },
                }
            }
        }
    }

    /// Whether every kind in the request is already allowed for this site.
    pub fn allowed(&self, profile: &[u8; 16], site: &str, kinds: &[Kind]) -> bool {
        let granted = self.site(profile, site);
        !kinds.is_empty() && kinds.iter().all(|kind| granted.contains(kind))
    }

    pub fn site(&self, profile: &[u8; 16], site: &str) -> Vec<Kind> {
        self.saved
            .profiles
            .get(&profile_key(profile))
            .and_then(|sites| sites.get(site))
            .cloned()
            .unwrap_or_default()
    }

    pub fn allow(&mut self, profile: &[u8; 16], site: &str, kinds: &[Kind]) -> Result<()> {
        let granted = self
            .saved
            .profiles
            .entry(profile_key(profile))
            .or_default()
            .entry(site.to_string())
            .or_default();
        granted.extend(kinds.iter().copied().filter(|kind| kind.rememberable()));
        granted.sort();
        granted.dedup();
        self.write()
    }

    pub fn forget(&mut self, profile: &[u8; 16], site: &str) -> Result<()> {
        let key = profile_key(profile);
        if let Some(sites) = self.saved.profiles.get_mut(&key) {
            sites.remove(site);
            if sites.is_empty() {
                self.saved.profiles.remove(&key);
            }
        }
        self.write()
    }

    fn write(&self) -> Result<()> {
        use std::io::Write;
        let Some(path) = &self.path else {
            anyhow::bail!("Saved site permissions are unavailable; this choice lasts until quit.");
        };
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&serde_json::to_vec_pretty(&self.saved)?)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: [u8; 16] = [7; 16];
    const OTHER: [u8; 16] = [9; 16];

    #[test]
    fn sites_are_web_origins() {
        assert_eq!(
            site("https", "meet.google.com", 0).as_deref(),
            Some("https://meet.google.com")
        );
        assert_eq!(
            site("https", "example.com", 443).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            site("http", "127.0.0.1", 8765).as_deref(),
            Some("http://127.0.0.1:8765")
        );
        assert_eq!(site("file", "", 0), None);
        assert_eq!(site("about", "blank", 0), None);
    }

    #[test]
    fn grants_persist_per_profile_and_site_and_can_be_reset() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        let meet = "https://meet.google.com";
        store
            .allow(&PROFILE, meet, &[Kind::Camera, Kind::Microphone])
            .unwrap();
        // Screen sharing is never remembered.
        store.allow(&PROFILE, meet, &[Kind::Screen]).unwrap();
        let store = Store::open(dir.path());
        assert!(store.allowed(&PROFILE, meet, &[Kind::Camera, Kind::Microphone]));
        assert!(store.allowed(&PROFILE, meet, &[Kind::Microphone]));
        assert!(!store.allowed(&PROFILE, meet, &[Kind::Screen]));
        assert!(!store.allowed(&PROFILE, meet, &[Kind::Location]));
        assert!(!store.allowed(&PROFILE, meet, &[]));
        assert!(!store.allowed(&OTHER, meet, &[Kind::Camera]));
        assert!(!store.allowed(&PROFILE, "https://evil.example", &[Kind::Camera]));
        let mut store = store;
        store.forget(&PROFILE, meet).unwrap();
        assert!(Store::open(dir.path()).site(&PROFILE, meet).is_empty());
    }

    #[test]
    fn unreadable_file_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("permissions.json");
        std::fs::write(&path, b"half written").unwrap();
        let mut store = Store::open(dir.path());
        assert!(
            store
                .allow(&PROFILE, "https://a.example", &[Kind::Camera])
                .is_err()
        );
        // The grant still holds for this session.
        assert!(store.allowed(&PROFILE, "https://a.example", &[Kind::Camera]));
        assert_eq!(std::fs::read(path).unwrap(), b"half written");
    }

    #[test]
    fn prompts_name_the_request() {
        assert_eq!(
            question("https://meet.google.com", &[Kind::Camera, Kind::Microphone]),
            "Allow “meet.google.com” to use your camera and microphone?"
        );
        assert_eq!(
            question("https://maps.example", &[Kind::Location]),
            "Allow “maps.example” to use your location?"
        );
        assert_eq!(summary(&[Kind::Camera, Kind::Location]), "Camera, location");
    }
}
