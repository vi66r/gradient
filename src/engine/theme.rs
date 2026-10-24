//! The page's own color, so the shell's control zone above it can match: the
//! first step toward "UI-less" pages whose window takes on the site's theme.
//! Read from an isolated content world (the page can't observe it) after each
//! load: a `theme-color` meta matching the current scheme, else the page's
//! background, else WebKit's default canvas color.
use super::Event;
use async_channel::Sender;
use block2::RcBlock;
use objc2::{MainThreadMarker, runtime::AnyObject};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};

const SCRIPT: &str = r#"(() => {
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  const meta = [...document.querySelectorAll('meta[name="theme-color" i]')]
    .find(meta => meta.content && (!meta.media || matchMedia(meta.media).matches));
  // Normalise any CSS color through a detached canvas (no DOM change).
  const normalise = color => {
    const context = document.createElement('canvas').getContext('2d');
    context.fillStyle = '#010203';
    context.fillStyle = color;
    return context.fillStyle === '#010203' ? '' : context.fillStyle;
  };
  if (meta) return normalise(meta.content);
  for (const element of [document.body, document.documentElement]) {
    const color = element && getComputedStyle(element).backgroundColor;
    if (color && color !== 'transparent' && color !== 'rgba(0, 0, 0, 0)') return normalise(color);
  }
  const scheme = getComputedStyle(document.documentElement).colorScheme || '';
  return dark && scheme.includes('dark') ? '#1e1e1e' : '#ffffff';
})()"#;

pub fn request(view: &WKWebView, id: u64, events: Sender<Event>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let reply = RcBlock::new(move |value: *mut AnyObject, _: *mut NSError| {
        let color = unsafe { value.as_ref() }
            .and_then(|value| value.downcast_ref::<NSString>())
            .and_then(|value| parse(&value.to_string()));
        let _ = events.try_send(Event::PageTint(id, color));
    });
    let world = unsafe { WKContentWorld::worldWithName(&NSString::from_str("Browser Theme"), mtm) };
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(SCRIPT),
            None,
            &world,
            Some(&reply),
        );
    }
}

/// `#rrggbb` only: translucent colors (canvas returns `rgba(...)`) can't be
/// matched by an opaque strip, so they fall back to the shell's own color.
pub fn parse(color: &str) -> Option<u32> {
    let hex = color.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn only_opaque_hex_colors_tint() {
        assert_eq!(parse("#1a2b3c"), Some(0x1a2b3c));
        assert_eq!(parse("rgba(10, 20, 30, 0.5)"), None);
        assert_eq!(parse("#fff"), None);
        assert_eq!(parse(""), None);
    }
}
