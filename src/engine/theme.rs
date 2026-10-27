//! The page's own color, painted behind the page while WebKit has nothing
//! drawn, and the first step toward "UI-less" pages whose window takes on the
//! site's theme.
//!
//! In order of preference, all key-value observable so the color follows live
//! (new pages, SPAs switching themes) without scripts or polling:
//! 0. Gradient's own sample: a tiny snapshot of the page's top edge, taken after
//!    each load and again over the next few seconds. WebKit samples only at
//!    first paint, so apps with a splash screen (Gmail's white "Loading")
//!    would otherwise keep the splash's color. Four small captures per load,
//!    nothing at idle.
//! 1. `_sampledPageTopColor` (SPI, enabled in `page_configuration`): the color
//!    actually visible across the top of the page. Gmail paints dark inside a
//!    white document, so only sampling matches it. Nil when the top isn't one
//!    color (a photo banner).
//! 2. An opaque declared `theme-color`. YouTube declares translucent white
//!    even in dark mode, so translucent ones are skipped.
//! 3. The page background (`underPageBackgroundColor`).
use super::Event;
use async_channel::Sender;
use block2::RcBlock;
use objc2::AnyThread;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, NSObject},
};
use objc2_app_kit::{NSBitmapImageRep, NSColor, NSColorRenderingIntent, NSColorSpace, NSImage};
use objc2_foundation::{NSError, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use std::{cell::Cell, ffi::c_void};

/// Height of the page edge Gradient samples, in points.
const EDGE: f64 = 8.;

const OBSERVED: [&str; 3] = [
    "_sampledPageTopColor",
    "themeColor",
    "underPageBackgroundColor",
];

struct Ivars {
    id: u64,
    events: Sender<Event>,
    view: Weak<WKWebView>,
    /// Gradient's latest top-edge sample, cleared for each new document.
    sampled: Cell<Option<u32>>,
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
            self.report();
        }
    }
);

impl Observer {
    fn report(&self) {
        if let Some(view) = self.ivars().view.load() {
            let (color, definite) = current(&view, self.ivars().sampled.get());
            let _ = self
                .ivars()
                .events
                .try_send(Event::PageTint(self.ivars().id, color, definite));
        }
    }
}

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
            sampled: Cell::new(None),
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

    /// A new document: forget the previous page's sample.
    pub fn reset(&self) {
        self.observer.ivars().sampled.set(None);
    }

    /// Samples the page's top edge; the result arrives as `Event::PageTint`.
    pub fn sample(&self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let bounds = self.view.bounds();
        // Leave out the scroll bar's lane on the right.
        let width = bounds.size.width - 16.;
        if width < 40. || bounds.size.height < 2. * EDGE {
            return;
        }
        let top = if self.view.isFlipped() {
            0.
        } else {
            bounds.size.height - EDGE
        };
        let config = unsafe { WKSnapshotConfiguration::new(mtm) };
        unsafe {
            config.setRect(NSRect::new(NSPoint::new(0., top), NSSize::new(width, EDGE)));
            config.setSnapshotWidth(Some(&NSNumber::new_f64(96.)));
        }
        let observer = self.observer.clone();
        let done = RcBlock::new(move |image: *mut NSImage, _: *mut NSError| {
            let color = unsafe { image.as_ref() }.and_then(uniform);
            observer.ivars().sampled.set(color);
            observer.report();
        });
        unsafe {
            self.view
                .takeSnapshotWithConfiguration_completionHandler(Some(&config), &done)
        };
    }
}

/// The edge's color if it is effectively one opaque color, else `None`.
fn uniform(image: &NSImage) -> Option<u32> {
    let cg =
        unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    // Read pixels as sRGB, honoring the snapshot's own color tag.
    let bitmap = bitmap.bitmapImageRepByConvertingToColorSpace_renderingIntent(
        &NSColorSpace::sRGBColorSpace(),
        NSColorRenderingIntent::Default,
    )?;
    let (wide, high) = (bitmap.pixelsWide(), bitmap.pixelsHigh());
    if wide <= 0 || high <= 0 {
        return None;
    }
    let mut samples = Vec::new();
    for y in (0..high).step_by((high as usize / 3).max(1)) {
        for x in (0..wide).step_by((wide as usize / 24).max(1)) {
            // The bitmap is sRGB now, but `colorAtX_y` labels its values
            // Generic RGB; converting that "back" to sRGB would lift #1f1f1f
            // to #292929. Its components are already the sRGB values.
            let color = bitmap.colorAtX_y(x, y)?;
            samples.push((
                rgb(
                    color.redComponent(),
                    color.greenComponent(),
                    color.blueComponent(),
                ),
                color.alphaComponent(),
            ));
        }
    }
    uniform_rgb(&samples)
}

/// Opaque and within a small spread on every channel: the average color.
fn uniform_rgb(samples: &[(u32, f64)]) -> Option<u32> {
    if samples.is_empty() || samples.iter().any(|(_, alpha)| *alpha < 0.999) {
        return None;
    }
    let channel = |rgb: u32, shift: u32| (rgb >> shift) & 0xff;
    let mut color = 0;
    for shift in [16, 8, 0] {
        let values: Vec<u32> = samples
            .iter()
            .map(|(rgb, _)| channel(*rgb, shift))
            .collect();
        let (low, high) = (values.iter().min()?, values.iter().max()?);
        if high - low > 16 {
            return None;
        }
        color |= (values.iter().sum::<u32>() / values.len() as u32) << shift;
    }
    Some(color)
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

/// The page's color, and whether it is definite: painted at the top or
/// declared opaque, rather than only the document background, which a page
/// shows before it paints its own colors.
fn current(view: &WKWebView, own: Option<u32>) -> (Option<u32>, bool) {
    if own.is_some() {
        return (own, true);
    }
    let sampled: Option<Retained<NSColor>> = unsafe {
        let available: bool =
            msg_send![view, respondsToSelector: objc2::sel!(_sampledPageTopColor)];
        if available {
            msg_send![view, _sampledPageTopColor]
        } else {
            None
        }
    };
    let theme = unsafe { view.themeColor() };
    let under = unsafe { view.underPageBackgroundColor() };
    let (sampled, theme) = (srgb(sampled.as_deref()), srgb(theme.as_deref()));
    let opaque = |color: Option<(u32, f64)>| color.is_some_and(|(_, alpha)| alpha >= 0.999);
    let definite = opaque(sampled) || opaque(theme);
    (pick(sampled, theme, srgb(Some(&under))), definite)
}

/// `0xRRGGBB` and alpha in sRGB.
fn srgb(color: Option<&NSColor>) -> Option<(u32, f64)> {
    let color = color?.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some((
        rgb(
            color.redComponent(),
            color.greenComponent(),
            color.blueComponent(),
        ),
        color.alphaComponent(),
    ))
}

/// `0xRRGGBB` from unit components.
fn rgb(red: f64, green: f64, blue: f64) -> u32 {
    let channel = |value: f64| (value.clamp(0., 1.) * 255.).round() as u32;
    channel(red) << 16 | channel(green) << 8 | channel(blue)
}

/// The first opaque color of: what's visible at the top, the declared theme
/// color, the page background. Translucent colors can't stand in for the
/// page on an opaque backdrop, so they're skipped.
fn pick(
    sampled: Option<(u32, f64)>,
    theme: Option<(u32, f64)>,
    background: Option<(u32, f64)>,
) -> Option<u32> {
    let opaque = |color: Option<(u32, f64)>| color.filter(|(_, alpha)| *alpha >= 0.999);
    opaque(sampled)
        .or(opaque(theme))
        .or(opaque(background))
        .map(|(rgb, _)| rgb)
}

#[cfg(test)]
mod tests {
    use super::{pick, uniform_rgb};

    #[test]
    fn edge_samples_must_be_one_opaque_color() {
        let gmail = [(0x1f1f1f, 1.), (0x1e1e1e, 1.), (0x202020, 1.)];
        assert_eq!(uniform_rgb(&gmail), Some(0x1f1f1f));
        // A photo banner or logo across the edge: no single color.
        assert_eq!(uniform_rgb(&[(0x1f1f1f, 1.), (0xe53935, 1.)]), None);
        assert_eq!(uniform_rgb(&[(0xffffff, 0.5)]), None);
        assert_eq!(uniform_rgb(&[]), None);
    }

    #[test]
    fn the_visible_top_color_wins_then_opaque_theme_then_background() {
        // Gmail: dark app painted inside a white document.
        assert_eq!(
            pick(Some((0x1e1e1e, 1.)), None, Some((0xffffff, 1.))),
            Some(0x1e1e1e)
        );
        // YouTube: translucent white theme-color over a near-black page.
        assert_eq!(
            pick(None, Some((0xffffff, 0.98)), Some((0x0f0f0f, 1.))),
            Some(0x0f0f0f)
        );
        // A deliberate opaque brand color beats the plain background.
        assert_eq!(
            pick(None, Some((0x1f2a44, 1.)), Some((0xffffff, 1.))),
            Some(0x1f2a44)
        );
        assert_eq!(pick(None, None, Some((0xffffff, 0.5))), None);
        assert_eq!(pick(None, None, None), None);
    }
}
