//! The page's own color, so the shell's control strip above it can match: the
//! first step toward "UI-less" pages whose window takes on the site's theme.
//!
//! WebKit's public `themeColor` and `underPageBackgroundColor` are both
//! key-value observable, so the strip follows live (new pages, SPAs switching
//! themes) without scripts or polling. A declared `theme-color` wins only when
//! it is fully opaque: YouTube declares translucent white even in dark mode,
//! while its real background is near-black.
use super::Event;
use async_channel::Sender;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, NSObject},
};
use objc2_app_kit::{NSColor, NSColorSpace};
use objc2_foundation::NSString;
use objc2_web_kit::WKWebView;
use std::ffi::c_void;

const OBSERVED: [&str; 2] = ["themeColor", "underPageBackgroundColor"];

struct Ivars {
    id: u64,
    events: Sender<Event>,
    view: Weak<WKWebView>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Observer;

    impl Observer {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key: *mut NSString,
            _object: *mut AnyObject,
            _change: *mut AnyObject,
            _context: *mut c_void,
        ) {
            if let Some(view) = self.ivars().view.load() {
                let _ = self
                    .ivars()
                    .events
                    .try_send(Event::PageTint(self.ivars().id, current(&view)));
            }
        }
    }
);

/// Keeps the observation alive for the page's lifetime.
pub struct Theme {
    view: Retained<WKWebView>,
    observer: Retained<Observer>,
}

impl Theme {
    pub fn install(view: Retained<WKWebView>, id: u64, events: Sender<Event>) -> Self {
        let mtm = MainThreadMarker::new().expect("Pages are created on the UI thread");
        let observer = Observer::alloc(mtm).set_ivars(Ivars {
            id,
            events,
            view: Weak::from_retained(&view),
        });
        let observer: Retained<Observer> = unsafe { msg_send![super(observer), init] };
        for key in OBSERVED {
            let key = NSString::from_str(key);
            let _: () = unsafe {
                msg_send![&*view, addObserver: &*observer, forKeyPath: &*key,
                    options: 0usize, context: std::ptr::null_mut::<c_void>()]
            };
        }
        Self { view, observer }
    }
}

impl Drop for Theme {
    fn drop(&mut self) {
        for key in OBSERVED {
            let key = NSString::from_str(key);
            let _: () = unsafe {
                msg_send![&*self.view, removeObserver: &*self.observer, forKeyPath: &*key]
            };
        }
    }
}

fn current(view: &WKWebView) -> Option<u32> {
    let theme = unsafe { view.themeColor() };
    let under = unsafe { view.underPageBackgroundColor() };
    pick(srgb(theme.as_deref()), srgb(Some(&under)))
}

/// `0xRRGGBB` and alpha in sRGB.
fn srgb(color: Option<&NSColor>) -> Option<(u32, f64)> {
    let color = color?.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let channel = |value: f64| (value.clamp(0., 1.) * 255.).round() as u32;
    Some((
        channel(color.redComponent()) << 16
            | channel(color.greenComponent()) << 8
            | channel(color.blueComponent()),
        color.alphaComponent(),
    ))
}

/// An opaque declared theme color, else the opaque page background. A
/// translucent color can't be matched by an opaque strip, so it's skipped.
fn pick(theme: Option<(u32, f64)>, background: Option<(u32, f64)>) -> Option<u32> {
    let opaque = |color: Option<(u32, f64)>| color.filter(|(_, alpha)| *alpha >= 0.999);
    opaque(theme).or(opaque(background)).map(|(rgb, _)| rgb)
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn translucent_theme_colors_defer_to_the_page_background() {
        // YouTube: translucent white theme-color over a near-black page.
        assert_eq!(
            pick(Some((0xffffff, 0.98)), Some((0x0f0f0f, 1.))),
            Some(0x0f0f0f)
        );
        // A deliberate opaque brand color wins.
        assert_eq!(
            pick(Some((0x1f2a44, 1.)), Some((0xffffff, 1.))),
            Some(0x1f2a44)
        );
        assert_eq!(pick(None, Some((0xfafbff, 1.))), Some(0xfafbff));
        assert_eq!(pick(None, Some((0xffffff, 0.5))), None);
        assert_eq!(pick(None, None), None);
    }
}
