//! Site icons for the sidebar. WKWebView has no public favicon API, so after a
//! normal tab finishes loading the shell reads the page's declared icon, fetches
//! it without cookies, and caches it per host. Private tabs never take part.
use anyhow::{Context, Result, bail};
use gpui::ImageFormat;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// Largest icon accepted from the network or disk.
const MAX_BYTES: u64 = 256 * 1024;
/// Icons are refreshed after this long; until then the disk copy is used.
const FRESH_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_CACHED: usize = 500;

/// Read-only: returns the best declared icon and the engine's user agent, so the
/// fetch looks like the page's own client. Prefers SVG, then the smallest
/// declared size of at least 32 px, then any icon.
pub const DISCOVER: &str = r#"(() => {
  const links = [...document.querySelectorAll('link[rel]')].filter(link =>
    /(^|\s)(icon|apple-touch-icon)(\s|$)/i.test(link.rel) && link.href);
  const size = link => {
    if (/svg/i.test(link.type) || /\.svg(\?|$)/i.test(link.href) || /any/i.test(link.sizes.value)) return 1e6;
    const sizes = [...link.sizes].map(s => parseInt(s, 10)).filter(n => n > 0);
    return sizes.length ? Math.max(...sizes) : 0;
  };
  const rank = link => {
    const s = size(link), touch = /apple-touch-icon/i.test(link.rel) ? 1 : 0;
    if (s === 1e6) return [0, 0, touch];
    if (s >= 32) return [1, s, touch];
    return [2, -s, touch];
  };
  links.sort((a, b) => {
    const x = rank(a), y = rank(b);
    return x[0] - y[0] || x[2] - y[2] || x[1] - y[1];
  });
  return { page: location.href, href: links.length ? links[0].href : null, agent: navigator.userAgent };
})()"#;

/// The cache key for a page: host plus any explicit port. `None` for pages
/// that have no icon of their own (non-HTTP schemes, missing hosts).
pub fn host_key(page: &str) -> Option<String> {
    let url = url::Url::parse(page).ok()?;
    crate::model::validate_url(&url).ok()?;
    let host = url
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// Chooses the icon address for `page`: a declared HTTP(S) icon, or the
/// origin's `/favicon.ico`. Data URLs and other schemes fall back as well.
pub fn icon_url(page: &str, declared: Option<&str>) -> Option<String> {
    let page = url::Url::parse(page).ok()?;
    crate::model::validate_url(&page).ok()?;
    let declared = declared
        .filter(|href| href.len() <= 2048)
        .and_then(|href| page.join(href).ok())
        .filter(|url| crate::model::validate_url(url).is_ok());
    Some(match declared {
        Some(url) => url.into(),
        None => page.join("/favicon.ico").ok()?.into(),
    })
}

/// Recognizes the formats GPUI can draw from their leading bytes.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    Some(match bytes {
        [0x89, b'P', b'N', b'G', ..] => ImageFormat::Png,
        [0xff, 0xd8, 0xff, ..] => ImageFormat::Jpeg,
        [b'G', b'I', b'F', b'8', ..] => ImageFormat::Gif,
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => ImageFormat::Webp,
        [0, 0, 1, 0, ..] => ImageFormat::Ico,
        [b'B', b'M', ..] => ImageFormat::Bmp,
        _ if text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg")) => {
            ImageFormat::Svg
        }
        _ => return None,
    })
}

pub fn fetch(url: &str, agent: &str) -> Result<Vec<u8>> {
    let client: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .max_redirects(3)
        .build()
        .into();
    let mut request = client.get(url).header("Accept", "image/*,*/*;q=0.5");
    if !agent.is_empty() && agent.len() <= 512 {
        request = request.header("User-Agent", agent);
    }
    let mut response = request.call().context("Icon request failed")?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Icon too large");
    }
    if sniff(&bytes).is_none() {
        bail!("Unrecognized icon format");
    }
    Ok(bytes)
}

/// Per-profile icon files, named by host key.
#[derive(Clone)]
pub struct DiskCache {
    dir: PathBuf,
}

impl DiskCache {
    pub fn new(profile: &Path) -> Self {
        Self {
            dir: profile.join("icons"),
        }
    }

    fn path(&self, key: &str) -> PathBuf {
        let name: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{name}.icon"))
    }

    /// The cached bytes and whether they are still fresh.
    pub fn read(&self, key: &str) -> Option<(Vec<u8>, bool)> {
        let path = self.path(key);
        let metadata = fs::metadata(&path).ok()?;
        if metadata.len() > MAX_BYTES {
            return None;
        }
        let fresh = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age < FRESH_FOR);
        let bytes = fs::read(path).ok()?;
        sniff(&bytes)?;
        Some((bytes, fresh))
    }

    pub fn write(&self, key: &str, bytes: &[u8]) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.dir)?;
        std::io::Write::write_all(&mut file, bytes)?;
        file.persist(self.path(key))?;
        self.prune();
        Ok(())
    }

    /// Keeps the newest icons when the cache outgrows its bound.
    fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "icon"))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        if files.len() <= MAX_CACHED {
            return;
        }
        files.sort();
        for (_, path) in &files[..files.len() - MAX_CACHED] {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_addresses_resolve_and_fall_back() {
        let page = "https://example.com/docs/page?x=1";
        assert_eq!(
            icon_url(page, Some("/static/icon.svg")).as_deref(),
            Some("https://example.com/static/icon.svg")
        );
        assert_eq!(
            icon_url(page, Some("https://cdn.example.net/i.png")).as_deref(),
            Some("https://cdn.example.net/i.png")
        );
        for unusable in [
            None,
            Some("data:image/png;base64,AAAA"),
            Some("javascript:alert(1)"),
        ] {
            assert_eq!(
                icon_url(page, unusable).as_deref(),
                Some("https://example.com/favicon.ico")
            );
        }
        assert_eq!(
            icon_url(page, Some("https://user:secret@example.com/i.png")).as_deref(),
            Some("https://example.com/favicon.ico")
        );
        assert!(icon_url("about:blank", None).is_none());
        assert!(icon_url("file:///etc/hosts", Some("x.png")).is_none());
    }

    #[test]
    fn host_keys_are_stable_and_path_safe() {
        assert_eq!(
            host_key("https://www.Example.com/a").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host_key("http://127.0.0.1:8766/?x").as_deref(),
            Some("127.0.0.1:8766")
        );
        assert!(host_key("about:blank").is_none());
        let cache = DiskCache::new(Path::new("/tmp/profile"));
        assert_eq!(
            cache.path("127.0.0.1:8766"),
            Path::new("/tmp/profile/icons/127.0.0.1_8766.icon")
        );
        assert_eq!(
            cache.path("../evil"),
            Path::new("/tmp/profile/icons/.._evil.icon")
        );
    }

    #[test]
    fn formats_are_sniffed_not_trusted() {
        assert!(matches!(
            sniff(b"\x89PNG\r\n\x1a\n"),
            Some(ImageFormat::Png)
        ));
        assert!(matches!(sniff(&[0, 0, 1, 0, 1, 0]), Some(ImageFormat::Ico)));
        assert!(matches!(
            sniff(b"\xef\xbb\xbf <svg xmlns='http://www.w3.org/2000/svg'/>"),
            Some(ImageFormat::Svg)
        ));
        assert!(sniff(b"<!doctype html><title>Not found</title>").is_none());
        assert!(sniff(b"").is_none());
    }
}
