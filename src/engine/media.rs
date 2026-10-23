//! Per-page media state and controls through WebKit SPI, as Safari does it.
//! Public WebKit has no page mute and no playing-audio signal. `_isPlayingAudio`
//! and `_hasActiveNowPlayingSession` are key-value observable, so the shell
//! hears about playback without polling, and `_setPageMuted:` silences the
//! whole page, Web Audio and cross-origin frames included.
//!
//! Chosen over public-API workarounds (Rafi, 2026-10-22): Browser isn't App
//! Store bound and is meant to be open source. Every selector is checked with
//! `respondsToSelector:` first, so if WebKit drops one the controls go away
//! instead of crashing.
//!
//! Track skipping has no SPI. It goes through the page's own Media Session
//! handlers instead: WebKit exposes `navigator.mediaSession.callActionHandler`,
//! and calling it from an isolated content world runs the handler the page
//! registered (the same one its media keys use) without anything the page
//! can observe. Handlers can't be listed, so skip controls appear when the
//! page publishes now-playing metadata; an unregistered action is a no-op.
use super::Event;
use async_channel::Sender;
use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObject, Sel},
    sel,
};
use objc2_foundation::{NSError, NSNumber, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};
use std::ffi::c_void;

const OBSERVED: [&str; 2] = ["_isPlayingAudio", "_hasActiveNowPlayingSession"];
/// `_WKMediaAudioMuted` in `_WKMediaMutedState`.
const AUDIO_MUTED: usize = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaState {
    pub playing: bool,
    pub muted: bool,
    /// The page drives a media session (it can be resumed from here).
    pub session: bool,
}

struct Ivars {
    id: u64,
    events: Sender<Event>,
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
            let _ = self.ivars().events.try_send(Event::MediaChanged(self.ivars().id));
        }
    }
);

pub struct Media {
    view: Retained<WKWebView>,
    observer: Option<Retained<Observer>>,
}

fn responds(view: &WKWebView, selector: Sel) -> bool {
    unsafe { msg_send![view, respondsToSelector: selector] }
}

impl Media {
    pub fn install(view: Retained<WKWebView>, id: u64, events: Sender<Event>) -> Self {
        let supported = [
            sel!(_isPlayingAudio),
            sel!(_hasActiveNowPlayingSession),
            sel!(_mediaMutedState),
            sel!(_setPageMuted:),
            sel!(_playPredominantOrNowPlayingMediaSession:),
        ]
        .into_iter()
        .all(|selector| responds(&view, selector));
        let observer = supported.then(|| {
            let mtm = MainThreadMarker::new().expect("Pages are created on the UI thread");
            let observer = Observer::alloc(mtm).set_ivars(Ivars { id, events });
            let observer: Retained<Observer> = unsafe { msg_send![super(observer), init] };
            for key in OBSERVED {
                let key = NSString::from_str(key);
                let _: () = unsafe {
                    msg_send![&*view, addObserver: &*observer, forKeyPath: &*key,
                        options: 0usize, context: std::ptr::null_mut::<c_void>()]
                };
            }
            observer
        });
        Self { view, observer }
    }

    pub fn state(&self) -> MediaState {
        if self.observer.is_none() {
            return MediaState::default();
        }
        let view = &*self.view;
        unsafe {
            let playing: bool = msg_send![view, _isPlayingAudio];
            let session: bool = msg_send![view, _hasActiveNowPlayingSession];
            let muted: usize = msg_send![view, _mediaMutedState];
            MediaState {
                playing,
                muted: muted & AUDIO_MUTED != 0,
                session,
            }
        }
    }

    pub fn set_muted(&self, muted: bool) {
        if self.observer.is_none() {
            return;
        }
        // Keep the capture-muting bits WebKit tracks in the same mask.
        let view = &*self.view;
        unsafe {
            let current: usize = msg_send![view, _mediaMutedState];
            let next = if muted {
                current | AUDIO_MUTED
            } else {
                current & !AUDIO_MUTED
            };
            let _: () = msg_send![view, _setPageMuted: next];
        }
    }

    pub fn pause(&self) {
        let done = RcBlock::new(|| {});
        unsafe {
            self.view
                .pauseAllMediaPlaybackWithCompletionHandler(Some(&done))
        };
    }

    pub fn play(&self) {
        if self.observer.is_none() {
            return;
        }
        let done = RcBlock::new(|_: Bool| {});
        let _: () =
            unsafe { msg_send![&*self.view, _playPredominantOrNowPlayingMediaSession: &*done] };
    }

    /// Whether the page publishes Media Session metadata, and so likely
    /// handles next/previous track. The answer arrives as `Event::MediaSession`.
    pub fn request_session(&self, id: u64, events: Sender<Event>) {
        let reply = RcBlock::new(move |value: *mut AnyObject, _: *mut NSError| {
            let session = unsafe { value.as_ref() }
                .and_then(|value| value.downcast_ref::<NSNumber>())
                .is_some_and(|value| value.as_bool());
            let _ = events.try_send(Event::MediaSession { id, session });
        });
        self.run("!!navigator.mediaSession?.metadata", Some(&reply));
    }

    pub fn skip(&self, forward: bool) {
        let action = if forward {
            "nexttrack"
        } else {
            "previoustrack"
        };
        self.run(
            &format!("navigator.mediaSession?.callActionHandler({{action: '{action}'}})"),
            None,
        );
    }

    fn run(
        &self,
        script: &str,
        reply: Option<&block2::Block<dyn Fn(*mut AnyObject, *mut NSError)>>,
    ) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        // Isolated from the page's globals, but sharing its DOM and session.
        let world =
            unsafe { WKContentWorld::worldWithName(&NSString::from_str("Browser Media"), mtm) };
        unsafe {
            self.view
                .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(script),
                    None,
                    &world,
                    reply,
                );
        }
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        if let Some(observer) = &self.observer {
            for key in OBSERVED {
                let key = NSString::from_str(key);
                let _: () = unsafe {
                    msg_send![&*self.view, removeObserver: &**observer, forKeyPath: &*key]
                };
            }
        }
    }
}
