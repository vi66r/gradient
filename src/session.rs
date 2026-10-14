use crate::model::Session;
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
};

pub struct Store {
    dir: PathBuf,
    _lock: File,
    writable: bool,
}

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("BROWSER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/RafiBrowser")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("rafi-browser")
    }
}

impl Store {
    pub fn open(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(dir.join("session.lock"))?;
            lock.try_lock_exclusive().context("This Browser profile is already open. Use BROWSER_DATA_DIR for a separate profile.")?;
            Ok(Self {
                dir,
                _lock: lock,
                writable: true,
            })
        }
        #[cfg(not(unix))]
        anyhow::bail!("Only macOS and Linux are supported.")
    }

    pub fn load(&mut self) -> Result<Session> {
        let path = self.dir.join("session.json");
        if !path.exists() {
            return Ok(Session::default());
        }
        let result = (|| {
            anyhow::ensure!(
                fs::metadata(&path)?.len() <= 4 * 1024 * 1024,
                "Session file is too large."
            );
            let session: Session = serde_json::from_slice(&fs::read(&path)?)?;
            session.validate()?;
            Ok(session)
        })();
        if result.is_err() {
            self.writable = false;
        }
        result
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        anyhow::ensure!(
            self.writable,
            "Session recovery is unavailable; the original file is being preserved."
        );
        session.validate()?;
        let mut file = tempfile::NamedTempFile::new_in(&self.dir)?;
        file.write_all(&serde_json::to_vec_pretty(session)?)?;
        file.as_file().sync_all()?;
        file.persist(self.dir.join("session.json"))?;
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_exclusive_profile_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).unwrap();
        assert!(Store::open(dir.path().into()).is_err());
        let original = Session::default();
        store.save(&original).unwrap();
        drop(store);
        let mut reopened = Store::open(dir.path().into()).unwrap();
        assert_eq!(reopened.load().unwrap(), original);
    }
    #[test]
    fn corrupt_session_stays_byte_for_byte_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        fs::write(&path, b"unfinished snapshot").unwrap();
        let mut store = Store::open(dir.path().into()).unwrap();
        assert!(store.load().is_err());
        assert!(store.save(&Session::default()).is_err());
        assert_eq!(fs::read(path).unwrap(), b"unfinished snapshot");
    }
}
