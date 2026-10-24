//! The page's WebKit UI delegate, in front of Wry's: opens popups as Browser
//! pages, closes them when they ask, and asks the user before a site uses the
//! camera, microphone, screen or location. Wry's own handlers answer
//! synchronously and can't see the frame, so they can't prompt.
use crate::{engine::Event, permissions::Kind};
use async_channel::Sender;
use block2::Block;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject, Sel},
};
use objc2_foundation::NSInteger;
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKNavigationAction, WKNavigationType, WKPermissionDecision,
    WKSecurityOrigin, WKUIDelegate, WKWebView, WKWebViewConfiguration, WKWindowFeatures,
};
use std::rc::Rc;

/// A page WebKit wants opened from this one.
pub struct Opening {
    pub url: String,
    /// A script's `window.open`, rather than a `target=_blank` link or form.
    pub popup: bool,
    /// ⌘-click or middle-click: open behind the current tab.
    pub background: bool,
}

/// Builds the opened page from WebKit's configuration (which carries the
/// opener's process, data store and `window.opener`) and returns its view.
pub type Open = Rc<dyn Fn(&WKWebViewConfiguration, Opening) -> Option<Retained<WKWebView>>>;

pub struct Policy {
    pub id: u64,
    pub events: Sender<Event>,
    pub open: Open,
    pub foreground: gpui::ForegroundExecutor,
}

pub struct Ivars {
    original: Retained<ProtocolObject<dyn WKUIDelegate>>,
    policy: Policy,
}

/// WebKit's decision handler, answered exactly once: dropping it denies, so a
/// closed tab or an undelivered prompt never leaves a request hanging.
struct Pending(Option<Box<dyn FnOnce(bool)>>);
impl Pending {
    fn answer(mut self, allow: bool) {
        if let Some(handler) = self.0.take() {
            handler(allow);
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(handler) = self.0.take() {
            handler(false);
        }
    }
}

/// `WKDisplayCapturePermissionDecision` (SPI): deny, or show the system's
/// screen picker.
const DISPLAY_DENY: NSInteger = 0;
const DISPLAY_SCREEN_PROMPT: NSInteger = 1;

fn decision(handler: &Block<dyn Fn(WKPermissionDecision)>) -> Pending {
    let handler = handler.copy();
    Pending(Some(Box::new(move |allow| {
        handler.call((if allow {
            WKPermissionDecision::Grant
        } else {
            WKPermissionDecision::Deny
        },))
    })))
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    pub struct Site;
    unsafe impl NSObjectProtocol for Site {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.ivars().original.respondsToSelector(selector)
        }
    }
    unsafe impl WKUIDelegate for Site {
        // `method_id` returns the view retained, as WebKit expects; its body
        // must end in the value (no early `return`).
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        unsafe fn create(&self, _view: &WKWebView, configuration: &WKWebViewConfiguration, action: &WKNavigationAction, _features: &WKWindowFeatures) -> Option<Retained<WKWebView>> {
            // The opener's navigation policy already admitted this URL (app
            // links and files never get here); an empty one is a script
            // writing into a blank window.
            let url = unsafe { action.request().URL() }.and_then(|url| url.absoluteString()).map(|url| url.to_string()).filter(|url| !url.is_empty()).unwrap_or_else(|| "about:blank".into());
            let kind = unsafe { action.navigationType() };
            let link = kind == WKNavigationType::LinkActivated;
            let modifiers = unsafe { action.modifierFlags() };
            let background = link && (modifiers.contains(objc2_app_kit::NSEventModifierFlags::Command) || unsafe { action.buttonNumber() } == 2);
            let popup = kind == WKNavigationType::Other;
            if crate::model::allowed_navigation(&url) {
                (self.ivars().policy.open)(configuration, Opening { url, popup, background })
            } else {
                None
            }
        }

        #[unsafe(method(webViewDidClose:))]
        unsafe fn did_close(&self, _view: &WKWebView) {
            // WebKit lets a script close only a window a script opened, or
            // one with no history to lose, as Safari and Chrome do.
            let _ = self.ivars().policy.events.try_send(Event::Closed(self.ivars().policy.id));
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        unsafe fn media(&self, view: &WKWebView, origin: &WKSecurityOrigin, frame: &WKFrameInfo, kind: WKMediaCaptureType, handler: &Block<dyn Fn(WKPermissionDecision)>) {
            let kinds = match kind {
                WKMediaCaptureType::Camera => vec![Kind::Camera],
                WKMediaCaptureType::Microphone => vec![Kind::Microphone],
                WKMediaCaptureType::CameraAndMicrophone => vec![Kind::Camera, Kind::Microphone],
                _ => Vec::new(),
            };
            self.ask(view, origin, frame, kinds, decision(handler));
        }
    }
    impl Site {
        // WebKit SPI (WKUIDelegatePrivate), as Safari uses: without it, WebKit
        // denies geolocation and screen capture outright.
        #[unsafe(method(_webView:requestGeolocationPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        unsafe fn location(&self, view: &WKWebView, origin: &WKSecurityOrigin, frame: &WKFrameInfo, handler: &Block<dyn Fn(WKPermissionDecision)>) {
            self.ask(view, origin, frame, vec![Kind::Location], decision(handler));
        }

        #[unsafe(method(_webView:requestDisplayCapturePermissionForOrigin:initiatedByFrame:withSystemAudio:decisionHandler:))]
        unsafe fn display(&self, view: &WKWebView, origin: &WKSecurityOrigin, frame: &WKFrameInfo, _audio: Bool, handler: &Block<dyn Fn(NSInteger)>) {
            let handler = handler.copy();
            let pending = Pending(Some(Box::new(move |allow| {
                handler.call((if allow { DISPLAY_SCREEN_PROMPT } else { DISPLAY_DENY },))
            })));
            self.ask(view, origin, frame, vec![Kind::Screen], pending);
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forward(&self, _selector: Sel) -> *const AnyObject { &*self.ivars().original as *const _ as *const AnyObject }
    }
);

impl Site {
    /// Only the tab's own site may ask. WebKit names the top page's origin
    /// even for an embedded frame's request, so the frame's own origin must
    /// match too: a frame from elsewhere is denied without a prompt.
    fn ask(
        &self,
        view: &WKWebView,
        origin: &WKSecurityOrigin,
        frame: &WKFrameInfo,
        kinds: Vec<Kind>,
        pending: Pending,
    ) {
        let policy = &self.ivars().policy;
        let site_of = |origin: &WKSecurityOrigin| {
            let (protocol, host, port) = unsafe {
                (
                    origin.protocol().to_string(),
                    origin.host().to_string(),
                    origin.port(),
                )
            };
            u16::try_from(port)
                .ok()
                .and_then(|port| crate::permissions::site(&protocol, &host, port))
        };
        let page = unsafe { view.URL() }
            .and_then(|url| url.absoluteString())
            .and_then(|url| url::Url::parse(&url.to_string()).ok())
            .map(|url| url.origin().ascii_serialization());
        let framed = site_of(&*unsafe { frame.securityOrigin() });
        let Some(site) = site_of(origin)
            .filter(|site| page.as_ref() == Some(site) && framed.as_ref() == Some(site))
        else {
            return;
        };
        if kinds.is_empty() {
            return;
        }
        let (reply, answer) = async_channel::bounded(1);
        let event = Event::Permission {
            id: policy.id,
            site,
            kinds,
            reply,
        };
        if policy.events.try_send(event).is_err() {
            return;
        }
        policy
            .foreground
            .spawn(async move {
                let allow = answer.recv().await == Ok(true);
                pending.answer(allow);
            })
            .detach();
    }

    pub fn install(view: &WKWebView, policy: Policy) -> Retained<Self> {
        let mtm = MainThreadMarker::new().unwrap();
        unsafe {
            let object = Self::alloc(mtm).set_ivars(Ivars {
                original: view.UIDelegate().expect("Wry installs a UI delegate"),
                policy,
            });
            let delegate: Retained<Self> = msg_send![super(object), init];
            view.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            delegate
        }
    }
}
