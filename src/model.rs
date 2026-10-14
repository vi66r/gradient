use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const START: &str = "browser://start";

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
    fn unsupported_sessions_are_rejected_instead_of_silently_replaced() {
        let session = Session {
            version: 2,
            ..Session::default()
        };
        assert!(session.validate().is_err());
    }
}
