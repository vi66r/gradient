use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const START: &str = "browser://start";

/// Longest app link handed to another app. Real `mailto:`/meeting links are far
/// shorter; a cap keeps a page from pushing megabytes through LaunchServices.
pub const APP_LINK_MAX: usize = 8 * 1024;

/// How a URL scheme is treated when a page or the address field asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// Loaded in the page (HTTP and HTTPS).
    Web,
    /// Belongs to another app (`mailto:`, `tel:`, `zoommtg:`…); handed to
    /// macOS after the user confirms.
    App,
    /// Engine-internal, local or system schemes. Never handed to another app.
    Never,
}

pub fn scheme(scheme: &str) -> Scheme {
    let scheme = scheme.to_ascii_lowercase();
    let valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    match scheme.as_str() {
        "http" | "https" => Scheme::Web,
        _ if !valid => Scheme::Never,
        // Page machinery and local content.
        "about" | "blob" | "data" | "file" | "javascript" | "browser"
        // Extension pages; WebKit serves them and enforces which pages may
        // load them.
        | "webkit-extension"
        // Script and system-settings handlers.
        | "applescript" | "help" | "x-man-page" | "itms-services"
        // Mounting network volumes or opening shells can leak credentials or
        // run commands chosen by the page.
        | "smb" | "afp" | "nfs" | "cifs" | "ftp" | "ftps" | "sftp" | "ssh" | "telnet" | "vnc" => {
            Scheme::Never
        }
        _ if scheme.starts_with("x-apple") => Scheme::Never,
        _ => Scheme::App,
    }
}

/// What the address field should do with what was typed.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// Load this address (or the start screen) in the tab.
    Web(String),
    /// Ask before handing this link to the app macOS has registered for it.
    App(url::Url),
}

/// Where address-field searches go. A menu chooses it for now; see
/// `settings.rs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Google,
    Kagi,
    Bing,
    Brave,
}

impl SearchEngine {
    pub const ALL: [Self; 5] = [
        Self::DuckDuckGo,
        Self::Google,
        Self::Kagi,
        Self::Bing,
        Self::Brave,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Google => "Google",
            Self::Kagi => "Kagi",
            Self::Bing => "Bing",
            Self::Brave => "Brave Search",
        }
    }

    fn base(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "https://duckduckgo.com/",
            Self::Google => "https://www.google.com/search",
            Self::Kagi => "https://kagi.com/search",
            Self::Bing => "https://www.bing.com/search",
            Self::Brave => "https://search.brave.com/search",
        }
    }

    /// Where the engine suggests searches for `query` (OpenSearch JSON:
    /// `["query", ["suggestion", …]]`).
    pub fn suggestions(self, query: &str) -> String {
        let (base, key) = match self {
            Self::DuckDuckGo => ("https://duckduckgo.com/ac/?type=list", "q"),
            Self::Google => (
                "https://suggestqueries.google.com/complete/search?client=firefox",
                "q",
            ),
            Self::Kagi => ("https://kagi.com/api/autosuggest", "q"),
            Self::Bing => ("https://api.bing.com/osjson.aspx", "query"),
            Self::Brave => ("https://search.brave.com/api/suggest", "q"),
        };
        let mut url = url::Url::parse(base).expect("suggestion addresses are valid");
        url.query_pairs_mut().append_pair(key, query);
        url.into()
    }

    pub fn search(self, query: &str) -> String {
        let mut url = url::Url::parse(self.base()).expect("search engine addresses are valid");
        url.query_pairs_mut().append_pair("q", query);
        url.into()
    }
}

/// Address-field input: app links (`mailto:a@b.com`, `tel:+1555…`) are offered
/// to another app; everything else follows [`destination`], including search.
pub fn entry(input: &str, engine: SearchEngine) -> Result<Entry> {
    match app_link(input) {
        Some(url) => Ok(Entry::App(url)),
        None => destination(input, engine).map(Entry::Web),
    }
}

/// `scheme:rest` input for an app scheme. `example.com:8080` and
/// `localhost:5173` are hosts with ports, and text with spaces is a search.
fn app_link(input: &str) -> Option<url::Url> {
    let input = input.trim();
    let (name, rest) = input.split_once(':')?;
    if input.chars().any(char::is_whitespace)
        || input.len() > APP_LINK_MAX
        || scheme(name) != Scheme::App
        || name.contains('.')
        || name.eq_ignore_ascii_case("localhost")
        || rest.is_empty()
        || is_port(rest.split(['/', '?', '#']).next()?)
    {
        return None;
    }
    url::Url::parse(input).ok()
}

fn is_port(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Network navigation, local files the user names, and our own start screen
/// are accepted. Pasting JavaScript, data URLs, or credentials must not execute
/// code or persist a password in the session snapshot.
pub fn destination(input: &str, engine: SearchEngine) -> Result<String> {
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
        return Ok(engine.search(input));
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
    Ok(engine.search(input))
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
    value == "about:blank"
        || url::Url::parse(value).is_ok_and(|url| validate_url(&url).is_ok())
        || is_extension_url(value)
}

/// An installed extension's own page (options, dashboards). WebKit serves
/// these only to the extension and to pages its manifest exposes them to.
pub fn is_extension_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| url.scheme() == "webkit-extension" && url.host_str().is_some())
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

/// How many colors a Space can take; see `ui::space_color`.
pub const SPACE_COLORS: u8 = 8;
/// More Spaces than this is a corrupt or hostile session file.
pub const MAX_SPACES: usize = 32;
/// Longest Space name, in characters.
pub const SPACE_NAME_MAX: usize = 40;

/// A Space as saved: its own website data (`id` names the WebKit data store),
/// tabs and color. The Private Space is never saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSpace {
    pub id: String,
    pub name: String,
    pub color: u8,
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

impl SavedSpace {
    /// The Space every profile starts with. It keeps the profile's original
    /// data store, so sign-ins from before Spaces carry over.
    pub fn first(profile: [u8; 16], tabs: Vec<SavedTab>, active: usize) -> Self {
        Self {
            id: uuid::Uuid::from_bytes(profile).to_string(),
            name: "Personal".into(),
            color: 0,
            tabs,
            active,
        }
    }

    pub fn profile(&self) -> Option<[u8; 16]> {
        uuid::Uuid::parse_str(&self.id)
            .ok()
            .map(|id| *id.as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub spaces: Vec<SavedSpace>,
    pub current: usize,
}

/// The single tab list sessions held before Spaces.
#[derive(Deserialize)]
struct LegacySession {
    tabs: Vec<SavedTab>,
    active: usize,
}

impl Session {
    pub fn fresh(profile: [u8; 16]) -> Self {
        Self {
            version: 2,
            spaces: vec![SavedSpace::first(
                profile,
                vec![SavedTab {
                    url: START.into(),
                    title: "New tab".into(),
                }],
                0,
            )],
            current: 0,
        }
    }

    /// Reads a saved session. One from before Spaces becomes a single
    /// Space on the profile's original data store.
    pub fn parse(bytes: &[u8], profile: [u8; 16]) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let session = if value.get("version").and_then(|version| version.as_u64()) == Some(1) {
            let legacy: LegacySession = serde_json::from_value(value)?;
            Self {
                version: 2,
                spaces: vec![SavedSpace::first(profile, legacy.tabs, legacy.active)],
                current: 0,
            }
        } else {
            serde_json::from_value(value)?
        };
        session.validate()?;
        Ok(session)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 2 {
            bail!("This session was created by an unsupported Browser version.");
        }
        if self.spaces.is_empty() || self.spaces.len() > MAX_SPACES {
            bail!("The session has an unsupported number of Spaces.");
        }
        if self
            .spaces
            .iter()
            .map(|space| space.tabs.len())
            .sum::<usize>()
            > 1000
        {
            bail!("The session contains too many tabs.");
        }
        let mut ids = std::collections::HashSet::new();
        for space in &self.spaces {
            if space.profile().is_none() || !ids.insert(&space.id) {
                bail!("The session contains an invalid Space.");
            }
            if space.name.chars().count() > SPACE_NAME_MAX || space.color >= SPACE_COLORS {
                bail!("The session contains an invalid Space.");
            }
            for tab in &space.tabs {
                if tab.url != START && !allowed_address(&tab.url) {
                    bail!("The session contains an unsupported address.");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG: SearchEngine = SearchEngine::DuckDuckGo;

    #[test]
    fn navigation_handles_search_domains_and_local_development() {
        assert_eq!(
            destination(" example.com/a ", DDG).unwrap(),
            "https://example.com/a"
        );
        assert_eq!(
            destination("localhost:5173/test", DDG).unwrap(),
            "http://localhost:5173/test"
        );
        assert_eq!(
            destination("127.0.0.1:8000", DDG).unwrap(),
            "http://127.0.0.1:8000/"
        );
        assert_eq!(
            destination("[::1]:8080", DDG).unwrap(),
            "http://[::1]:8080/"
        );
        let search = url::Url::parse(&destination("3D tools & things", DDG).unwrap()).unwrap();
        assert_eq!(search.query_pairs().next().unwrap().1, "3D tools & things");
    }

    #[test]
    fn searches_go_to_the_chosen_engine() {
        for engine in SearchEngine::ALL {
            let url = url::Url::parse(&destination("rust & gpui", engine).unwrap()).unwrap();
            let query = url.query_pairs().find(|(key, _)| key == "q").unwrap().1;
            assert_eq!(query, "rust & gpui", "{engine:?}");
        }
        assert!(
            destination("hello", SearchEngine::Kagi)
                .unwrap()
                .starts_with("https://kagi.com/search?q=")
        );
        assert!(
            destination("hello", SearchEngine::Google)
                .unwrap()
                .starts_with("https://www.google.com/search?q=")
        );
        // Addresses don't depend on the engine.
        assert_eq!(
            destination("example.com", SearchEngine::Bing).unwrap(),
            "https://example.com/"
        );
    }

    #[test]
    fn navigation_rejects_privileged_schemes_and_embedded_credentials() {
        for input in [
            "javascript:alert(1)",
            "data:text/html,hello",
            "https://user:secret@example.com",
            "ftp://example.com",
        ] {
            assert!(destination(input, DDG).is_err(), "accepted {input}");
            assert!(!allowed_navigation(input));
        }
    }

    #[test]
    fn extension_pages_may_load_in_tabs() {
        let page = "webkit-extension://3f2a9c1e-0b4d-4e8a-9f61-2c7d5b8e1a40/dashboard.html";
        assert!(is_extension_url(page));
        assert!(allowed_navigation(page));
        assert!(allowed_address(page));
        assert!(!is_extension_url("webkit-extension:///dashboard.html"));
        assert!(!is_extension_url("https://example.com/"));
    }

    #[test]
    fn address_field_opens_local_files() {
        assert_eq!(
            destination("/Users/rafi/notes.html", DDG).unwrap(),
            "file:///Users/rafi/notes.html"
        );
        // Literal paths: spaces and percent signs are part of the name.
        assert_eq!(
            destination(" /tmp/My Report 100%.pdf ", DDG).unwrap(),
            "file:///tmp/My%20Report%20100%25.pdf"
        );
        assert_eq!(
            destination("file:///tmp/My%20Report.pdf", DDG).unwrap(),
            "file:///tmp/My%20Report.pdf"
        );
        assert_eq!(
            destination("file://localhost/tmp/a.html#top", DDG).unwrap(),
            "file:///tmp/a.html#top"
        );
        assert_eq!(
            destination("FILE:///tmp/a.txt", DDG).unwrap(),
            "file:///tmp/a.txt"
        );
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            destination("~/Desktop/a.png", DDG).unwrap(),
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
            let url = url::Url::parse(&destination(input, DDG).unwrap()).unwrap();
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
            assert!(destination(input, DDG).is_err(), "accepted {input}");
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
        let mut session = Session::fresh([7; 16]);
        session.spaces[0].tabs = vec![SavedTab {
            url,
            title: String::new(),
        }];
        assert!(session.validate().is_ok());
    }

    #[test]
    fn unsupported_sessions_are_rejected_instead_of_silently_replaced() {
        let session = Session {
            version: 3,
            ..Session::fresh([7; 16])
        };
        assert!(session.validate().is_err());
        let mut twins = Session::fresh([7; 16]);
        twins.spaces.push(twins.spaces[0].clone());
        assert!(twins.validate().is_err());
        let mut nameless = Session::fresh([7; 16]);
        nameless.spaces[0].id = "not a uuid".into();
        assert!(nameless.validate().is_err());
    }

    #[test]
    fn sessions_from_before_spaces_become_one_space_on_the_original_store() {
        let legacy = br#"{"version":1,"tabs":[{"url":"https://example.com/","title":"Example"}],"active":0}"#;
        let session = Session::parse(legacy, [7; 16]).unwrap();
        assert_eq!(session.version, 2);
        assert_eq!(session.spaces.len(), 1);
        assert_eq!(session.spaces[0].profile(), Some([7; 16]));
        assert_eq!(session.spaces[0].name, "Personal");
        assert_eq!(session.spaces[0].tabs[0].url, "https://example.com/");
        // Spaces round-trip as written.
        let saved = serde_json::to_vec(&session).unwrap();
        assert_eq!(Session::parse(&saved, [9; 16]).unwrap(), session);
    }

    #[test]
    fn schemes_are_classified_for_app_handoff() {
        for web in ["http", "https", "HTTPS"] {
            assert_eq!(scheme(web), Scheme::Web, "{web}");
        }
        for app in [
            "mailto", "tel", "sms", "facetime", "maps", "zoommtg", "slack", "spotify", "vscode",
            "msteams", "MAILTO",
        ] {
            assert_eq!(scheme(app), Scheme::App, "{app}");
        }
        for never in [
            "file",
            "javascript",
            "JavaScript",
            "data",
            "blob",
            "about",
            "browser",
            "webkit-extension",
            "applescript",
            "x-apple.systempreferences",
            "x-apple-helpbasic",
            "x-man-page",
            "smb",
            "",
            "1tel",
            "ma il",
        ] {
            assert_eq!(scheme(never), Scheme::Never, "{never}");
        }
    }

    #[test]
    fn address_entries_offer_app_links_and_keep_search_and_hosts() {
        let app = |input: &str| match entry(input, DDG).unwrap_or_else(|e| panic!("{input}: {e}")) {
            Entry::App(url) => url.to_string(),
            Entry::Web(url) => panic!("{input} became {url}"),
        };
        let web = |input: &str| match entry(input, DDG).unwrap_or_else(|e| panic!("{input}: {e}")) {
            Entry::Web(url) => url,
            Entry::App(url) => panic!("{input} offered to an app as {url}"),
        };
        assert_eq!(
            app(" mailto:someone@example.com "),
            "mailto:someone@example.com"
        );
        assert_eq!(app("tel:+15551234567"), "tel:+15551234567");
        assert_eq!(
            app("zoommtg://zoom.us/join?confno=1"),
            "zoommtg://zoom.us/join?confno=1"
        );
        assert_eq!(web("https://example.com"), "https://example.com/");
        assert_eq!(web("example.com:8080/a"), "https://example.com:8080/a");
        assert_eq!(web("localhost:5173"), "http://localhost:5173/");
        assert_eq!(web(""), START);
        for search in ["hello world", "what is: a colon", "3D tools & things"] {
            assert!(
                web(search).starts_with("https://duckduckgo.com/?q="),
                "{search}"
            );
        }
        // Local files the user names load in the page; they never go to an app.
        assert_eq!(web("file:///etc/hosts"), "file:///etc/hosts");
        for never in [
            "javascript:alert(1)",
            "data:text/html,hello",
            "x-apple.systempreferences:com.apple.preference.security",
            "applescript://com.apple.scripteditor?action=new",
        ] {
            assert!(entry(never, DDG).is_err(), "accepted {never}");
        }
        let long = format!("mailto:a@example.com?body={}", "x".repeat(APP_LINK_MAX));
        assert!(
            !matches!(entry(&long, DDG), Ok(Entry::App(_))),
            "uncapped app link"
        );
    }
}
