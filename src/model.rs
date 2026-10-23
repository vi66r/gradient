use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const START: &str = "browser://start";

/// Network navigation, local files the user names, and our own start screen
/// are accepted. Pasting JavaScript, data URLs, or credentials must not execute
/// code or persist a password in the session snapshot.
pub fn destination(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() || input == START {
        return Ok(START.into());
    }
    if let Some(path) = local_path(input) {
        return file_url_from_path(&path);
    }
    let lower = input.to_ascii_lowercase();
    if lower.starts_with("file:") {
        let parsed = url::Url::parse(input)?;
        validate_file_url(&parsed)?;
        return Ok(parsed.into());
    }
    // Relative paths can't name a file without a working directory.
    if input.starts_with("./") || input.starts_with("../") {
        return search(input);
    }
    let network = lower.starts_with("http://") || lower.starts_with("https://");
    if input.contains("://")
        || ["javascript:", "data:", "file:", "about:"]
            .iter()
            .any(|s| lower.starts_with(s))
    {
        let parsed = url::Url::parse(input)?;
        validate_url(&parsed)?;
        return Ok(parsed.into());
    }
    let host_like = !input.chars().any(char::is_whitespace)
        && (input.contains('.') || input.starts_with("localhost") || input.starts_with('['));
    if network || host_like {
        let raw = if network {
            input.to_string()
        } else {
            let local = input.starts_with("localhost")
                || input.starts_with("127.")
                || input.starts_with("[::1]");
            format!("{}://{input}", if local { "http" } else { "https" })
        };
        if let Ok(url) = url::Url::parse(&raw) {
            validate_url(&url)?;
            return Ok(url.to_string());
        }
        if network {
            bail!("That address is not a valid HTTP or HTTPS URL.");
        }
    }
    if let Ok(url) = url::Url::parse(input) {
        validate_url(&url)?;
        return Ok(url.to_string());
    }
    search(input)
}

fn search(input: &str) -> Result<String> {
    let mut search = url::Url::parse("https://duckduckgo.com/")?;
    search.query_pairs_mut().append_pair("q", input);
    Ok(search.into())
}

/// An absolute path (`/Users/…`) or a home-relative one (`~/…`), taken
/// literally: `%` and spaces are file-name characters here, not URL syntax.
fn local_path(input: &str) -> Option<std::path::PathBuf> {
    if input.starts_with('/') && !input.starts_with("//") {
        return Some(input.into());
    }
    let rest = input.strip_prefix('~')?;
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(std::path::PathBuf::from(home).join(rest.trim_start_matches('/')))
}

pub fn file_url_from_path(path: &std::path::Path) -> Result<String> {
    match url::Url::from_file_path(path) {
        Ok(url) => Ok(url.into()),
        Err(()) => bail!("That isn't a valid file path."),
    }
}

/// Local files only: no remote host (`file://server/…`) and no credentials.
pub fn validate_file_url(url: &url::Url) -> Result<()> {
    if url.scheme() != "file" || url.host_str().is_some_and(|host| !host.is_empty()) {
        bail!("Only files on this computer can be opened.");
    }
    if !url.username().is_empty() || url.password().is_some() || url.to_file_path().is_err() {
        bail!("That file address is not valid.");
    }
    Ok(())
}

pub fn is_file_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| validate_file_url(&url).is_ok())
}

pub fn file_path(value: &str) -> Option<std::path::PathBuf> {
    let url = url::Url::parse(value).ok()?;
    validate_file_url(&url).ok()?;
    url.to_file_path().ok()
}

/// Policy for any navigation to a `file:` URL. Browser may start one (address
/// field, drop, open panel, session restore, its own Back/Forward/Reload), and
/// a local page may link to another local file. A web page may never navigate
/// to or open one, in any frame.
pub fn file_navigation_allowed(target: &str, browser: bool, source: Option<&str>) -> bool {
    is_file_url(target) && (browser || source.is_some_and(is_file_url))
}

/// The directory WebKit may read for a local page, so its relative images and
/// stylesheets load. A file at the disk root gets access to itself only.
pub fn read_access(path: &std::path::Path) -> std::path::PathBuf {
    match path.parent() {
        Some(parent) if parent.parent().is_some() => parent.to_path_buf(),
        _ => path.to_path_buf(),
    }
}

/// Why a local file can't be shown, checked before it reaches WebKit.
pub fn file_problem(value: &str) -> Option<String> {
    let Some(path) = file_path(value) else {
        return Some("That file address is not valid.".into());
    };
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => {
            Some("Browser can't show folders. Open a file instead.".into())
        }
        Ok(_) => None,
        Err(_) => Some(format!(
            "{} couldn't be found. It may have been moved, renamed or deleted.",
            path.display()
        )),
    }
}

/// The decoded last path segment of a file URL, for titles and the resting
/// address field.
pub fn file_name(value: &str) -> Option<String> {
    let path = file_path(value)?;
    Some(
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    )
}

pub fn validate_url(url: &url::Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Only HTTP and HTTPS addresses are supported.");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!(
            "Addresses containing credentials are not supported. Sign in on the website instead."
        );
    }
    Ok(())
}

pub fn allowed_navigation(value: &str) -> bool {
    value == "about:blank" || url::Url::parse(value).is_ok_and(|url| validate_url(&url).is_ok())
}

/// Addresses the shell may show, save and restore: web pages and local files.
/// What a page may navigate to is the narrower `allowed_navigation`.
pub fn allowed_address(value: &str) -> bool {
    allowed_navigation(value) || is_file_url(value)
}

pub fn display_host(value: &str) -> String {
    if let Some(name) = file_name(value) {
        return name;
    }
    url::Url::parse(value)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_string())
        })
        .unwrap_or_else(|| "New tab".into())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            version: 1,
            tabs: vec![SavedTab {
                url: START.into(),
                title: "New tab".into(),
            }],
            active: 0,
        }
    }
}

impl Session {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("This session was created by an unsupported Browser version.");
        }
        if self.tabs.len() > 1000 {
            bail!("The session contains too many tabs.");
        }
        for tab in &self.tabs {
            if tab.url != START && !allowed_address(&tab.url) {
                bail!("The session contains an unsupported address.");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_handles_search_domains_and_local_development() {
        assert_eq!(
            destination(" example.com/a ").unwrap(),
            "https://example.com/a"
        );
        assert_eq!(
            destination("localhost:5173/test").unwrap(),
            "http://localhost:5173/test"
        );
        assert_eq!(
            destination("127.0.0.1:8000").unwrap(),
            "http://127.0.0.1:8000/"
        );
        assert_eq!(destination("[::1]:8080").unwrap(), "http://[::1]:8080/");
        let search = url::Url::parse(&destination("3D tools & things").unwrap()).unwrap();
        assert_eq!(search.query_pairs().next().unwrap().1, "3D tools & things");
    }

    #[test]
    fn navigation_rejects_privileged_schemes_and_embedded_credentials() {
        for input in [
            "javascript:alert(1)",
            "data:text/html,hello",
            "https://user:secret@example.com",
            "ftp://example.com",
        ] {
            assert!(destination(input).is_err(), "accepted {input}");
            assert!(!allowed_navigation(input));
        }
    }

    #[test]
    fn address_field_opens_local_files() {
        assert_eq!(
            destination("/Users/rafi/notes.html").unwrap(),
            "file:///Users/rafi/notes.html"
        );
        // Literal paths: spaces and percent signs are part of the name.
        assert_eq!(
            destination(" /tmp/My Report 100%.pdf ").unwrap(),
            "file:///tmp/My%20Report%20100%25.pdf"
        );
        assert_eq!(
            destination("file:///tmp/My%20Report.pdf").unwrap(),
            "file:///tmp/My%20Report.pdf"
        );
        assert_eq!(
            destination("file://localhost/tmp/a.html#top").unwrap(),
            "file:///tmp/a.html#top"
        );
        assert_eq!(
            destination("FILE:///tmp/a.txt").unwrap(),
            "file:///tmp/a.txt"
        );
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            destination("~/Desktop/a.png").unwrap(),
            url::Url::from_file_path(format!("{home}/Desktop/a.png"))
                .unwrap()
                .as_str()
        );
        assert_eq!(
            file_name("file:///tmp/My%20Report.pdf").as_deref(),
            Some("My Report.pdf")
        );
        assert_eq!(display_host("file:///tmp/My%20Report.pdf"), "My Report.pdf");
        assert!(allowed_address("file:///tmp/a.html"));
        assert!(!allowed_navigation("file:///tmp/a.html"));
    }

    #[test]
    fn relative_paths_and_words_still_search() {
        for input in [
            "./notes.html",
            "../index.html",
            "~bob/x",
            "notes",
            "my file.html",
        ] {
            let url = url::Url::parse(&destination(input).unwrap()).unwrap();
            assert_eq!(url.host_str(), Some("duckduckgo.com"), "{input}");
            assert_eq!(url.query_pairs().next().unwrap().1, input);
        }
    }

    #[test]
    fn remote_or_credentialed_file_urls_are_rejected() {
        for input in [
            "file://server/share/a.html",
            "file://user:pw@localhost/tmp/a",
        ] {
            assert!(destination(input).is_err(), "accepted {input}");
            assert!(!allowed_address(input), "allowed {input}");
        }
    }

    #[test]
    fn only_browser_or_local_pages_may_open_files() {
        let file = "file:///tmp/a.html";
        assert!(file_navigation_allowed(file, true, None));
        assert!(file_navigation_allowed(
            file,
            false,
            Some("file:///tmp/b.html")
        ));
        assert!(!file_navigation_allowed(file, false, None));
        assert!(!file_navigation_allowed(
            file,
            false,
            Some("https://example.com/")
        ));
        assert!(!file_navigation_allowed(file, false, Some("about:blank")));
        assert!(!file_navigation_allowed("https://example.com/", true, None));
        assert!(!file_navigation_allowed("file://server/a", true, None));
        assert_eq!(
            read_access(std::path::Path::new("/Users/rafi/site/index.html")),
            std::path::Path::new("/Users/rafi/site")
        );
        assert_eq!(
            read_access(std::path::Path::new("/index.html")),
            std::path::Path::new("/index.html")
        );
    }

    #[test]
    fn missing_files_and_folders_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("page.html");
        std::fs::write(&file, "<p>hi</p>").unwrap();
        let url = file_url_from_path(&file).unwrap();
        assert_eq!(file_problem(&url), None);
        assert!(file_problem(&file_url_from_path(dir.path()).unwrap()).is_some());
        std::fs::remove_file(&file).unwrap();
        assert!(file_problem(&url).unwrap().contains("couldn't be found"));
        // A missing file still restores; the tab shows the error instead.
        let session = Session {
            version: 1,
            tabs: vec![SavedTab {
                url,
                title: String::new(),
            }],
            active: 0,
        };
        assert!(session.validate().is_ok());
    }

    #[test]
    fn unsupported_sessions_are_rejected_instead_of_silently_replaced() {
        let session = Session {
            version: 2,
            ..Session::default()
        };
        assert!(session.validate().is_err());
    }
}
