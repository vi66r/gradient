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

/// Address-field input: app links (`mailto:a@b.com`, `tel:+1555…`) are offered
/// to another app; everything else follows [`destination`], including search.
pub fn entry(input: &str) -> Result<Entry> {
    match app_link(input) {
        Some(url) => Ok(Entry::App(url)),
        None => destination(input).map(Entry::Web),
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

/// Only explicit network navigation and our own start screen are accepted.
/// In particular, pasting JavaScript, file URLs, or credentials must not execute
/// code, expose local files, or persist a password in the session snapshot.
pub fn destination(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() || input == START {
        return Ok(START.into());
    }
    let lower = input.to_ascii_lowercase();
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
    let mut search = url::Url::parse("https://duckduckgo.com/")?;
    search.query_pairs_mut().append_pair("q", input);
    Ok(search.into())
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

pub fn display_host(value: &str) -> String {
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
            if tab.url != START && !allowed_navigation(&tab.url) {
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
            "file:///etc/passwd",
            "data:text/html,hello",
            "https://user:secret@example.com",
            "ftp://example.com",
        ] {
            assert!(destination(input).is_err(), "accepted {input}");
            assert!(!allowed_navigation(input));
        }
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
        let app = |input: &str| match entry(input).unwrap_or_else(|e| panic!("{input}: {e}")) {
            Entry::App(url) => url.to_string(),
            Entry::Web(url) => panic!("{input} became {url}"),
        };
        let web = |input: &str| match entry(input).unwrap_or_else(|e| panic!("{input}: {e}")) {
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
        for never in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hello",
            "x-apple.systempreferences:com.apple.preference.security",
            "applescript://com.apple.scripteditor?action=new",
        ] {
            assert!(entry(never).is_err(), "accepted {never}");
        }
        let long = format!("mailto:a@example.com?body={}", "x".repeat(APP_LINK_MAX));
        assert!(
            !matches!(entry(&long), Ok(Entry::App(_))),
            "uncapped app link"
        );
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
