//! Downloads land in ~/Downloads under a name no existing file, or download in
//! flight, already has. The list of downloads lives in memory only.
use std::path::{Path, PathBuf};

/// Where downloads are saved. `GRADIENT_DOWNLOADS_DIR` points a disposable
/// profile somewhere else, like `GRADIENT_DATA_DIR`.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GRADIENT_DOWNLOADS_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Downloads")
}

/// A plain file name from the server's or page's suggestion: no folders, no
/// hidden files, no control characters, short enough for any volume.
pub fn file_name(suggested: &str) -> String {
    let name: String = suggested
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim().trim_start_matches('.').trim();
    if name.is_empty() {
        return "download".into();
    }
    const MAX: usize = 200;
    if name.len() <= MAX {
        return name.into();
    }
    let (stem, extension) = split(name);
    let extension = if extension.len() < MAX / 2 {
        extension
    } else {
        ""
    };
    let mut end = MAX - extension.len();
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{extension}", &stem[..end])
}

/// Splits off the extension, keeping compound archive extensions whole.
fn split(name: &str) -> (&str, &str) {
    let lower = name.to_ascii_lowercase();
    for compound in [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"] {
        if lower.len() > compound.len() && lower.ends_with(compound) {
            return name.split_at(name.len() - compound.len());
        }
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `name` in `dir`, or `name (1)`, `name (2)`… before the extension, for the
/// first that isn't `taken`. Never overwrites.
pub fn unique(dir: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let (stem, extension) = split(name);
    let mut path = dir.join(name);
    let mut counter = 1;
    while taken(&path) {
        path = dir.join(format!("{stem} ({counter}){extension}"));
        counter += 1;
    }
    path
}

/// Whether a `Content-Disposition` header asks for the response to be saved
/// rather than shown.
pub fn is_attachment(disposition: &str) -> bool {
    disposition
        .split(';')
        .next()
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
}

/// Whether anything, even a broken symlink, already has this path.
pub fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Bytes received, and the total when the server sent a length.
    Active {
        received: i64,
        total: Option<i64>,
    },
    Done,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Download {
    pub id: u64,
    /// The tab it started in; a private tab's downloads are forgotten with it.
    pub tab: u64,
    pub private: bool,
    pub name: String,
    pub path: PathBuf,
    pub state: State,
}

impl Download {
    pub fn active(&self) -> bool {
        matches!(self.state, State::Active { .. })
    }

    /// Fraction complete, when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        match self.state {
            State::Active {
                received,
                total: Some(total),
            } if total > 0 => Some((received as f64 / total as f64).clamp(0., 1.) as f32),
            State::Done => Some(1.),
            _ => None,
        }
    }

    /// The short note beside the name.
    pub fn status(&self) -> String {
        match self.state {
            State::Active { .. } if self.fraction().is_some() => {
                format!("{}%", (self.fraction().unwrap_or(0.) * 100.).floor())
            }
            State::Active { received, .. } => size(received),
            State::Done => String::new(),
            State::Failed => "Failed".into(),
        }
    }
}

/// A byte count as Finder shows it (decimal units).
pub fn size(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    match bytes {
        b if b < 1e3 => format!("{b} B"),
        b if b < 1e6 => format!("{:.0} KB", b / 1e3),
        b if b < 1e9 => format!("{:.1} MB", b / 1e6),
        b => format!("{:.2} GB", b / 1e9),
    }
}

impl crate::Browser {
    pub(crate) fn download_started(
        &mut self,
        id: u64,
        tab: u64,
        name: String,
        path: PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        let popup = tab;
        let tab = self.popups.get(&popup).copied().unwrap_or(popup);
        // A popup that closed to hand its download back is done with.
        if !self.all_tabs().any(|candidate| candidate.id == popup) {
            self.popups.remove(&popup);
        }
        // A tab that's already gone can't vouch that it was normal.
        let private = self
            .all_tabs()
            .find(|candidate| candidate.id == tab)
            .is_none_or(|tab| tab.private);
        self.downloads.insert(
            0,
            Download {
                id,
                tab,
                private,
                name,
                path,
                state: State::Active {
                    received: 0,
                    total: None,
                },
            },
        );
        // Keep the list short: older finished rows fall off.
        while self.downloads.len() > 5 {
            match self
                .downloads
                .iter()
                .rposition(|download| !download.active())
            {
                Some(oldest) => self.downloads.remove(oldest),
                None => break,
            };
        }
        self.poll_downloads(cx);
    }

    pub(crate) fn download_ended(&mut self, id: u64, ok: bool) {
        if let Some(download) = self.downloads.iter_mut().find(|download| download.id == id) {
            download.state = if ok { State::Done } else { State::Failed };
        }
        self.prune_downloads();
    }

    /// Refreshes progress while anything downloads, then stops.
    fn poll_downloads(&mut self, cx: &mut gpui::Context<Self>) {
        if self.downloads_polling {
            return;
        }
        self.downloads_polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let more = this
                    .update(cx, |this, cx| {
                        for download in &mut this.downloads {
                            if download.active()
                                && let Some((received, total)) =
                                    crate::engine::download_progress(download.id)
                            {
                                download.state = State::Active { received, total };
                            }
                        }
                        cx.notify();
                        this.downloads_polling = this.downloads.iter().any(Download::active);
                        this.downloads_polling
                    })
                    .unwrap_or(false);
                if !more {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn cancel_download(&mut self, id: u64) {
        crate::engine::cancel_download(id);
        self.downloads.retain(|download| download.id != id);
    }

    pub(crate) fn dismiss_download(&mut self, id: u64) {
        self.downloads
            .retain(|download| download.id != id || download.active());
    }

    pub(crate) fn reveal_download(&mut self, id: u64) {
        let Some(download) = self.downloads.iter().find(|download| download.id == id) else {
            return;
        };
        if exists(&download.path) {
            crate::engine::reveal(&download.path);
        } else {
            self.notice = Some(format!("“{}” is no longer in its folder.", download.name));
        }
    }

    /// A private tab's downloads are forgotten once it closes. One still in
    /// flight keeps its row (and Cancel) until it ends.
    pub(crate) fn prune_downloads(&mut self) {
        let tabs: Vec<u64> = self.all_tabs().map(|tab| tab.id).collect();
        self.downloads.retain(|download| {
            !download.private || download.active() || tabs.contains(&download.tab)
        });
        self.popups.retain(|popup, _| tabs.contains(popup));
    }

    /// A main-frame load became a download. A tab opened just for it has
    /// nothing to show: a page's popup closes, anything else returns to the
    /// start page instead of reporting a failed load.
    pub(crate) fn page_became_download(
        &mut self,
        id: u64,
        empty: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs[index].loading = false;
        if !empty {
            return;
        }
        if let Some(&opener) = self.popups.get(&id)
            && self.tabs.len() > 1
        {
            for download in self
                .downloads
                .iter_mut()
                .filter(|download| download.tab == id)
            {
                download.tab = opener;
            }
            let private = self.tabs[index].private;
            self.close_unchecked(index, window, cx);
            // It never showed a page; there's nothing to reopen.
            if !private {
                self.closed.pop();
            }
            // WebKit names the download's destination after this, and
            // `download_started` still needs to find the opener.
            self.popups.insert(id, opener);
            return;
        }
        let tab = &mut self.tabs[index];
        tab.page = None;
        tab.grant = None;
        tab.saved = crate::model::SavedTab {
            url: crate::model::START.into(),
            title: "New Tab".into(),
        };
        self.dirty = true;
        if index == self.active {
            self.sync_address(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn suggested_names_become_plain_visible_file_names() {
        assert_eq!(file_name("report.pdf"), "report.pdf");
        assert_eq!(file_name("../../etc/passwd"), "-..-etc-passwd");
        assert_eq!(file_name(".zshrc"), "zshrc");
        assert_eq!(file_name("a:b\nc.txt"), "a-b-c.txt");
        assert_eq!(file_name("  "), "download");
        assert_eq!(file_name("..."), "download");
        let long = format!("{}.zip", "é".repeat(300));
        let short = file_name(&long);
        assert!(short.len() <= 200 && short.ends_with(".zip"), "{short}");
    }

    #[test]
    fn taken_names_get_a_counter_before_the_extension() {
        let dir = Path::new("/d");
        let taken: HashSet<PathBuf> = ["/d/a.pdf", "/d/a (1).pdf", "/d/x.tar.gz", "/d/README"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let is_taken = |path: &Path| taken.contains(path);
        assert_eq!(unique(dir, "b.pdf", is_taken), Path::new("/d/b.pdf"));
        assert_eq!(unique(dir, "a.pdf", is_taken), Path::new("/d/a (2).pdf"));
        assert_eq!(
            unique(dir, "x.tar.gz", is_taken),
            Path::new("/d/x (1).tar.gz")
        );
        assert_eq!(unique(dir, "README", is_taken), Path::new("/d/README (1)"));
        assert_eq!(
            unique(dir, "my.notes.txt", |path| path
                == Path::new("/d/my.notes.txt")),
            Path::new("/d/my.notes (1).txt")
        );
    }

    #[test]
    fn only_attachment_dispositions_download() {
        assert!(is_attachment("attachment"));
        assert!(is_attachment(" Attachment ; filename=\"a b.pdf\""));
        assert!(!is_attachment("inline; filename=a.pdf"));
        assert!(!is_attachment("attachments"));
        assert!(!is_attachment(""));
    }

    #[test]
    fn status_shows_percent_size_or_failure() {
        let mut download = Download {
            id: 1,
            tab: 1,
            private: false,
            name: "a".into(),
            path: "/d/a".into(),
            state: State::Active {
                received: 512,
                total: Some(1024),
            },
        };
        assert_eq!(download.status(), "50%");
        download.state = State::Active {
            received: 2_500_000,
            total: None,
        };
        assert_eq!(
            (download.status(), download.fraction()),
            ("2.5 MB".into(), None)
        );
        download.state = State::Failed;
        assert_eq!(download.status(), "Failed");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1_234_567_890), "1.23 GB");
    }
}
