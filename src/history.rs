//! Browsing history, kept per Space like its website data: one file per
//! Space ID under `history/`, read the first time the address field needs
//! it. The Private Space never records anything (callers never ask for it).
//! An entry is a page address with its latest title, how often it was
//! visited and when last; that's enough for frecency ranking. Each Space keeps
//! at most `MAX_ENTRIES`, and nothing older than `MAX_AGE`.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const MAX_ENTRIES: usize = 5000;
/// Entries not visited for this long are dropped (180 days, in seconds).
pub const MAX_AGE: u64 = 180 * 24 * 60 * 60;
/// Longer addresses aren't worth suggesting and would bloat the file.
const MAX_URL: usize = 2048;
const MAX_TITLE: usize = 300;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    /// Unix seconds.
    pub last: u64,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    entries: Vec<Entry>,
}

struct Book {
    entries: Vec<Entry>,
    dirty: bool,
    /// False when the file existed but couldn't be read: this session's
    /// visits stay in memory and the original file is left alone.
    writable: bool,
}

pub struct History {
    dir: PathBuf,
    books: HashMap<[u8; 16], Book>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The address history keeps for a page: web pages only, without the
/// fragment (`#section` is the same page) or credentials.
pub fn key(url: &str) -> Option<String> {
    let mut url = url::Url::parse(url).ok()?;
    crate::model::validate_url(&url).ok()?;
    url.set_fragment(None);
    let url = String::from(url);
    (url.len() <= MAX_URL).then_some(url)
}

/// Visits weighted by how recent the last one was, as in Firefox's buckets.
pub fn frecency(entry: &Entry, now: u64) -> f64 {
    let days = now.saturating_sub(entry.last) / (24 * 60 * 60);
    let weight = match days {
        0..=4 => 100.,
        5..=14 => 70.,
        15..=31 => 50.,
        32..=90 => 30.,
        _ => 10.,
    };
    f64::from(entry.visits.max(1)) * weight
}

fn file_name(space: &[u8; 16]) -> String {
    format!("{}.json", uuid::Uuid::from_bytes(*space))
}

/// Drops entries past `MAX_AGE`, then the least recently visited beyond
/// `MAX_ENTRIES`.
fn prune(entries: &mut Vec<Entry>, now: u64) {
    entries.retain(|entry| now.saturating_sub(entry.last) <= MAX_AGE);
    if entries.len() > MAX_ENTRIES {
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.last));
        entries.truncate(MAX_ENTRIES);
    }
}

impl History {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.join("history"),
            books: HashMap::new(),
        }
    }

    fn book(&mut self, space: [u8; 16]) -> &mut Book {
        let path = self.dir.join(file_name(&space));
        self.books.entry(space).or_insert_with(|| {
            let read = || -> Result<Vec<Entry>> {
                if !path.exists() {
                    return Ok(Vec::new());
                }
                anyhow::ensure!(
                    fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
                    "History file is too large."
                );
                let saved: Saved = serde_json::from_slice(&fs::read(&path)?)?;
                anyhow::ensure!(saved.version == 1, "Unknown history version.");
                Ok(saved.entries)
            };
            match read() {
                Ok(mut entries) => {
                    entries.retain(|entry| key(&entry.url).as_ref() == Some(&entry.url));
                    prune(&mut entries, now());
                    Book {
                        entries,
                        dirty: false,
                        writable: true,
                    }
                }
                Err(error) => {
                    log::error!("History unavailable: {error}");
                    Book {
                        entries: Vec::new(),
                        dirty: false,
                        writable: false,
                    }
                }
            }
        })
    }

    pub fn entries(&mut self, space: [u8; 16]) -> &[Entry] {
        &self.book(space).entries
    }

    /// A page was shown in a Space. Written out by the next `flush`.
    pub fn visit(&mut self, space: [u8; 16], url: &str, title: &str, now: u64) {
        let Some(url) = key(url) else {
            return;
        };
        let book = self.book(space);
        let title: String = title.trim().chars().take(MAX_TITLE).collect();
        match book.entries.iter_mut().find(|entry| entry.url == url) {
            Some(entry) => {
                entry.visits = entry.visits.saturating_add(1);
                entry.last = now;
                if !title.is_empty() {
                    entry.title = title;
                }
            }
            None => {
                book.entries.push(Entry {
                    url,
                    title,
                    visits: 1,
                    last: now,
                });
                if book.entries.len() > MAX_ENTRIES {
                    prune(&mut book.entries, now);
                }
            }
        }
        book.dirty = true;
    }

    /// The page's title arrived (or changed) after its visit was recorded.
    pub fn retitle(&mut self, space: [u8; 16], url: &str, title: &str) {
        let (Some(url), title) = (key(url), title.trim()) else {
            return;
        };
        if title.is_empty() {
            return;
        }
        let book = self.book(space);
        if let Some(entry) = book.entries.iter_mut().find(|entry| entry.url == url)
            && entry.title != title
        {
            entry.title = title.chars().take(MAX_TITLE).collect();
            book.dirty = true;
        }
    }

    /// Forgets everything the Space visited, on disk too.
    pub fn clear(&mut self, space: [u8; 16]) -> Result<()> {
        let book = self.book(space);
        book.entries.clear();
        book.dirty = false;
        book.writable = true;
        self.remove_file(&space)
    }

    /// Forgets one page (History's remove button).
    pub fn remove(&mut self, space: [u8; 16], url: &str) {
        let book = self.book(space);
        let before = book.entries.len();
        book.entries.retain(|entry| entry.url != url);
        book.dirty |= book.entries.len() != before;
    }

    /// Forgets the pages last visited at or after `since` (Unix seconds).
    /// An entry keeps only its latest visit, so a page seen long ago and
    /// again today goes entirely, as in Safari.
    pub fn clear_since(&mut self, space: [u8; 16], since: u64) {
        let book = self.book(space);
        let before = book.entries.len();
        book.entries.retain(|entry| entry.last < since);
        book.dirty |= book.entries.len() != before;
    }

    /// The Space was deleted.
    pub fn forget_space(&mut self, space: [u8; 16]) -> Result<()> {
        self.books.remove(&space);
        self.remove_file(&space)
    }

    fn remove_file(&self, space: &[u8; 16]) -> Result<()> {
        match fs::remove_file(self.dir.join(file_name(space))) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// Writes every Space whose history changed since the last flush.
    pub fn flush(&mut self) -> Result<()> {
        let mut result = Ok(());
        for (space, book) in &mut self.books {
            if !book.dirty || !book.writable {
                continue;
            }
            let write = || -> Result<()> {
                fs::create_dir_all(&self.dir)?;
                let saved = Saved {
                    version: 1,
                    entries: book.entries.clone(),
                };
                let mut file = tempfile::NamedTempFile::new_in(&self.dir)?;
                file.write_all(&serde_json::to_vec(&saved)?)?;
                file.as_file().sync_all()?;
                file.persist(self.dir.join(file_name(space)))?;
                Ok(())
            };
            match write() {
                Ok(()) => book.dirty = false,
                Err(error) => result = Err(error),
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: [u8; 16] = [7; 16];
    const HOME: [u8; 16] = [9; 16];
    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn keys_are_web_pages_without_fragments() {
        assert_eq!(
            key("https://example.com/a#top").as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(
            key("http://localhost:5173").as_deref(),
            Some("http://localhost:5173/")
        );
        for never in [
            "file:///tmp/a.html",
            "browser://start",
            "about:blank",
            "https://user:pw@example.com/",
            "javascript:alert(1)",
        ] {
            assert_eq!(key(never), None, "{never}");
        }
        assert_eq!(
            key(&format!("https://a.example/{}", "x".repeat(MAX_URL))),
            None
        );
    }

    #[test]
    fn visits_count_per_space_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_800_000_000;
        let mut history = History::new(dir.path());
        history.visit(WORK, "https://github.com/", "GitHub", now);
        history.visit(WORK, "https://github.com/#readme", "", now + 5);
        history.visit(HOME, "https://news.example/", "News", now);
        history.retitle(WORK, "https://github.com/", "GitHub · Build software");
        history.flush().unwrap();
        let mut history = History::new(dir.path());
        let work = history.entries(WORK);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].visits, 2);
        assert_eq!(work[0].last, now + 5);
        assert_eq!(work[0].title, "GitHub · Build software");
        assert_eq!(history.entries(HOME)[0].url, "https://news.example/");
    }

    #[test]
    fn clearing_and_deleting_a_space_remove_its_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = History::new(dir.path());
        history.visit(WORK, "https://a.example/", "A", now());
        history.visit(HOME, "https://b.example/", "B", now());
        history.flush().unwrap();
        history.clear(WORK).unwrap();
        assert!(history.entries(WORK).is_empty());
        history.forget_space(HOME).unwrap();
        assert!(!dir.path().join("history").join(file_name(&HOME)).exists());
        let mut reopened = History::new(dir.path());
        assert!(reopened.entries(WORK).is_empty());
        assert!(reopened.entries(HOME).is_empty());
    }

    #[test]
    fn old_and_excess_entries_are_pruned() {
        let now = 1_800_000_000;
        let mut entries: Vec<Entry> = (0..MAX_ENTRIES as u64 + 10)
            .map(|index| Entry {
                url: format!("https://site{index}.example/"),
                title: String::new(),
                visits: 1,
                last: now - index,
            })
            .collect();
        entries.push(Entry {
            url: "https://ancient.example/".into(),
            title: String::new(),
            visits: 900,
            last: now - MAX_AGE - DAY,
        });
        prune(&mut entries, now);
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert!(
            entries
                .iter()
                .all(|entry| entry.url != "https://ancient.example/")
        );
        // The most recent survive.
        assert!(
            entries
                .iter()
                .any(|entry| entry.url == "https://site0.example/")
        );
    }

    #[test]
    fn recent_visits_outrank_old_ones() {
        let now = 1_800_000_000;
        let entry = |visits, days_ago: u64| Entry {
            url: "https://a.example/".into(),
            title: String::new(),
            visits,
            last: now - days_ago * DAY,
        };
        assert!(frecency(&entry(3, 1), now) > frecency(&entry(3, 40), now));
        assert!(frecency(&entry(10, 20), now) > frecency(&entry(2, 1), now));
    }

    #[test]
    fn an_unreadable_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history").join(file_name(&WORK));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"half written").unwrap();
        let mut history = History::new(dir.path());
        history.visit(WORK, "https://a.example/", "A", now());
        assert_eq!(history.entries(WORK).len(), 1);
        history.flush().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"half written");
    }

    #[test]
    fn single_pages_and_recent_ranges_can_be_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_800_000_000;
        let mut history = History::new(dir.path());
        history.visit(WORK, "https://old.example/", "Old", now - 3 * DAY);
        history.visit(WORK, "https://today.example/", "Today", now - 60);
        history.visit(WORK, "https://gone.example/", "Gone", now - 2 * DAY);
        history.visit(HOME, "https://today.example/", "Today", now - 60);
        history.remove(WORK, "https://gone.example/");
        history.clear_since(WORK, now - DAY);
        history.flush().unwrap();
        let mut reopened = History::new(dir.path());
        let urls: Vec<&str> = reopened
            .entries(WORK)
            .iter()
            .map(|entry| entry.url.as_str())
            .collect();
        assert_eq!(urls, ["https://old.example/"]);
        assert_eq!(reopened.entries(HOME).len(), 1, "other Spaces keep theirs");
    }
}
