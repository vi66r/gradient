//! Find in page through WebKit SPI, as Safari does it. Public
//! `findString:withConfiguration:` only says whether a match exists. The
//! `_findString:options:maxCount:` SPI also tells a `_WKFindDelegate` how many
//! matches there are and which one is selected, and draws Safari's find
//! overlay: the page dims, matches show through and the current one pops.
//!
//! Same rule as media (Rafi, 2026-10-22): every selector is checked first, and
//! if WebKit drops one, find turns off instead of crashing.
//!
//! WebKit searches from the end of the current selection, so typing "a", "ap",
//! "app" would walk to the next match on every keystroke. A changed query
//! first collapses the selection to its start (from an isolated content
//! world), so the current match stays put while it still matches.
use super::{Event, Matches};
use async_channel::Sender;
use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, Sel},
    sel,
};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};

/// `_WKFindOptions`.
const CASE_INSENSITIVE: usize = 1 << 0;
const BACKWARDS: usize = 1 << 3;
const WRAP_AROUND: usize = 1 << 4;
const SHOW_OVERLAY: usize = 1 << 5;
const SHOW_FIND_INDICATOR: usize = 1 << 6;
const SHOW_HIGHLIGHT: usize = 1 << 7;
const DETERMINE_MATCH_INDEX: usize = 1 << 9;

/// Counting stops here; WebKit reports `NSNotFound` beyond it.
pub const MAX_MATCHES: usize = 1000;

/// Which match to select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindStep {
    /// The query changed: keep the current match if it still matches.
    Typed,
    Next,
    Previous,
}

struct Ivars {
    id: u64,
    events: Sender<Event>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(_webView:didFindMatches:forString:withMatchIndex:))]
        fn found(&self, _view: *mut AnyObject, count: usize, query: *mut NSString, index: isize) {
            let count = (count <= MAX_MATCHES).then_some(count);
            self.report(query, Matches { count, index: usize::try_from(index).ok() });
        }

        #[unsafe(method(_webView:didFailToFindString:))]
        fn failed(&self, _view: *mut AnyObject, query: *mut NSString) {
            self.report(query, Matches { count: Some(0), index: None });
        }
    }
);

impl Delegate {
    fn report(&self, query: *mut NSString, matches: Matches) {
        let Some(query) = (unsafe { query.as_ref() }) else {
            return;
        };
        let _ = self.ivars().events.try_send(Event::FindMatches {
            id: self.ivars().id,
            query: query.to_string(),
            matches,
        });
    }
}

pub struct Find {
    view: Retained<WKWebView>,
    /// WebKit holds the delegate weakly.
    delegate: Option<Retained<Delegate>>,
}

fn responds(view: &WKWebView, selector: Sel) -> bool {
    unsafe { msg_send![view, respondsToSelector: selector] }
}

impl Find {
    pub fn install(view: Retained<WKWebView>, id: u64, events: Sender<Event>) -> Self {
        let supported = [
            sel!(_findString:options:maxCount:),
            sel!(_hideFindUI),
            sel!(_setFindDelegate:),
        ]
        .into_iter()
        .all(|selector| responds(&view, selector));
        let delegate = supported.then(|| {
            let mtm = MainThreadMarker::new().expect("Pages are created on the UI thread");
            let delegate = Delegate::alloc(mtm).set_ivars(Ivars { id, events });
            let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
            let _: () = unsafe { msg_send![&*view, _setFindDelegate: &*delegate] };
            delegate
        });
        Self { view, delegate }
    }

    /// Selects a match for `query`; the count arrives as `Event::FindMatches`.
    pub fn find(&self, query: &str, step: FindStep) {
        if self.delegate.is_none() {
            return;
        }
        let options = CASE_INSENSITIVE
            | WRAP_AROUND
            | SHOW_OVERLAY
            | SHOW_FIND_INDICATOR
            | SHOW_HIGHLIGHT
            | DETERMINE_MATCH_INDEX
            | if step == FindStep::Previous {
                BACKWARDS
            } else {
                0
            };
        let query = NSString::from_str(query);
        if step != FindStep::Typed {
            search(&self.view, &query, options);
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let view = self.view.clone();
        let then = RcBlock::new(move |_: *mut AnyObject, _: *mut NSError| {
            search(&view, &query, options);
        });
        let world =
            unsafe { WKContentWorld::worldWithName(&NSString::from_str("Gradient Find"), mtm) };
        unsafe {
            self.view
                .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(
                        "{ const s = getSelection(); if (s && s.rangeCount) s.collapseToStart(); }",
                    ),
                    None,
                    &world,
                    Some(&then),
                );
        }
    }

    /// Clears the overlay and highlights.
    pub fn end(&self) {
        if self.delegate.is_some() {
            let _: () = unsafe { msg_send![&*self.view, _hideFindUI] };
        }
    }
}

fn search(view: &WKWebView, query: &NSString, options: usize) {
    let _: () =
        unsafe { msg_send![view, _findString: query, options: options, maxCount: MAX_MATCHES] };
}

impl Drop for Find {
    fn drop(&mut self) {
        if self.delegate.is_some() {
            let _: () =
                unsafe { msg_send![&*self.view, _setFindDelegate: std::ptr::null::<AnyObject>()] };
        }
    }
}
