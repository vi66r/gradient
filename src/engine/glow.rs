//! The page card's glow while its tab is shared with agents: an accent
//! hairline along the card's edge and a soft halo beyond it, breathing
//! faintly. Core Animation draws and animates it beside the page, so the
//! pulse costs the app nothing; GPUI would have to redraw the window for
//! every frame of it.
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use objc2_app_kit::{NSBezierPath, NSColor, NSView};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};

/// How far the halo reaches beyond the card.
const REACH: f64 = 24.;
const HALO: f32 = 0.42;
/// One breath, in and out.
const BREATH: f64 = 2.4;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    pub struct Glow;

    impl Glow {
        /// Purely visual: clicks, scrolls and hovers reach what's beneath.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

fn class(name: &std::ffi::CStr) -> &'static AnyClass {
    AnyClass::get(name).expect("QuartzCore is linked by AppKit")
}

fn rounded(rect: NSRect, radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius)
}

impl Glow {
    /// A glow in `color` (`0xRRGGBB`) for a card with corners of `radius`;
    /// it breathes unless `pulse` is off (Reduce Motion).
    pub fn new(mtm: MainThreadMarker, color: u32, pulse: bool) -> Retained<Self> {
        let view: Retained<Self> =
            unsafe { msg_send![Self::alloc(mtm), initWithFrame: NSRect::ZERO] };
        view.setWantsLayer(true);
        let channel = |shift: u32| f64::from((color >> shift) & 0xff) / 255.;
        let accent =
            NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.);
        let edge = accent.colorWithAlphaComponent(0.6);
        unsafe {
            let root: *mut AnyObject = msg_send![&view, layer];
            let halo: Retained<AnyObject> = msg_send![class(c"CALayer"), layer];
            let _: () = msg_send![&halo, setShadowColor: &*accent.CGColor()];
            let _: () = msg_send![&halo, setShadowOpacity: HALO];
            let _: () = msg_send![&halo, setShadowRadius: 9f64];
            let _: () = msg_send![&halo, setShadowOffset: NSSize::ZERO];
            let line: Retained<AnyObject> = msg_send![class(c"CAShapeLayer"), layer];
            let _: () = msg_send![&line, setFillColor: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![&line, setStrokeColor: &*edge.CGColor()];
            let _: () = msg_send![&line, setLineWidth: 1f64];
            let mask: Retained<AnyObject> = msg_send![class(c"CAShapeLayer"), layer];
            let _: () = msg_send![&mask, setFillRule: &*NSString::from_str("even-odd")];
            let _: () = msg_send![root, addSublayer: &*halo];
            let _: () = msg_send![root, addSublayer: &*line];
            let _: () = msg_send![root, setMask: &*mask];
            if pulse {
                let key = NSString::from_str("opacity");
                let breath: *mut AnyObject =
                    msg_send![class(c"CABasicAnimation"), animationWithKeyPath: &*key];
                let _: () = msg_send![breath, setFromValue: &*NSNumber::new_f64(1.)];
                let _: () = msg_send![breath, setToValue: &*NSNumber::new_f64(0.55)];
                let _: () = msg_send![breath, setDuration: BREATH / 2.];
                let _: () = msg_send![breath, setAutoreverses: true];
                let _: () = msg_send![breath, setRepeatCount: f32::INFINITY];
                let ease = objc2_quartz_core::CAMediaTimingFunction::functionWithControlPoints(
                    0.42, 0., 0.58, 1.,
                );
                let _: () = msg_send![breath, setTimingFunction: &*ease];
                let _: () = msg_send![root, addAnimation: breath, forKey: &*key];
            }
        }
        view
    }

    fn sublayers(
        &self,
    ) -> Option<(
        Retained<AnyObject>,
        Retained<AnyObject>,
        Retained<AnyObject>,
    )> {
        unsafe {
            let root: *mut AnyObject = msg_send![self, layer];
            let root = root.as_ref()?;
            let layers: Retained<objc2_foundation::NSArray<AnyObject>> = msg_send![root, sublayers];
            let mask: Option<Retained<AnyObject>> = msg_send![root, mask];
            Some((layers.objectAtIndex(0), layers.objectAtIndex(1), mask?))
        }
    }

    /// Fits the glow around the card whose inside (the page's host, within
    /// the card's 1 pt border on its left and top) is `host`, in the
    /// superview's coordinates, with corners of `radius`.
    pub fn place(&self, host: NSRect, radius: f64) {
        let frame = NSRect::new(
            NSPoint::new(host.origin.x - REACH, host.origin.y - REACH),
            NSSize::new(host.size.width + 2. * REACH, host.size.height + 2. * REACH),
        );
        if self.frame() == frame {
            return;
        }
        self.setFrame(frame);
        let Some((halo, line, mask)) = self.sublayers() else {
            return;
        };
        let (width, height) = (host.size.width, host.size.height);
        // Local and unflipped: the card's top is the high edge. Its border
        // runs along the left and top only.
        let card = NSRect::new(
            NSPoint::new(REACH - 1., REACH),
            NSSize::new(width + 1., height + 1.),
        );
        // The page shows through the hole; the window's edges hide the
        // card's right and bottom, so the hole runs on past the bottom.
        let hole = NSRect::new(
            NSPoint::new(REACH, REACH - 2. * radius),
            NSSize::new(width, height + 2. * radius),
        );
        let bounds = self.bounds();
        let shape = NSBezierPath::bezierPathWithRect(bounds);
        shape.appendBezierPath(&rounded(hole, radius - 1.));
        let edge = rounded(
            NSRect::new(
                NSPoint::new(card.origin.x + 0.5, card.origin.y + 0.5),
                NSSize::new(card.size.width - 1., card.size.height - 1.),
            ),
            radius - 0.5,
        );
        unsafe {
            // Sizes change with the window: no implicit animation.
            let _: () = msg_send![class(c"CATransaction"), begin];
            let _: () = msg_send![class(c"CATransaction"), setDisableActions: true];
            let _: () = msg_send![&*halo, setFrame: bounds];
            let _: () = msg_send![&*halo, setShadowPath: &*rounded(card, radius).CGPath()];
            let _: () = msg_send![&*line, setFrame: bounds];
            let _: () = msg_send![&*line, setPath: &*edge.CGPath()];
            let _: () = msg_send![&*mask, setFrame: bounds];
            let _: () = msg_send![&*mask, setPath: &*shape.CGPath()];
            let _: () = msg_send![class(c"CATransaction"), commit];
        }
    }

    /// An agent acted in the tab: the halo brightens for a moment.
    pub fn flare(&self) {
        let Some((halo, ..)) = self.sublayers() else {
            return;
        };
        unsafe {
            let key = NSString::from_str("shadowOpacity");
            let flare: *mut AnyObject =
                msg_send![class(c"CABasicAnimation"), animationWithKeyPath: &*key];
            let _: () = msg_send![flare, setFromValue: &*NSNumber::new_f32(0.95)];
            let _: () = msg_send![flare, setToValue: &*NSNumber::new_f32(HALO)];
            let _: () = msg_send![flare, setDuration: 0.9f64];
            let ease = objc2_quartz_core::CAMediaTimingFunction::functionWithControlPoints(
                0.33, 1., 0.68, 1.,
            );
            let _: () = msg_send![flare, setTimingFunction: &*ease];
            let _: () = msg_send![&*halo, addAnimation: flare, forKey: &*key];
        }
    }
}
