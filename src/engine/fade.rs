//! Native finishing for the page's arrival. WebKit draws above GPUI, so the
//! shell can't paint over the page itself:
//! - `set_backdrop` paints the page's host in the page's own color, which
//!   shows while WebKit has nothing to draw.
//! - `fade_in` brings a newly committed page up from transparent.
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use objc2_app_kit::{NSColor, NSView};
use objc2_foundation::{NSNumber, NSString};

/// The page's fade-in and backdrop change: GPUI's ease-out-cubic, as a Bézier.
pub const MOTION: f64 = 0.24;
const CURVE: [f32; 4] = [0.33, 1., 0.68, 1.];

fn curve() -> Retained<objc2_quartz_core::CAMediaTimingFunction> {
    let [x1, y1, x2, y2] = CURVE;
    objc2_quartz_core::CAMediaTimingFunction::functionWithControlPoints(x1, y1, x2, y2)
}

fn layer(view: &NSView) -> Option<&AnyObject> {
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        layer.as_ref()
    }
}

/// Paints `backdrop`, the page's host, in the page's color (`0xRRGGBB`).
/// WebKit draws nothing for a moment when a navigation starts and until the
/// new page arrives; the host's color shows then instead of a white flash.
/// `animated` eases from the previous color.
pub fn set_backdrop(backdrop: &NSView, color: u32, animated: bool) {
    backdrop.setWantsLayer(true);
    let Some(layer) = layer(backdrop) else {
        return;
    };
    let channel = |shift: u32| f64::from((color >> shift) & 0xff) / 255.;
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.);
    unsafe {
        let before: *mut AnyObject = msg_send![layer, backgroundColor];
        let _: () = msg_send![layer, setBackgroundColor: &*color.CGColor()];
        if animated
            && !before.is_null()
            && let Some(class) = AnyClass::get(c"CABasicAnimation")
        {
            let key = NSString::from_str("backgroundColor");
            let animation: *mut AnyObject = msg_send![class, animationWithKeyPath: &*key];
            let _: () = msg_send![animation, setFromValue: before];
            let _: () = msg_send![animation, setToValue: &*color.CGColor()];
            let _: () = msg_send![animation, setDuration: MOTION];
            let _: () = msg_send![animation, setTimingFunction: &*curve()];
            let _: () = msg_send![layer, addAnimation: animation, forKey: &*key];
        }
    }
}

/// A new document committed: hide `view` until WebKit presents it, so it
/// never shows unfinished. The host's color stands in meanwhile.
pub fn hold(view: &NSView) {
    // AppKit owns a view's layer opacity; `alphaValue` is the way to set it.
    view.setAlphaValue(0.);
}

/// Fades a held `view` in over its host.
pub fn fade_in(view: &NSView) {
    let Some(class) = AnyClass::get(c"CABasicAnimation") else {
        return;
    };
    view.setAlphaValue(1.);
    let Some(page) = layer(view) else {
        return;
    };
    unsafe {
        let animation: *mut AnyObject =
            msg_send![class, animationWithKeyPath: &*NSString::from_str("opacity")];
        let _: () = msg_send![animation, setFromValue: &*NSNumber::new_f64(0.)];
        let _: () = msg_send![animation, setToValue: &*NSNumber::new_f64(1.)];
        let _: () = msg_send![animation, setDuration: MOTION];
        let _: () = msg_send![animation, setTimingFunction: &*curve()];
        let _: () =
            msg_send![page, addAnimation: animation, forKey: &*NSString::from_str("page-fade-in")];
    }
}
