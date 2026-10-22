//! Gate normal AppKit termination (including Dock Quit), while forwarding the
//! existing GPUI delegate's launch, menu, URL, and shutdown callbacks unchanged.
use super::Event;
use async_channel::Sender;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel},
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply};
use std::cell::Cell;

struct Ivars {
    original: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
    events: Sender<Event>,
    allowed: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct BrowserApplicationDelegate;

    unsafe impl NSObjectProtocol for BrowserApplicationDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.ivars().original.respondsToSelector(selector)
        }
    }

    unsafe impl NSApplicationDelegate for BrowserApplicationDelegate {
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            if self.ivars().allowed.get() {
                NSApplicationTerminateReply::TerminateNow
            } else {
                // Queue rather than re-enter GPUI from an AppKit callback. A
                // full/closed channel fails safe: the app remains open.
                let _ = self.ivars().events.try_send(Event::QuitRequested);
                NSApplicationTerminateReply::TerminateCancel
            }
        }
    }

    impl BrowserApplicationDelegate {
        #[unsafe(method(forwardingTargetForSelector:))]
        fn forward(&self, _selector: Sel) -> *const AnyObject {
            &*self.ivars().original as *const _ as *const AnyObject
        }
    }
);

pub struct TerminationGuard(Retained<BrowserApplicationDelegate>);

impl TerminationGuard {
    pub fn install(events: Sender<Event>) -> Self {
        let mtm = MainThreadMarker::new().expect("App delegate is installed on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        let original = app.delegate().expect("GPUI installed its app delegate");
        let allocated = BrowserApplicationDelegate::alloc(mtm).set_ivars(Ivars {
            original,
            events,
            allowed: Cell::new(false),
        });
        let delegate: Retained<BrowserApplicationDelegate> =
            unsafe { msg_send![super(allocated), init] };
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Self(delegate)
    }

    pub fn allow(&self) {
        self.0.ivars().allowed.set(true);
    }
}

impl Drop for TerminationGuard {
    fn drop(&mut self) {
        let mtm = MainThreadMarker::new().expect("App delegate is confined to the main thread");
        NSApplication::sharedApplication(mtm).setDelegate(Some(&self.0.ivars().original));
    }
}
