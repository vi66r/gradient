//! Web extensions through WebKit's `WKWebExtension` API (macOS 15.4+).
//!
//! Each Space has its own extension controller, as it has its own data
//! store: a saved Space's controller is persistent (keyed by the Space ID, so
//! extension storage and compiled content-blocking rules survive relaunch);
//! the Private Space's is not. Every page in a Space joins its controller.
//!
//! Extensions see each Space as one window holding that Space's tabs. The
//! shell describes its tabs every frame (`sync_tabs`) and this module turns
//! the differences into WebKit's `didOpenTab`/`didCloseTab`/`didActivateTab`/
//! `didChangeTabProperties` calls. WebKit asks about tabs and windows
//! synchronously, so the model lives here and never reaches into the shell;
//! what extensions ask the shell to do (open, activate, close or navigate a
//! tab, prompt for permissions) arrives as `ExtensionEvent`s.
//!
//! Native messaging (`runtime.sendNativeMessage`, `connectNative`) goes to a
//! `NativeMessaging` hook; until one is set, extensions are told no app
//! answers. See docs/extensions.md.
use crate::engine::{Event, ExtensionAction, ExtensionEvent, ExtensionManifest, ExtensionSpec};
use async_channel::Sender;
use block2::{DynBlock, RcBlock};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class,
    rc::{Retained, Weak},
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{NSBitmapImageFileType, NSGraphicsContext, NSImage, NSPopover};
use objc2_foundation::{
    NSArray, NSCopying, NSDate, NSDictionary, NSError, NSPoint, NSRect, NSRectEdge, NSSet, NSSize,
    NSString, NSUInteger, NSURL, NSUUID,
};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionAction, WKWebExtensionContext,
    WKWebExtensionContextPermissionStatus, WKWebExtensionController,
    WKWebExtensionControllerConfiguration, WKWebExtensionControllerDelegate,
    WKWebExtensionDataRecord, WKWebExtensionMatchPattern, WKWebExtensionMessagePort,
    WKWebExtensionTab, WKWebExtensionTabChangedProperties, WKWebExtensionTabConfiguration,
    WKWebExtensionWindow, WKWebExtensionWindowConfiguration, WKWebExtensionWindowState,
    WKWebExtensionWindowType, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
};

const GRANTED: WKWebExtensionContextPermissionStatus =
    WKWebExtensionContextPermissionStatus::GrantedExplicitly;

/// Relays `runtime.sendNativeMessage` and `runtime.connectNative` to the app
/// an extension belongs to (Safari extensions talk to their `.appex`). Both
/// must answer their handler exactly once.
pub trait NativeMessaging {
    fn send(
        &self,
        context: &WKWebExtensionContext,
        application: Option<&NSString>,
        message: &AnyObject,
        reply: RcBlock<dyn Fn(*mut AnyObject, *mut NSError)>,
    );
    fn connect(
        &self,
        context: &WKWebExtensionContext,
        port: &WKWebExtensionMessagePort,
        done: RcBlock<dyn Fn(*mut NSError)>,
    );
}

/// Installs the native-messaging relay for every Space.
#[allow(dead_code)]
pub fn set_native_messaging(relay: Rc<dyn NativeMessaging>) {
    NATIVE.set(Some(relay));
}

/// Where the shell's events go, and how replies to its prompts are awaited.
#[derive(Clone)]
pub struct Shell {
    pub events: Sender<Event>,
    pub foreground: gpui::ForegroundExecutor,
}

/// A tab as the shell describes it to extensions.
pub struct TabSnapshot {
    pub id: u64,
    pub view: Option<Retained<WKWebView>>,
    pub title: String,
    pub url: Option<String>,
    pub loading: bool,
}

struct Loaded {
    install: String,
    context: Retained<WKWebExtensionContext>,
    /// The manifest declares a toolbar action.
    action: bool,
}

struct Space {
    id: [u8; 16],
    private: bool,
    controller: Retained<WKWebExtensionController>,
    // The controller's delegate is weak.
    _delegate: Retained<Delegate>,
    window: Retained<ExtensionWindow>,
    tabs: RefCell<Vec<Retained<ExtensionTab>>>,
    active: Cell<Option<u64>>,
    /// Enabled installs in toolbar order, including ones still loading.
    wanted: RefCell<Vec<ExtensionSpec>>,
    loaded: RefCell<Vec<Loaded>>,
}

type IconCache = HashMap<(String, usize), Arc<[u8]>>;

thread_local! {
    static SPACES: RefCell<HashMap<[u8; 16], Rc<Space>>> = RefCell::new(HashMap::new());
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
    static NATIVE: RefCell<Option<Rc<dyn NativeMessaging>>> = const { RefCell::new(None) };
    /// `tabs.create` requests waiting for the shell's new tab ID, by token.
    static OPENING: RefCell<HashMap<u64, OpenTab>> = RefCell::new(HashMap::new());
    /// New tabs WebKit is waiting for, until the next sync announces them.
    static AWAITING: RefCell<HashMap<u64, OpenTab>> = RefCell::new(HashMap::new());
    static TOKENS: Cell<u64> = const { Cell::new(1) };
    /// Where each extension's toolbar button was last drawn, for its popup.
    static ANCHORS: RefCell<HashMap<String, NSRect>> = RefCell::new(HashMap::new());
    /// Encoded action icons, by install and the image WebKit returned.
    static ICONS: RefCell<IconCache> = RefCell::new(HashMap::new());
}

/// Whether this macOS has WebKit's extension API.
pub fn available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| unsafe {
        objc2::msg_send![objc2::class!(WKWebViewConfiguration), instancesRespondToSelector: objc2::sel!(setWebExtensionController:)]
    })
}

fn space(id: &[u8; 16]) -> Option<Rc<Space>> {
    SPACES.with_borrow(|spaces| spaces.get(id).cloned())
}

fn send(event: ExtensionEvent) {
    SHELL.with_borrow(|shell| {
        if let Some(shell) = shell {
            let _ = shell.events.try_send(Event::Extension(event));
        }
    });
}

fn error(message: &str) -> Retained<NSError> {
    let key = unsafe { objc2_foundation::NSLocalizedDescriptionKey };
    let text = NSString::from_str(message);
    let info: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
        &[key],
        &[&*Retained::into_super(Retained::into_super(text))],
    );
    unsafe {
        NSError::errorWithDomain_code_userInfo(&NSString::from_str("Browser"), 1, Some(&info))
    }
}

fn install_of(context: &WKWebExtensionContext) -> String {
    unsafe { context.uniqueIdentifier() }.to_string()
}

fn tab_object(tab: &Retained<ExtensionTab>) -> Retained<ProtocolObject<dyn WKWebExtensionTab>> {
    ProtocolObject::from_retained(tab.clone())
}

fn window_object(
    window: &Retained<ExtensionWindow>,
) -> Retained<ProtocolObject<dyn WKWebExtensionWindow>> {
    ProtocolObject::from_retained(window.clone())
}

fn app_window(mtm: MainThreadMarker) -> Option<Retained<objc2_app_kit::NSWindow>> {
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    app.mainWindow().or_else(|| app.windows().firstObject())
}

/// A `tabs.create` completion: answered with the tab, or with an error if it
/// is dropped first.
type TabHandler = RcBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>;
struct OpenTab(Option<TabHandler>);
impl OpenTab {
    fn finish(mut self, tab: &ProtocolObject<dyn WKWebExtensionTab>) {
        if let Some(handler) = self.0.take() {
            handler.call((tab as *const _ as *mut _, std::ptr::null_mut()));
        }
    }
}
impl Drop for OpenTab {
    fn drop(&mut self) {
        if let Some(handler) = self.0.take() {
            handler.call((
                std::ptr::null_mut(),
                Retained::as_ptr(&error("Browser didn’t open the tab.")) as *mut _,
            ));
        }
    }
}

pub struct WindowIvars {
    space: [u8; 16],
    private: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = WindowIvars]
    pub struct ExtensionWindow;
    unsafe impl NSObjectProtocol for ExtensionWindow {}
    unsafe impl WKWebExtensionWindow for ExtensionWindow {
        #[unsafe(method_id(tabsForWebExtensionContext:))]
        unsafe fn tabs(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
            let tabs: Vec<_> = space(&self.ivars().space)
                .map(|space| space.tabs.borrow().iter().map(tab_object).collect())
                .unwrap_or_default();
            NSArray::from_retained_slice(&tabs)
        }

        #[unsafe(method_id(activeTabForWebExtensionContext:))]
        unsafe fn active_tab(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
            space(&self.ivars().space).and_then(|space| {
                let active = space.active.get()?;
                space
                    .tabs
                    .borrow()
                    .iter()
                    .find(|tab| tab.ivars().id == active)
                    .map(tab_object)
            })
        }

        #[unsafe(method(windowTypeForWebExtensionContext:))]
        unsafe fn kind(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowType {
            WKWebExtensionWindowType::Normal
        }

        #[unsafe(method(windowStateForWebExtensionContext:))]
        unsafe fn state(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowState {
            let mtm = self.mtm();
            match app_window(mtm) {
                Some(window)
                    if window
                        .styleMask()
                        .contains(objc2_app_kit::NSWindowStyleMask::FullScreen) =>
                {
                    WKWebExtensionWindowState::Fullscreen
                }
                Some(window) if window.isMiniaturized() => WKWebExtensionWindowState::Minimized,
                _ => WKWebExtensionWindowState::Normal,
            }
        }

        #[unsafe(method(isPrivateForWebExtensionContext:))]
        unsafe fn private(&self, _context: &WKWebExtensionContext) -> bool {
            self.ivars().private
        }

        #[unsafe(method(frameForWebExtensionContext:))]
        unsafe fn frame(&self, _context: &WKWebExtensionContext) -> NSRect {
            app_window(self.mtm()).map_or(NSRect::ZERO, |window| window.frame())
        }

        #[unsafe(method(screenFrameForWebExtensionContext:))]
        unsafe fn screen_frame(&self, _context: &WKWebExtensionContext) -> NSRect {
            app_window(self.mtm())
                .and_then(|window| window.screen())
                .map_or(NSRect::ZERO, |screen| screen.frame())
        }

        #[unsafe(method(focusForWebExtensionContext:completionHandler:))]
        unsafe fn focus(
            &self,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            send(ExtensionEvent::FocusSpace {
                space: self.ivars().space,
            });
            handler.call((std::ptr::null_mut(),));
        }
    }
);

impl ExtensionWindow {
    fn new(space: [u8; 16], private: bool, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WindowIvars { space, private });
        unsafe { objc2::msg_send![super(this), init] }
    }
}

#[derive(Default)]
struct TabState {
    view: Option<Retained<WKWebView>>,
    title: String,
    url: Option<String>,
    loading: bool,
}

pub struct TabIvars {
    id: u64,
    space: [u8; 16],
    state: RefCell<TabState>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TabIvars]
    pub struct ExtensionTab;
    unsafe impl NSObjectProtocol for ExtensionTab {}
    unsafe impl WKWebExtensionTab for ExtensionTab {
        #[unsafe(method_id(windowForWebExtensionContext:))]
        unsafe fn window(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            space(&self.ivars().space).map(|space| window_object(&space.window))
        }

        #[unsafe(method(indexInWindowForWebExtensionContext:))]
        unsafe fn index(&self, _context: &WKWebExtensionContext) -> NSUInteger {
            let id = self.ivars().id;
            space(&self.ivars().space)
                .and_then(|space| {
                    space
                        .tabs
                        .borrow()
                        .iter()
                        .position(|tab| tab.ivars().id == id)
                })
                .unwrap_or(usize::MAX)
        }

        #[unsafe(method_id(webViewForWebExtensionContext:))]
        unsafe fn web_view(&self, _context: &WKWebExtensionContext) -> Option<Retained<WKWebView>> {
            self.ivars().state.borrow().view.clone()
        }

        #[unsafe(method_id(titleForWebExtensionContext:))]
        unsafe fn title(&self, _context: &WKWebExtensionContext) -> Option<Retained<NSString>> {
            Some(NSString::from_str(&self.ivars().state.borrow().title))
        }

        #[unsafe(method_id(urlForWebExtensionContext:))]
        unsafe fn url(&self, _context: &WKWebExtensionContext) -> Option<Retained<NSURL>> {
            let state = self.ivars().state.borrow();
            // The page's own address once it has one; WebKit reports
            // redirects before the shell hears of them.
            state
                .view
                .as_ref()
                .and_then(|view| unsafe { view.URL() })
                .or_else(|| {
                    state
                        .url
                        .as_deref()
                        .and_then(|url| NSURL::URLWithString(&NSString::from_str(url)))
                })
        }

        #[unsafe(method(isLoadingCompleteForWebExtensionContext:))]
        unsafe fn loaded(&self, _context: &WKWebExtensionContext) -> bool {
            !self.ivars().state.borrow().loading
        }

        #[unsafe(method(isSelectedForWebExtensionContext:))]
        unsafe fn selected(&self, _context: &WKWebExtensionContext) -> bool {
            space(&self.ivars().space)
                .is_some_and(|space| space.active.get() == Some(self.ivars().id))
        }

        #[unsafe(method(isPlayingAudioForWebExtensionContext:))]
        unsafe fn playing(&self, _context: &WKWebExtensionContext) -> bool {
            let view = self.ivars().state.borrow().view.clone();
            view.is_some_and(|view| super::media::playing_audio(&view))
        }

        #[unsafe(method(activateForWebExtensionContext:completionHandler:))]
        unsafe fn activate(
            &self,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            send(ExtensionEvent::Activate {
                tab: self.ivars().id,
            });
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(closeForWebExtensionContext:completionHandler:))]
        unsafe fn close(
            &self,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            send(ExtensionEvent::Close {
                tab: self.ivars().id,
            });
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(loadURL:forWebExtensionContext:completionHandler:))]
        unsafe fn load(
            &self,
            url: &NSURL,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            match url
                .absoluteString()
                .map(|url| url.to_string())
                .filter(|url| crate::model::allowed_navigation(url))
            {
                Some(url) => {
                    send(ExtensionEvent::Navigate {
                        tab: self.ivars().id,
                        url,
                    });
                    handler.call((std::ptr::null_mut(),));
                }
                None => handler.call((Retained::as_ptr(&error(
                    "Browser opens only web and extension pages in tabs.",
                )) as *mut _,)),
            }
        }

        #[unsafe(method(reloadFromOrigin:forWebExtensionContext:completionHandler:))]
        unsafe fn reload(
            &self,
            origin: bool,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let view = self.ivars().state.borrow().view.clone();
            if let Some(view) = view {
                unsafe {
                    if origin {
                        view.reloadFromOrigin();
                    } else {
                        view.reload();
                    }
                }
            }
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(goBackForWebExtensionContext:completionHandler:))]
        unsafe fn back(
            &self,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let view = self.ivars().state.borrow().view.clone();
            if let Some(view) = view {
                unsafe { view.goBack() };
            }
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(goForwardForWebExtensionContext:completionHandler:))]
        unsafe fn forward(
            &self,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let view = self.ivars().state.borrow().view.clone();
            if let Some(view) = view {
                unsafe { view.goForward() };
            }
            handler.call((std::ptr::null_mut(),));
        }

        // Clicking an extension's toolbar button grants it `activeTab` on
        // the tab in front, as in Chrome and Safari.
        #[unsafe(method(shouldGrantPermissionsOnUserGestureForWebExtensionContext:))]
        unsafe fn grants_on_gesture(&self, _context: &WKWebExtensionContext) -> bool {
            true
        }
    }
);

impl ExtensionTab {
    fn new(id: u64, space: [u8; 16], mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TabIvars {
            id,
            space,
            state: RefCell::default(),
        });
        unsafe { objc2::msg_send![super(this), init] }
    }
}

pub struct DelegateIvars {
    space: [u8; 16],
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    pub struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl WKWebExtensionControllerDelegate for Delegate {
        #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
        unsafe fn open_windows(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
            let windows: Vec<_> = space(&self.ivars().space)
                .map(|space| window_object(&space.window))
                .into_iter()
                .collect();
            NSArray::from_retained_slice(&windows)
        }

        // Each Space is one window, and the only one its extensions know, so
        // it is always the focused (last focused) one.
        #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
        unsafe fn focused_window(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            space(&self.ivars().space).map(|space| window_object(&space.window))
        }

        #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
        unsafe fn open_tab(
            &self,
            _controller: &WKWebExtensionController,
            configuration: &WKWebExtensionTabConfiguration,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>,
        ) {
            let pending = OpenTab(Some(handler.copy()));
            let url = unsafe { configuration.url() }
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string());
            if url
                .as_deref()
                .is_some_and(|url| !crate::model::allowed_navigation(url))
            {
                return; // Dropping `pending` answers with an error.
            }
            let token = TOKENS.get();
            TOKENS.set(token + 1);
            OPENING.with_borrow_mut(|opening| opening.insert(token, pending));
            send(ExtensionEvent::OpenTab {
                space: self.ivars().space,
                url,
                active: unsafe { configuration.shouldBeActive() },
                token,
            });
        }

        #[unsafe(method(webExtensionController:openNewWindowUsingConfiguration:forExtensionContext:completionHandler:))]
        unsafe fn open_window(
            &self,
            _controller: &WKWebExtensionController,
            _configuration: &WKWebExtensionWindowConfiguration,
            _context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionWindow>, *mut NSError)>,
        ) {
            // One window shows one Space at a time; an extension can open
            // tabs, not windows of its own.
            handler.call((
                std::ptr::null_mut(),
                Retained::as_ptr(&error("Browser doesn’t open windows for extensions.")) as *mut _,
            ));
        }

        #[unsafe(method(webExtensionController:promptForPermissions:inTab:forExtensionContext:completionHandler:))]
        unsafe fn prompt_permissions(
            &self,
            _controller: &WKWebExtensionController,
            permissions: &NSSet<NSString>,
            _tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(NonNull<NSSet<NSString>>, *mut NSDate)>,
        ) {
            let requested: Vec<String> = permissions
                .allObjects()
                .iter()
                .map(|permission| permission.to_string())
                .collect();
            let handler = handler.copy();
            let all = permissions.copy();
            self.prompt(context, requested, Vec::new(), move |allow| {
                let empty = NSSet::new();
                let answer = if allow { &all } else { &empty };
                handler.call((NonNull::from(&**answer), std::ptr::null_mut()));
            });
        }

        #[unsafe(method(webExtensionController:promptForPermissionMatchPatterns:inTab:forExtensionContext:completionHandler:))]
        unsafe fn prompt_patterns(
            &self,
            _controller: &WKWebExtensionController,
            patterns: &NSSet<WKWebExtensionMatchPattern>,
            _tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(NonNull<NSSet<WKWebExtensionMatchPattern>>, *mut NSDate)>,
        ) {
            let requested: Vec<String> = patterns
                .allObjects()
                .iter()
                .map(|pattern| unsafe { pattern.string() }.to_string())
                .collect();
            let handler = handler.copy();
            let all = patterns.copy();
            self.prompt(context, Vec::new(), requested, move |allow| {
                let empty = NSSet::new();
                let answer = if allow { &all } else { &empty };
                handler.call((NonNull::from(&**answer), std::ptr::null_mut()));
            });
        }

        #[unsafe(method(webExtensionController:promptForPermissionToAccessURLs:inTab:forExtensionContext:completionHandler:))]
        unsafe fn prompt_urls(
            &self,
            _controller: &WKWebExtensionController,
            urls: &NSSet<NSURL>,
            _tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(NonNull<NSSet<NSURL>>, *mut NSDate)>,
        ) {
            let sites: Vec<String> = urls
                .allObjects()
                .iter()
                .filter_map(|url| url.absoluteString())
                .map(|url| url.to_string())
                .collect();
            let handler = handler.copy();
            let all = urls.copy();
            self.prompt(context, Vec::new(), sites, move |allow| {
                let empty = NSSet::new();
                let answer = if allow { &all } else { &empty };
                handler.call((NonNull::from(&**answer), std::ptr::null_mut()));
            });
        }

        #[unsafe(method(webExtensionController:didUpdateAction:forExtensionContext:))]
        unsafe fn action_updated(
            &self,
            _controller: &WKWebExtensionController,
            _action: &WKWebExtensionAction,
            _context: &WKWebExtensionContext,
        ) {
            send(ExtensionEvent::ActionsChanged);
        }

        #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
        unsafe fn present_popup(
            &self,
            _controller: &WKWebExtensionController,
            action: &WKWebExtensionAction,
            context: &WKWebExtensionContext,
            handler: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            match present(action, &install_of(context), self.mtm()) {
                Ok(()) => handler.call((std::ptr::null_mut(),)),
                Err(message) => handler.call((Retained::as_ptr(&error(message)) as *mut _,)),
            }
        }

        #[unsafe(method(webExtensionController:sendMessage:toApplicationWithIdentifier:forExtensionContext:replyHandler:))]
        unsafe fn native_message(
            &self,
            _controller: &WKWebExtensionController,
            message: &AnyObject,
            application: Option<&NSString>,
            context: &WKWebExtensionContext,
            reply: &DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>,
        ) {
            match NATIVE.with_borrow(Clone::clone) {
                Some(relay) => relay.send(context, application, message, reply.copy()),
                None => reply.call((
                    std::ptr::null_mut(),
                    Retained::as_ptr(&error("No app answers this extension in Browser yet."))
                        as *mut _,
                )),
            }
        }

        #[unsafe(method(webExtensionController:connectUsingMessagePort:forExtensionContext:completionHandler:))]
        unsafe fn native_port(
            &self,
            _controller: &WKWebExtensionController,
            port: &WKWebExtensionMessagePort,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            match NATIVE.with_borrow(Clone::clone) {
                Some(relay) => relay.connect(context, port, done.copy()),
                None => done.call((Retained::as_ptr(&error(
                    "No app answers this extension in Browser yet.",
                )) as *mut _,)),
            }
        }
    }
);

impl Delegate {
    fn new(space: [u8; 16], mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars { space });
        unsafe { objc2::msg_send![super(this), init] }
    }

    /// Asks the shell; `answer` runs exactly once, with `false` when the
    /// shell can't ask (another dialog is open, the Space isn't on screen).
    fn prompt(
        &self,
        context: &WKWebExtensionContext,
        permissions: Vec<String>,
        patterns: Vec<String>,
        answer: impl FnOnce(bool) + 'static,
    ) {
        let Some(shell) = SHELL.with_borrow(Clone::clone) else {
            answer(false);
            return;
        };
        let space = self.ivars().space;
        let install = install_of(context);
        let name = unsafe { context.webExtension().displayName() }
            .map(|name| name.to_string())
            .unwrap_or_default();
        let (reply, receive) = async_channel::bounded(1);
        let event = Event::Extension(ExtensionEvent::Prompt {
            space,
            install: install.clone(),
            name,
            permissions,
            patterns,
            reply,
        });
        if shell.events.try_send(event).is_err() {
            answer(false);
            return;
        }
        let context = Weak::from(context);
        shell
            .foreground
            .spawn(async move {
                let allow = receive.recv().await == Ok(true);
                answer(allow);
                if allow && let Some(context) = context.load() {
                    report_grants(&context);
                }
            })
            .detach();
    }
}

/// Tells the shell what an extension may now use, so it can be saved.
fn report_grants(context: &WKWebExtensionContext) {
    let (permissions, patterns) = unsafe {
        (
            context
                .currentPermissions()
                .allObjects()
                .iter()
                .map(|permission| permission.to_string())
                .collect(),
            context
                .currentPermissionMatchPatterns()
                .allObjects()
                .iter()
                .map(|pattern| pattern.string().to_string())
                .collect(),
        )
    };
    send(ExtensionEvent::Granted {
        install: install_of(context),
        permissions,
        patterns,
    });
}

/// Shows an action's popup under its toolbar button. WebKit's popover sizes
/// itself to the popup page and closes it when dismissed.
fn present(
    action: &WKWebExtensionAction,
    install: &str,
    mtm: MainThreadMarker,
) -> Result<(), &'static str> {
    let popover: Retained<NSPopover> =
        unsafe { action.popupPopover() }.ok_or("This extension has no popup.")?;
    let window = app_window(mtm).ok_or("Browser has no window.")?;
    let content = window.contentView().ok_or("Browser has no window.")?;
    let height = content.bounds().size.height;
    // The shell records anchors in window coordinates with a top-left
    // origin; the content view is unflipped. Without one (a popup the
    // extension opened itself), drop from the window's top-right corner.
    let anchor = ANCHORS.with_borrow(|anchors| anchors.get(install).copied());
    let anchor = anchor.unwrap_or_else(|| {
        NSRect::new(
            NSPoint::new(content.bounds().size.width - 48., 8.),
            NSSize::new(28., 28.),
        )
    });
    let rect = if content.isFlipped() {
        anchor
    } else {
        NSRect::new(
            NSPoint::new(
                anchor.origin.x,
                height - anchor.origin.y - anchor.size.height,
            ),
            anchor.size,
        )
    };
    if popover.isShown() {
        popover.close();
        return Ok(());
    }
    popover.showRelativeToRect_ofView_preferredEdge(rect, &content, NSRectEdge::MinY);
    Ok(())
}

/// The Space's controller, created on first use. Pages join it through
/// `attach`; the shell loads its extensions through `set_extensions`.
fn ensure(
    id: [u8; 16],
    private: bool,
    store: &WKWebsiteDataStore,
    mtm: MainThreadMarker,
) -> Rc<Space> {
    if let Some(space) = space(&id) {
        return space;
    }
    let configuration = unsafe {
        if private {
            WKWebExtensionControllerConfiguration::nonPersistentConfiguration(mtm)
        } else {
            WKWebExtensionControllerConfiguration::configurationWithIdentifier(
                &NSUUID::from_bytes(id),
                mtm,
            )
        }
    };
    unsafe {
        // Extension pages and workers identify as Safari, as Browser's pages
        // do; extensions pick their code paths by user agent (Bitwarden's
        // background crashed without it).
        let pages = WKWebViewConfiguration::new(mtm);
        pages.setApplicationNameForUserAgent(Some(&NSString::from_str(&format!(
            "Version/{} Safari/605.1.15",
            super::safari_version()
        ))));
        configuration.setWebViewConfiguration(Some(&pages));
        // An extension's own requests carry the Space's cookies, never
        // another Space's.
        configuration.setDefaultWebsiteDataStore(Some(store));
    }
    let controller = unsafe {
        WKWebExtensionController::initWithConfiguration(
            WKWebExtensionController::alloc(mtm),
            &configuration,
        )
    };
    let delegate = Delegate::new(id, mtm);
    unsafe { controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    let window = ExtensionWindow::new(id, private, mtm);
    let space = Rc::new(Space {
        id,
        private,
        controller,
        _delegate: delegate,
        window,
        tabs: RefCell::default(),
        active: Cell::new(None),
        wanted: RefCell::default(),
        loaded: RefCell::default(),
    });
    SPACES.with_borrow_mut(|spaces| spaces.insert(id, space.clone()));
    let window = window_object(&space.window);
    unsafe {
        space.controller.didOpenWindow(&window);
        space.controller.didFocusWindow(Some(&window));
    }
    space
}

/// Joins a page to its Space's controller.
pub fn attach(
    configuration: &WKWebViewConfiguration,
    profile: [u8; 16],
    private: bool,
    store: &WKWebsiteDataStore,
    mtm: MainThreadMarker,
) {
    if !available() {
        return;
    }
    let space = ensure(profile, private, store, mtm);
    unsafe { configuration.setWebExtensionController(Some(&space.controller)) };
}

/// Loads a Space's enabled extensions, in toolbar order, and unloads the
/// rest. Grants are applied before an extension starts.
pub fn set_extensions(
    id: [u8; 16],
    private: bool,
    store: &WKWebsiteDataStore,
    specs: Vec<ExtensionSpec>,
    shell: Shell,
    mtm: MainThreadMarker,
) {
    if !available() {
        return;
    }
    SHELL.set(Some(shell));
    let space = ensure(id, private, store, mtm);
    let stale: Vec<Loaded> = {
        let mut loaded = space.loaded.borrow_mut();
        let (keep, stale) = std::mem::take(&mut *loaded)
            .into_iter()
            .partition(|loaded| specs.iter().any(|spec| spec.install == loaded.install));
        *loaded = keep;
        stale
    };
    for loaded in stale {
        if let Err(error) = unsafe {
            space
                .controller
                .unloadExtensionContext_error(&loaded.context)
        } {
            log::warn!("Unloading an extension: {}", error.localizedDescription());
        }
    }
    let pending: Vec<ExtensionSpec> = {
        let loaded = space.loaded.borrow();
        let previous = space.wanted.borrow();
        specs
            .iter()
            .filter(|spec| !loaded.iter().any(|loaded| loaded.install == spec.install))
            // Already being read from disk.
            .filter(|spec| !previous.iter().any(|wanted| wanted.install == spec.install))
            .cloned()
            .collect()
    };
    *space.wanted.borrow_mut() = specs;
    for spec in pending {
        load(&space, spec, mtm);
    }
    order(&space);
    send(ExtensionEvent::ActionsChanged);
}

/// Keeps the loaded list in the order the shell gave.
fn order(space: &Space) {
    let wanted = space.wanted.borrow();
    space.loaded.borrow_mut().sort_by_key(|loaded| {
        wanted
            .iter()
            .position(|spec| spec.install == loaded.install)
            .unwrap_or(usize::MAX)
    });
}

fn load(space: &Rc<Space>, spec: ExtensionSpec, mtm: MainThreadMarker) {
    let url =
        NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&spec.dir.to_string_lossy()), true);
    let weak = Rc::downgrade(space);
    let done = RcBlock::new(
        move |extension: *mut WKWebExtension, failure: *mut NSError| {
            let Some(space) = weak.upgrade() else {
                return;
            };
            // Disabled or removed while it was being read.
            let wanted = space.wanted.borrow().iter().any(|wanted| wanted == &spec);
            if !wanted
                || space
                    .loaded
                    .borrow()
                    .iter()
                    .any(|loaded| loaded.install == spec.install)
            {
                return;
            }
            let result = match unsafe { extension.as_ref() } {
                Some(extension) => start(&space, extension, &spec, mtm),
                None => Err(unsafe { failure.as_ref() }
                    .map(|failure| failure.localizedDescription().to_string())
                    .unwrap_or_else(|| "It couldn’t be read.".into())),
            };
            if let Err(reason) = &result {
                log::warn!("Extension {} didn't load: {reason}", spec.install);
            }
            send(ExtensionEvent::Loaded {
                space: space.id,
                install: spec.install.clone(),
                error: result.err(),
            });
            send(ExtensionEvent::ActionsChanged);
        },
    );
    unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm) };
}

fn start(
    space: &Rc<Space>,
    extension: &WKWebExtension,
    spec: &ExtensionSpec,
    mtm: MainThreadMarker,
) -> Result<(), String> {
    unsafe {
        let context = WKWebExtensionContext::contextForExtension(extension);
        // A stable identity keeps the extension's storage across launches
        // (and is its `runtime.id`).
        context.setUniqueIdentifier(&NSString::from_str(&spec.install));
        if let Some(base) = NSURL::URLWithString(&NSString::from_str(&format!(
            "webkit-extension://{}/",
            spec.install
        ))) {
            context.setBaseURL(&base);
        }
        context.setInspectable(true);
        context.setHasAccessToPrivateData(space.private);
        for permission in &spec.permissions {
            context.setPermissionStatus_forPermission(GRANTED, &NSString::from_str(permission));
        }
        for pattern in &spec.patterns {
            match WKWebExtensionMatchPattern::matchPatternWithString(
                &NSString::from_str(pattern),
                mtm,
            ) {
                Some(pattern) => context.setPermissionStatus_forMatchPattern(GRANTED, &pattern),
                None => log::warn!(
                    "Extension {}: unreadable granted pattern {pattern}",
                    spec.install
                ),
            }
        }
        space
            .controller
            .loadExtensionContext_error(&context)
            .map_err(|error| error.localizedDescription().to_string())?;
        for error in context.errors().iter() {
            log::warn!(
                "Extension {}: {}",
                spec.install,
                error.localizedDescription()
            );
        }
        let action = manifest_has_action(extension);
        space.loaded.borrow_mut().push(Loaded {
            install: spec.install.clone(),
            context,
            action,
        });
    }
    order(space);
    Ok(())
}

fn manifest_has_action(extension: &WKWebExtension) -> bool {
    let manifest = unsafe { extension.manifest() };
    ["action", "browser_action", "page_action"]
        .iter()
        .any(|key| manifest.objectForKey(&NSString::from_str(key)).is_some())
}

/// Reads an extension folder the way WebKit will load it, for the install
/// prompt. The answer arrives on the returned channel.
pub fn inspect(
    dir: &std::path::Path,
    mtm: MainThreadMarker,
) -> async_channel::Receiver<Result<ExtensionManifest, String>> {
    let (send, receive) = async_channel::bounded(1);
    if !available() {
        let _ = send.try_send(Err("Extensions need macOS 15.4 or later.".into()));
        return receive;
    }
    let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&dir.to_string_lossy()), true);
    let done = RcBlock::new(
        move |extension: *mut WKWebExtension, failure: *mut NSError| {
            let result = match unsafe { extension.as_ref() } {
                Some(extension) => unsafe {
                    Ok(ExtensionManifest {
                        name: extension
                            .displayName()
                            .map(|name| name.to_string())
                            .unwrap_or_default(),
                        version: extension
                            .displayVersion()
                            .or_else(|| extension.version())
                            .map(|version| version.to_string())
                            .unwrap_or_default(),
                        permissions: extension
                            .requestedPermissions()
                            .allObjects()
                            .iter()
                            .map(|permission| permission.to_string())
                            .collect(),
                        patterns: extension
                            .requestedPermissionMatchPatterns()
                            .allObjects()
                            .iter()
                            .map(|pattern| pattern.string().to_string())
                            .collect(),
                        problems: extension
                            .errors()
                            .iter()
                            .map(|error| error.localizedDescription().to_string())
                            .collect(),
                    })
                },
                None => Err(unsafe { failure.as_ref() }
                    .map(|failure| failure.localizedDescription().to_string())
                    .unwrap_or_else(|| "It couldn’t be read.".into())),
            };
            let _ = send.try_send(result);
        },
    );
    unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm) };
    receive
}

/// Brings WebKit's view of a Space's tabs up to date with the shell's.
pub fn sync_tabs(id: [u8; 16], tabs: Vec<TabSnapshot>, active: Option<u64>, mtm: MainThreadMarker) {
    let Some(space) = space(&id) else {
        return;
    };
    let mut opened = Vec::new();
    let mut changed = Vec::new();
    let closed: Vec<Retained<ExtensionTab>>;
    {
        let mut current = space.tabs.borrow_mut();
        let mut previous: HashMap<u64, Retained<ExtensionTab>> =
            current.drain(..).map(|tab| (tab.ivars().id, tab)).collect();
        for snapshot in tabs {
            let (tab, new) = match previous.remove(&snapshot.id) {
                Some(tab) => (tab, false),
                None => (ExtensionTab::new(snapshot.id, id, mtm), true),
            };
            let mut properties = WKWebExtensionTabChangedProperties::None;
            {
                let mut state = tab.ivars().state.borrow_mut();
                let same_view = match (&state.view, &snapshot.view) {
                    (Some(old), Some(new)) => Retained::as_ptr(old) == Retained::as_ptr(new),
                    (None, None) => true,
                    _ => false,
                };
                if !same_view {
                    state.view = snapshot.view;
                    properties |= WKWebExtensionTabChangedProperties::Loading
                        | WKWebExtensionTabChangedProperties::URL;
                }
                if state.title != snapshot.title {
                    state.title = snapshot.title;
                    properties |= WKWebExtensionTabChangedProperties::Title;
                }
                if state.url != snapshot.url {
                    state.url = snapshot.url;
                    properties |= WKWebExtensionTabChangedProperties::URL;
                }
                if state.loading != snapshot.loading {
                    state.loading = snapshot.loading;
                    properties |= WKWebExtensionTabChangedProperties::Loading;
                }
            }
            if new {
                opened.push(tab.clone());
            } else if !properties.is_empty() {
                changed.push((tab.clone(), properties));
            }
            current.push(tab);
        }
        closed = previous.into_values().collect();
    }
    let before = space.active.replace(active);
    let previous_active = before.and_then(|before| {
        closed
            .iter()
            .chain(space.tabs.borrow().iter())
            .find(|tab| tab.ivars().id == before)
            .cloned()
    });
    // No borrows are held from here: WebKit asks about tabs and windows
    // while it handles these.
    let controller = &space.controller;
    unsafe {
        for tab in &closed {
            controller.didCloseTab_windowIsClosing(&tab_object(tab), false);
        }
        for tab in &opened {
            let object = tab_object(tab);
            controller.didOpenTab(&object);
            let waiting = AWAITING.with_borrow_mut(|awaiting| awaiting.remove(&tab.ivars().id));
            if let Some(waiting) = waiting {
                waiting.finish(&object);
            }
        }
        for (tab, properties) in &changed {
            controller.didChangeTabProperties_forTab(*properties, &tab_object(tab));
        }
    }
    if before != active {
        let tab = active.and_then(|active| {
            space
                .tabs
                .borrow()
                .iter()
                .find(|tab| tab.ivars().id == active)
                .cloned()
        });
        if let Some(tab) = tab {
            let previous = previous_active.as_ref().map(tab_object);
            unsafe {
                controller.didActivateTab_previousActiveTab(&tab_object(&tab), previous.as_deref())
            };
        }
        send(ExtensionEvent::ActionsChanged);
    }
}

/// The shell opened the tab an extension asked for (`None` if it couldn't).
pub fn tab_opened(token: u64, tab: Option<u64>) {
    let Some(pending) = OPENING.with_borrow_mut(|opening| opening.remove(&token)) else {
        return;
    };
    if let Some(tab) = tab {
        AWAITING.with_borrow_mut(|awaiting| awaiting.insert(tab, pending));
    }
}

/// The toolbar actions for a Space's tab, in toolbar order.
pub fn actions(id: [u8; 16], tab: Option<u64>) -> Vec<ExtensionAction> {
    let Some(space) = space(&id) else {
        return Vec::new();
    };
    let tab = tab.and_then(|tab| {
        space
            .tabs
            .borrow()
            .iter()
            .find(|candidate| candidate.ivars().id == tab)
            .map(tab_object)
    });
    let contexts: Vec<(String, Retained<WKWebExtensionContext>)> = space
        .loaded
        .borrow()
        .iter()
        .filter(|loaded| loaded.action)
        .map(|loaded| (loaded.install.clone(), loaded.context.clone()))
        .collect();
    contexts
        .into_iter()
        .filter_map(|(install, context)| {
            let action = unsafe { context.actionForTab(tab.as_deref()) }?;
            let label = unsafe { action.label() }.to_string();
            let badge = unsafe { action.badgeText() }.to_string();
            let enabled = unsafe { action.isEnabled() };
            let icon = unsafe { action.iconForSize(NSSize::new(16., 16.)) }
                .and_then(|image| icon_png(&install, &image));
            Some(ExtensionAction {
                install,
                label,
                badge,
                enabled,
                icon,
            })
        })
        .collect()
}

/// An action icon as a 32 px PNG (16 pt at 2×), cached per image.
fn icon_png(install: &str, image: &NSImage) -> Option<Arc<[u8]>> {
    let key = (install.to_string(), image as *const NSImage as usize);
    if let Some(png) = ICONS.with_borrow(|icons| icons.get(&key).cloned()) {
        return Some(png);
    }
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace};
    const SIZE: isize = 32;
    let png = unsafe {
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            SIZE,
            SIZE,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )?;
        let graphics = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&graphics));
        image.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::ZERO, NSSize::new(SIZE as f64, SIZE as f64)),
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.,
        );
        NSGraphicsContext::restoreGraphicsState_class();
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?
    };
    let png: Arc<[u8]> = png.to_vec().into();
    ICONS.with_borrow_mut(|icons| icons.insert(key, png.clone()));
    Some(png)
}

/// The user clicked an extension's toolbar button; `anchor` is the button's
/// frame in window coordinates (top-left origin), for its popup.
pub fn perform_action(id: [u8; 16], install: &str, tab: Option<u64>, anchor: NSRect) {
    let Some(space) = space(&id) else {
        return;
    };
    ANCHORS.with_borrow_mut(|anchors| anchors.insert(install.to_string(), anchor));
    let context = space
        .loaded
        .borrow()
        .iter()
        .find(|loaded| loaded.install == install)
        .map(|loaded| loaded.context.clone());
    let tab = tab.and_then(|tab| {
        space
            .tabs
            .borrow()
            .iter()
            .find(|candidate| candidate.ivars().id == tab)
            .map(tab_object)
    });
    if let Some(context) = context {
        unsafe { context.performActionForTab(tab.as_deref()) };
    }
}

/// Erases an extension's storage in a Space and unloads it.
pub fn remove(id: [u8; 16], install: &str, mtm: MainThreadMarker) {
    let Some(space) = space(&id) else {
        return;
    };
    space
        .wanted
        .borrow_mut()
        .retain(|spec| spec.install != install);
    let loaded = {
        let mut loaded = space.loaded.borrow_mut();
        let index = loaded.iter().position(|loaded| loaded.install == install);
        index.map(|index| loaded.remove(index))
    };
    let Some(loaded) = loaded else {
        return;
    };
    let controller = space.controller.clone();
    let context = loaded.context.clone();
    let types = unsafe { WKWebExtensionController::allExtensionDataTypes(mtm) };
    let erase_types = types.clone();
    let done = RcBlock::new(move |record: *mut WKWebExtensionDataRecord| {
        let finished = {
            let (controller, context) = (controller.clone(), context.clone());
            RcBlock::new(move || unload(&controller, &context))
        };
        match unsafe { record.as_ref() } {
            Some(record) => unsafe {
                let records = NSArray::from_slice(&[record]);
                controller.removeDataOfTypes_fromDataRecords_completionHandler(
                    &erase_types,
                    &records,
                    &finished,
                );
            },
            None => unload(&controller, &context),
        }
    });
    unsafe {
        space
            .controller
            .fetchDataRecordOfTypes_forExtensionContext_completionHandler(
                &types,
                &loaded.context,
                &done,
            )
    };
    send(ExtensionEvent::ActionsChanged);
}

fn unload(controller: &WKWebExtensionController, context: &WKWebExtensionContext) {
    if let Err(error) = unsafe { controller.unloadExtensionContext_error(context) } {
        log::warn!("Unloading an extension: {}", error.localizedDescription());
    }
}

/// A Space went away: its controller closes. `erase` (a deleted Space) also
/// removes everything its extensions stored.
pub fn forget(id: [u8; 16], erase: bool, mtm: MainThreadMarker) {
    let Some(space) = SPACES.with_borrow_mut(|spaces| spaces.remove(&id)) else {
        return;
    };
    let window = window_object(&space.window);
    unsafe { space.controller.didCloseWindow(&window) };
    let contexts: Vec<Retained<WKWebExtensionContext>> = space
        .loaded
        .borrow_mut()
        .drain(..)
        .map(|loaded| loaded.context)
        .collect();
    if !erase {
        for context in &contexts {
            unload(&space.controller, context);
        }
        return;
    }
    let controller = space.controller.clone();
    let types = unsafe { WKWebExtensionController::allExtensionDataTypes(mtm) };
    let erase_types = types.clone();
    let done = RcBlock::new(move |records: NonNull<NSArray<WKWebExtensionDataRecord>>| {
        let (controller_done, contexts) = (controller.clone(), contexts.clone());
        let finished = RcBlock::new(move || {
            for context in &contexts {
                unload(&controller_done, context);
            }
        });
        unsafe {
            controller.removeDataOfTypes_fromDataRecords_completionHandler(
                &erase_types,
                records.as_ref(),
                &finished,
            )
        };
    });
    unsafe {
        space
            .controller
            .fetchDataRecordsOfTypes_completionHandler(&types, &done)
    };
    send(ExtensionEvent::ActionsChanged);
}
