//! Native finishing for the page's top edge and arrival. WebKit draws above
//! GPUI, so the shell can't paint over the page itself:
//! - `EdgeFade` sits over the top of the page and runs from the control
//!   strip's color to transparent, so content dissolves into the strip
//!   instead of meeting it at a hard line. It never takes clicks.
//! - `fade_in` brings a newly committed page up from transparent, in step
//!   with the strip's color transition.
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSView};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSString};

/// How far the page dissolves into the strip.
pub const EDGE: f64 = 14.;
/// Matches the shell's NORMAL motion so strip and page move together.
const FADE_IN: f64 = 0.24;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    pub struct EdgeFade;

    impl EdgeFade {
        /// Purely visual: clicks, scrolls and hovers reach the page beneath.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }

        #[unsafe(method(makeBackingLayer))]
        fn make_backing_layer(&self) -> *mut AnyObject {
            let class = AnyClass::get(c"CAGradientLayer").expect("QuartzCore is linked by AppKit");
            unsafe { msg_send![class, layer] }
        }
    }
);

impl EdgeFade {
    /// A strip across the top of `host` (an unflipped view), kept there as
    /// the host resizes.
    pub fn attach(host: &NSView, mtm: MainThreadMarker, color: u32) -> Retained<Self> {
        let bounds = host.bounds();
        let frame = NSRect::new(
            NSPoint::new(0., bounds.size.height - EDGE),
            objc2_foundation::NSSize::new(bounds.size.width, EDGE),
        );
        let view: Retained<Self> = unsafe { msg_send![Self::alloc(mtm), initWithFrame: frame] };
        view.setWantsLayer(true);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        view.set_color(color);
        host.addSubview(&view);
        view
    }

    /// Gradient from `color` (`0xRRGGBB`) at the top edge to transparent.
    /// Core Animation animates the change implicitly, alongside the strip.
    pub fn set_color(&self, color: u32) {
        let channel = |shift: u32| f64::from((color >> shift) & 0xff) / 255.;
        let tone = |alpha: f64| {
            NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), alpha)
        };
        let (solid, clear) = (tone(1.), tone(0.));
        let (solid, clear) = (solid.CGColor(), clear.CGColor());
        // CGColor is toll-free bridged to an Objective-C object.
        let colors: [*const AnyObject; 2] = [
            (&*solid as *const _ as *const AnyObject),
            (&*clear as *const _ as *const AnyObject),
        ];
        let array_class = AnyClass::get(c"NSArray").expect("Foundation is linked");
        unsafe {
            let array: *mut AnyObject =
                msg_send![array_class, arrayWithObjects: colors.as_ptr(), count: 2usize];
            let layer: *mut AnyObject = msg_send![self, layer];
            if let Some(layer) = layer.as_ref() {
                // Unflipped layer space: y = 1 is the top edge.
                let _: () = msg_send![layer, setStartPoint: NSPoint::new(0.5, 1.)];
                let _: () = msg_send![layer, setEndPoint: NSPoint::new(0.5, 0.)];
                let _: () = msg_send![layer, setColors: array];
            }
        }
    }
}

/// Fades `view` in from transparent; used when a new document commits.
pub fn fade_in(view: &NSView) {
    view.setWantsLayer(true);
    let (Some(animation_class), Some(timing_class)) = (
        AnyClass::get(c"CABasicAnimation"),
        AnyClass::get(c"CAMediaTimingFunction"),
    ) else {
        return;
    };
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        let Some(layer) = layer.as_ref() else {
            return;
        };
        let animation: *mut AnyObject =
            msg_send![animation_class, animationWithKeyPath: &*NSString::from_str("opacity")];
        let timing: *mut AnyObject =
            msg_send![timing_class, functionWithName: &*NSString::from_str("easeOut")];
        let _: () = msg_send![animation, setFromValue: &*NSNumber::new_f64(0.)];
        let _: () = msg_send![animation, setToValue: &*NSNumber::new_f64(1.)];
        let _: () = msg_send![animation, setDuration: FADE_IN];
        let _: () = msg_send![animation, setTimingFunction: timing];
        let _: () =
            msg_send![layer, addAnimation: animation, forKey: &*NSString::from_str("page-fade-in")];
    }
}
