use crate::{engine::Event, lifecycle::WorkState};
use async_channel::Sender;
use block2::{Block, RcBlock};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel},
};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKNavigation, WKNavigationAction, WKNavigationActionPolicy,
    WKNavigationDelegate, WKNavigationType, WKUserScript, WKUserScriptInjectionTime, WKWebView,
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

fn world() -> Retained<WKContentWorld> {
    unsafe {
        WKContentWorld::worldWithName(
            &NSString::from_str("Browser Work Protection"),
            MainThreadMarker::new().unwrap(),
        )
    }
}

pub async fn assess(
    view: Retained<WKWebView>,
    frame: Option<Retained<WKFrameInfo>>,
    background: gpui::BackgroundExecutor,
) -> WorkState {
    let (send, receive) = async_channel::bounded(1);
    let callback = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        let state = if error.is_null() {
            unsafe { value.as_ref() }
                .and_then(|value| value.downcast_ref::<NSString>())
                .map(|value| WorkState::parse(&value.to_string()))
                .unwrap_or_else(WorkState::unknown)
        } else {
            WorkState::unknown()
        };
        let _ = send.try_send(state);
    });
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str("JSON.stringify(globalThis.__browserWorkState())"),
            frame.as_deref(),
            &world(),
            Some(&callback),
        );
    }
    futures_lite::future::race(
        async {
            receive
                .recv()
                .await
                .unwrap_or_else(|_| WorkState::unknown())
        },
        async {
            background.timer(Duration::from_millis(1500)).await;
            WorkState::unknown()
        },
    )
    .await
}

/// Files the user just dropped on this page. If the page doesn't take the drop
/// itself, WebKit loads the file in place; that load opens new tabs instead.
#[derive(Clone, Default)]
pub struct Dropped(Rc<RefCell<Option<DroppedPaths>>>);

type DroppedPaths = (Instant, Vec<PathBuf>);

impl Dropped {
    pub fn record(&self, paths: Vec<PathBuf>) {
        *self.0.borrow_mut() = (!paths.is_empty()).then(|| (Instant::now(), paths));
    }

    fn take_matching(&self, path: &Path) -> Option<Vec<PathBuf>> {
        let mut slot = self.0.borrow_mut();
        let (at, paths) = slot.as_ref()?;
        if at.elapsed() > Duration::from_secs(10) || !paths.iter().any(|dropped| dropped == path) {
            return None;
        }
        slot.take().map(|(_, paths)| paths)
    }
}

/// What a page's navigation delegate needs from `Page::create`.
pub struct Policy {
    pub id: u64,
    pub events: Sender<Event>,
    pub file_gate: super::FileGate,
    pub dropped: Dropped,
}

pub struct Ivars {
    original: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    id: u64,
    events: Sender<Event>,
    file_gate: super::FileGate,
    dropped: Dropped,
    /// The local file Browser itself is loading (address field, drop, open
    /// panel, restore); its main-frame decision is Browser's, not the page's.
    file: RefCell<Option<PathBuf>>,
    foreground: gpui::ForegroundExecutor,
    background: gpui::BackgroundExecutor,
    busy: Rc<Cell<bool>>,
    approved: Rc<RefCell<Option<WorkState>>>,
    /// Set just before the shell (address field, agent) loads a URL, so that
    /// main-frame decision is guarded while the page's own loads are not.
    shell: Cell<bool>,
}

// Every retained WebKit decision must complete exactly once, even when its
// task is dropped or its UI event cannot be delivered.
struct Decision(Option<RcBlock<dyn Fn(WKNavigationActionPolicy)>>);
impl Decision {
    fn take(&mut self) -> RcBlock<dyn Fn(WKNavigationActionPolicy)> {
        self.0.take().unwrap()
    }
}
impl Drop for Decision {
    fn drop(&mut self) {
        if let Some(handler) = self.0.take() {
            handler.call((WKNavigationActionPolicy::Cancel,));
        }
    }
}
struct Busy(Rc<Cell<bool>>);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    pub struct Protection;
    unsafe impl NSObjectProtocol for Protection {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.ivars().original.respondsToSelector(selector)
        }
    }
    unsafe impl WKNavigationDelegate for Protection {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn decide(&self, view: &WKWebView, action: &WKNavigationAction, handler: &Block<dyn Fn(WKNavigationActionPolicy)>) {
            let state = self.ivars();
            // Only the browser discarding a page needs a guard: Back/Forward
            // (buttons, keys, swipes), Reload, and shell-loaded addresses, in
            // the main frame. A page's own links, redirects and subframe loads
            // are the page's business and must never wait on a probe, prompt,
            // or be cancelled because another check is running. A new window
            // does not replace this page; keep Wry's popup policy.
            let frame = unsafe { action.targetFrame() };
            let main = frame.as_ref().is_some_and(|frame| unsafe { frame.isMainFrame() });
            let kind = unsafe { action.navigationType() };
            let shell = main && state.shell.replace(false);
            let browser = shell || kind == WKNavigationType::BackForward || kind == WKNavigationType::Reload;
            let initial = unsafe { view.URL() }.is_none_or(|url| url.absoluteString().is_none_or(|url| url.to_string() == "about:blank"));
            let frame_url = frame.as_ref().and_then(|frame| unsafe { frame.request().URL() }).and_then(|url| url.absoluteString()).map(|url| url.to_string());
            let target_url = unsafe { action.request().URL() }.and_then(|url| url.absoluteString()).map(|url| url.to_string());
            let same_document = match (&frame_url, &target_url) {
                (Some(from), Some(to)) => from != to && from.split('#').next() == to.split('#').next(),
                _ => false,
            };
            // Wry's policy only admits HTTP(S), which would cancel ordinary page
            // machinery: srcdoc, blob and data frames. Top-level data: stays
            // blocked, as in other browsers. Downloads keep Wry's policy.
            let scheme = target_url.as_deref().and_then(|url| url.split_once(':')).map(|(scheme, _)| scheme.to_ascii_lowercase());
            // Local files: Browser may load them (and replay them through its
            // own Back/Forward/Reload), and a local page may link to another.
            // A web page may never navigate to or open one, in any frame.
            let file = scheme.as_deref() == Some("file");
            if file {
                let target = target_url.clone().unwrap_or_default();
                let path = crate::model::file_path(&target);
                let expected = main && path.is_some() && {
                    let mut expected = state.file.borrow_mut();
                    let matches = *expected == path;
                    if matches { expected.take(); }
                    matches
                };
                let history = kind == WKNavigationType::BackForward || kind == WKNavigationType::Reload;
                // `sourceFrame` is declared nonnull but is nil for some
                // app-initiated loads; read it as nullable.
                let source: Option<Retained<WKFrameInfo>> = unsafe { msg_send![action, sourceFrame] };
                let source_url = source.and_then(|frame| unsafe { frame.request().URL() }).and_then(|url| url.absoluteString()).map(|url| url.to_string());
                if main && !expected && !history
                    && let Some(paths) = path.as_deref().and_then(|path| state.dropped.take_matching(path))
                {
                    handler.call((WKNavigationActionPolicy::Cancel,));
                    let urls = paths.iter().filter_map(|path| crate::model::file_url_from_path(path).ok()).collect();
                    let _ = state.events.try_send(Event::OpenFiles(state.id, urls));
                    return;
                }
                if !crate::model::file_navigation_allowed(&target, expected || history, source_url.as_deref()) {
                    handler.call((WKNavigationActionPolicy::Cancel,));
                    let _ = state.events.try_send(Event::Notice("Web pages can’t open files on this computer.".into()));
                    return;
                }
            }
            let embedded = frame.is_some() && !main && matches!(scheme.as_deref(), Some("about" | "blob" | "data"));
            let download = action.respondsToSelector(objc2::sel!(shouldPerformDownload)) && unsafe { action.shouldPerformDownload() };
            if embedded && !download {
                handler.call((WKNavigationActionPolicy::Allow,));
                return;
            }
            if !main || !browser || initial || frame_url.as_deref() == Some("about:blank") || same_document {
                if file { state.file_gate.set(true); }
                unsafe { state.original.webView_decidePolicyForNavigationAction_decisionHandler(view, action, handler); }
                return;
            }
            if state.busy.replace(true) { handler.call((WKNavigationActionPolicy::Cancel,)); return; }
            let busy = Busy(state.busy.clone());
            let (view, action, original) = (view.retain(), action.retain(), state.original.clone());
            let mut decision = Decision(Some(handler.copy()));
            let (events, id, background) = (state.events.clone(), state.id, state.background.clone());
            let approved = state.approved.clone();
            let file_gate = file.then(|| state.file_gate.clone());
            state.foreground.spawn(async move {
                let _busy = busy;
                let before = assess(view.clone(), frame.clone(), background.clone()).await;
                let already_approved = approved.borrow().as_ref().is_some_and(|prior| prior.still_covered(&before));
                if !before.clear() && !already_approved {
                    let (reply, answer) = async_channel::bounded(1);
                    if events.try_send(Event::NavigationDecision { id, reply }).is_err()
                        || answer.recv().await != Ok(true) { return; }
                    let after = assess(view.clone(), frame, background).await;
                    if !before.still_covered(&after) {
                        let _ = events.try_send(Event::Notice("The page changed while the dialog was open. Navigation was canceled; try again.".into()));
                        return;
                    }
                    *approved.borrow_mut() = Some(after);
                }
                // Forward the ORIGINAL action and decision, preserving POST
                // bodies, history entries, reload semantics, and Wry policy.
                if let Some(gate) = file_gate { gate.set(true); }
                unsafe { original.webView_decidePolicyForNavigationAction_decisionHandler(&view, &action, &decision.take()); }
            }).detach();
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        unsafe fn committed(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            self.ivars().approved.borrow_mut().take();
            unsafe { self.ivars().original.webView_didCommitNavigation(view, navigation); }
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        unsafe fn failed_provisional(&self, view: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
            self.ivars().approved.borrow_mut().take();
            let original = &self.ivars().original;
            if original.respondsToSelector(objc2::sel!(webView:didFailProvisionalNavigation:withError:)) {
                unsafe { original.webView_didFailProvisionalNavigation_withError(view, navigation, error); }
            }
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        unsafe fn failed(&self, view: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
            self.ivars().approved.borrow_mut().take();
            let original = &self.ivars().original;
            if original.respondsToSelector(objc2::sel!(webView:didFailNavigation:withError:)) {
                unsafe { original.webView_didFailNavigation_withError(view, navigation, error); }
            }
        }
    }
    impl Protection {
        #[unsafe(method(forwardingTargetForSelector:))]
        fn forward(&self, _selector: Sel) -> *const AnyObject { &*self.ivars().original as *const _ as *const AnyObject }
    }
);

impl Protection {
    pub fn navigation_pending(&self) -> bool {
        self.ivars().busy.get()
    }

    pub fn expect_shell_navigation(&self) {
        self.ivars().shell.set(true);
    }

    pub fn expect_file_navigation(&self, path: PathBuf) {
        *self.ivars().file.borrow_mut() = Some(path);
    }

    pub fn install(view: &WKWebView, policy: Policy, cx: &gpui::App) -> Retained<Self> {
        let Policy {
            id,
            events,
            file_gate,
            dropped,
        } = policy;
        let mtm = MainThreadMarker::new().unwrap();
        unsafe {
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                WKUserScript::alloc(mtm),
                &NSString::from_str(include_str!("work-state.js")),
                WKUserScriptInjectionTime::AtDocumentStart,
                false,
                &world(),
            );
            view.configuration()
                .userContentController()
                .addUserScript(&script);
            let object = Self::alloc(mtm).set_ivars(Ivars {
                original: view
                    .navigationDelegate()
                    .expect("Wry installs a navigation delegate"),
                id,
                events,
                file_gate,
                dropped,
                file: RefCell::new(None),
                foreground: cx.foreground_executor().clone(),
                background: cx.background_executor().clone(),
                busy: Rc::new(Cell::new(false)),
                approved: Rc::new(RefCell::new(None)),
                shell: Cell::new(false),
            });
            let delegate: Retained<Self> = msg_send![super(object), init];
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            delegate
        }
    }
}
