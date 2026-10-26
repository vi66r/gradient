//! Keep native web content outside GPUI's texture pipeline: WebKit owns rendering,
//! text input, accessibility, and GPU compositing. The shell only positions it.
use async_channel::Sender;
use gpui::{App, Bounds, Pixels, Point, Window};
use std::{cell::Cell, rc::Rc};

#[derive(Debug)]
pub enum Event {
    QuitRequested,
    /// A page started or stopped playing audio or a media session.
    MediaChanged(u64),
    /// A new main-frame document committed (not same-document navigation).
    Committed(u64),
    /// The page's own color (`0xRRGGBB`), painted behind it; `true` when it
    /// is definite (painted or declared), not just the document background.
    PageTint(u64, Option<u32>, bool),
    /// A new document's first visually non-empty layout.
    Painted(u64),
    /// The color at the page's left edge beside the selected tab's cell
    /// (`Page::sample_side`), or `None` when it isn't one color.
    SideColor(u64, Option<u32>),
    /// The user scrolled or typed somewhere in the app, which may have moved
    /// the page's content (`watch_page_input`).
    PageInput,
    /// Find-in-page matches for `query` in a page.
    FindMatches {
        id: u64,
        query: String,
        matches: Matches,
    },
    /// Whether a page publishes Media Session metadata (track controls).
    MediaSession {
        id: u64,
        session: bool,
    },
    NavigationDecision {
        id: u64,
        reply: Sender<bool>,
    },
    /// A page's main frame asked for a link another app handles. Reply
    /// `true` to open it, `false` if the user declined; drop to ignore.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    AppLink {
        id: u64,
        url: String,
        app: String,
        site: String,
        scheme: String,
        reply: Sender<bool>,
    },
    /// A download got its destination and began writing.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    DownloadStarted {
        id: u64,
        tab: u64,
        name: String,
        path: std::path::PathBuf,
    },
    /// A download finished (`ok`) or failed. Cancelled downloads send nothing.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    DownloadEnded {
        id: u64,
        ok: bool,
    },
    /// A tab's main-frame load turned into a download. `empty` when the tab
    /// has no document to show (it was opened for the download).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    PageBecameDownload {
        id: u64,
        empty: bool,
    },
    /// A site asks to use the camera, microphone, screen or location. Reply
    /// `true` to allow; dropping `reply` denies.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Permission {
        id: u64,
        site: String,
        kinds: Vec<crate::permissions::Kind>,
        reply: Sender<bool>,
    },
    /// A page opened another through WebKit (`window.open`, `target=_blank`):
    /// take its page with `take_opened(id)`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Opened {
        opener: u64,
        id: u64,
        url: String,
        popup: bool,
        background: bool,
    },
    /// The user clicked a site's notification (its Notification Center ID).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotificationClicked(String),
    /// A page closed itself (`window.close()`; WebKit allows it in a window
    /// a script opened, or one with no history).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Closed(u64),
    /// A picture of a page as it was just now (`Page::snapshot`), or `None`
    /// if WebKit couldn't take one.
    Snapshot {
        id: u64,
        image: Option<std::sync::Arc<gpui::RenderImage>>,
    },
    Title(u64, String),
    Loading(u64, bool, String),
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NewTab(u64, String),
    /// Local files dropped on a page, as file URLs, to open in new tabs.
    OpenFiles(u64, Vec<String>),
    Notice(String),
    /// A fetched site icon for a host key, or `None` when none could be used.
    Icon(String, Option<Vec<u8>>),
    /// A primary click in window coordinates (top-left origin) that native
    /// views may consume before GPUI sees it.
    NativeClick(Point<Pixels>),
    AgentResult {
        id: u64,
        epoch: u64,
        origin: String,
        reply: std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>,
        value: String,
    },
    /// Something a web extension did or asked for; see `extensions.rs`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Extension(ExtensionEvent),
}

/// What web extensions tell or ask the shell.
#[derive(Debug)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum ExtensionEvent {
    /// An extension started in a Space, or failed to.
    Loaded {
        space: [u8; 16],
        install: String,
        error: Option<String>,
    },
    /// Toolbar buttons (icon, badge, which ones) may have changed.
    ActionsChanged,
    /// `tabs.create`: open a tab in the Space (the start page without a
    /// URL), then report it with `extension_tab_opened(token, …)`.
    OpenTab {
        space: [u8; 16],
        url: Option<String>,
        active: bool,
        token: u64,
    },
    Navigate {
        tab: u64,
        url: String,
    },
    Activate {
        tab: u64,
    },
    Close {
        tab: u64,
    },
    /// `windows.update({focused: true})`: bring the Space on screen.
    FocusSpace {
        space: [u8; 16],
    },
    /// An extension asks for more than it was granted. Reply `true` to
    /// allow; dropping `reply` denies.
    Prompt {
        space: [u8; 16],
        install: String,
        name: String,
        permissions: Vec<String>,
        /// Match patterns, or the URLs of sites it asks for.
        patterns: Vec<String>,
        reply: Sender<bool>,
    },
    /// What an extension may use now, to be saved.
    Granted {
        install: String,
        permissions: Vec<String>,
        patterns: Vec<String>,
    },
}

/// An extension as WebKit reads its manifest.
#[derive(Clone, Debug, Default)]
pub struct ExtensionManifest {
    pub name: String,
    pub version: String,
    /// Permissions WebKit supports that the manifest requests.
    pub permissions: Vec<String>,
    /// Host match patterns it requests.
    pub patterns: Vec<String>,
    /// Manifest problems WebKit reported (it may still load).
    pub problems: Vec<String>,
}

/// An enabled extension for a Space's controller, with what it was granted.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionSpec {
    pub install: String,
    pub dir: std::path::PathBuf,
    pub permissions: Vec<String>,
    pub patterns: Vec<String>,
}

/// A toolbar button's state for the tab in front.
#[derive(Clone)]
pub struct ExtensionAction {
    pub install: String,
    pub label: String,
    pub badge: String,
    pub enabled: bool,
    /// A PNG, 32 px square.
    pub icon: Option<std::sync::Arc<[u8]>>,
}

/// A tab, as extensions see it.
pub struct ExtensionTab<'a> {
    pub id: u64,
    pub page: Option<&'a Page>,
    pub title: &'a str,
    /// `None` for the start page.
    pub url: Option<&'a str>,
    pub loading: bool,
}

/// Loads a Space's enabled extensions (in toolbar order) and unloads the
/// others. Its pages join its extension controller when they're created.
pub fn set_extensions(
    space: [u8; 16],
    private: bool,
    specs: Vec<ExtensionSpec>,
    events: Sender<Event>,
    cx: &App,
) {
    #[cfg(target_os = "macos")]
    {
        let mtm = objc2::MainThreadMarker::new().expect("Extensions run on the UI thread");
        let shell = extensions::Shell {
            events,
            foreground: cx.foreground_executor().clone(),
        };
        let store = data_store(private, space, mtm);
        extensions::set_extensions(space, private, &store, specs, shell, mtm);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (space, private, specs, events, cx);
}

/// Whether this system can run web extensions at all.
pub fn extensions_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        extensions::available()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Reads an unpacked extension for the install prompt.
pub fn inspect_extension(
    dir: &std::path::Path,
) -> async_channel::Receiver<Result<ExtensionManifest, String>> {
    #[cfg(target_os = "macos")]
    {
        let mtm = objc2::MainThreadMarker::new().expect("Extensions run on the UI thread");
        extensions::inspect(dir, mtm)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dir;
        let (send, receive) = async_channel::bounded(1);
        let _ = send.try_send(Err("Extensions need macOS.".into()));
        receive
    }
}

/// Describes a Space's tabs to its extensions; cheap when nothing changed.
pub fn sync_extension_tabs<'a>(
    space: [u8; 16],
    tabs: impl Iterator<Item = ExtensionTab<'a>>,
    active: Option<u64>,
) {
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewExtMacOS;
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        if !extensions::has_space(&space) {
            return;
        }
        let tabs = tabs
            .map(|tab| extensions::TabSnapshot {
                id: tab.id,
                view: tab
                    .page
                    .map(|page| objc2::rc::Retained::into_super(page.raw.webview())),
                title: tab.title,
                url: tab.url,
                loading: tab.loading,
            })
            .collect();
        extensions::sync_tabs(space, tabs, active, mtm);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (space, tabs.count(), active);
}

/// The shell opened (or couldn't open) the tab `ExtensionEvent::OpenTab`
/// asked for.
pub fn extension_tab_opened(token: u64, tab: Option<u64>) {
    #[cfg(target_os = "macos")]
    extensions::tab_opened(token, tab);
    #[cfg(not(target_os = "macos"))]
    let _ = (token, tab);
}

/// Toolbar buttons for a Space's tab, in toolbar order.
pub fn extension_actions(space: [u8; 16], tab: Option<u64>) -> Vec<ExtensionAction> {
    #[cfg(target_os = "macos")]
    {
        extensions::actions(space, tab)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (space, tab);
        Vec::new()
    }
}

/// A click on an extension's toolbar button. `anchor` is the button in
/// window coordinates, for its popup.
pub fn perform_extension_action(
    space: [u8; 16],
    install: &str,
    tab: Option<u64>,
    anchor: Bounds<Pixels>,
) {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        let rect = NSRect::new(
            NSPoint::new(f64::from(anchor.origin.x), f64::from(anchor.origin.y)),
            NSSize::new(f64::from(anchor.size.width), f64::from(anchor.size.height)),
        );
        extensions::perform_action(space, install, tab, rect);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (space, install, tab, anchor);
}

/// Erases an extension's storage in a Space and stops it, before its files
/// are removed.
pub fn remove_extension(space: [u8; 16], install: &str) {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        extensions::remove(space, install, mtm);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (space, install);
}

/// A Space closed (the Private Space) or was deleted (`erase`): its
/// extension controller goes, and with `erase` everything it stored.
pub fn forget_extensions(space: [u8; 16], erase: bool) {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        extensions::forget(space, erase, mtm);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (space, erase);
}

/// Whether macOS can hand a link to another app, and which one.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum AppHandler {
    App(String),
    None,
    /// This platform build cannot hand links to other apps.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unsupported,
}

pub fn app_handler(url: &str) -> AppHandler {
    #[cfg(target_os = "macos")]
    {
        app_link::handler(url).map_or(AppHandler::None, AppHandler::App)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        AppHandler::Unsupported
    }
}

/// Opens a link in its registered app. Call only after the user confirmed.
pub fn open_app_link(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        app_link::open(url)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        false
    }
}

/// Bytes received by a download in flight, and its total when known.
pub fn download_progress(id: u64) -> Option<(i64, Option<i64>)> {
    #[cfg(target_os = "macos")]
    {
        download::progress(id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        None
    }
}

/// Stops a download and removes its partial file.
pub fn cancel_download(id: u64) {
    #[cfg(target_os = "macos")]
    download::cancel(id);
    #[cfg(not(target_os = "macos"))]
    let _ = id;
}

/// Stops every download in flight, before quitting.
pub fn cancel_downloads() {
    #[cfg(target_os = "macos")]
    download::cancel_all();
}

/// Shows a downloaded file in the file manager.
pub fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    download::reveal(path);
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}
/// How many times a find query matches, and which match is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Matches {
    /// `None` when there are more than WebKit was asked to count.
    pub count: Option<usize>,
    /// Zero-based; `None` when unknown or nothing matched.
    pub index: Option<usize>,
}

/// Which match find in page selects next.
#[cfg(target_os = "macos")]
pub use find::{FindStep, MAX_MATCHES};
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindStep {
    Typed,
    Next,
    Previous,
}
#[cfg(not(target_os = "macos"))]
pub const MAX_MATCHES: usize = 1000;

pub struct PageOptions<'a> {
    pub id: u64,
    pub url: &'a str,
    pub private: bool,
    pub profile: [u8; 16],
    /// Tab IDs, shared with the engine so pages a page opens get one too.
    pub ids: &'a Ids,
}

/// The next free tab ID. Tabs and the pages WebKit opens draw from one counter.
pub type Ids = Rc<Cell<u64>>;

pub fn next_id(ids: &Ids) -> u64 {
    let id = ids.get();
    ids.set(id + 1);
    id
}

/// What a page needs to build the pages it opens. WebKit asks for a popup's
/// view synchronously, outside any GPUI update, so this carries what
/// `Page::create` otherwise takes from the window and app.
#[cfg(target_os = "macos")]
#[derive(Clone)]
struct Host {
    events: Sender<Event>,
    ids: Ids,
    foreground: gpui::ForegroundExecutor,
    background: gpui::BackgroundExecutor,
    parent: Parent,
}

#[cfg(target_os = "macos")]
thread_local! {
    /// Pages WebKit opened, until the shell takes them into a tab.
    static OPENED: std::cell::RefCell<std::collections::HashMap<u64, Rc<Page>>> =
        Default::default();
}

/// The page WebKit opened for `Event::Opened`.
pub fn take_opened(id: u64) -> Option<Rc<Page>> {
    #[cfg(target_os = "macos")]
    {
        OPENED.with(|opened| opened.borrow_mut().remove(&id))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        None
    }
}

#[cfg(target_os = "macos")]
impl Host {
    fn open(
        &self,
        opener: u64,
        private: bool,
        profile: [u8; 16],
        configuration: &objc2_web_kit::WKWebViewConfiguration,
        opening: site::Opening,
    ) -> Option<objc2::rc::Retained<objc2_web_kit::WKWebView>> {
        use wry::WebViewExtMacOS;
        let id = next_id(&self.ids);
        // WebKit requires the returned view to use exactly this configuration.
        let page = match Page::build(
            id,
            private,
            profile,
            Some(objc2::Message::retain(configuration)),
            None,
            self,
        ) {
            Ok(page) => page,
            Err(error) => {
                log::error!("Opening a page: {error}");
                return None;
            }
        };
        let view = objc2::rc::Retained::into_super(page.raw.webview());
        OPENED.with(|opened| opened.borrow_mut().insert(id, page));
        let event = Event::Opened {
            opener,
            id,
            url: opening.url,
            popup: opening.popup,
            background: opening.background,
        };
        if self.events.try_send(event).is_err() {
            take_opened(id);
            return None;
        }
        Some(view)
    }
}

/// The GPUI window's view, as a Wry parent that outlives the call.
#[cfg(target_os = "macos")]
#[derive(Clone)]
struct Parent(objc2::rc::Retained<objc2_app_kit::NSView>);

#[cfg(target_os = "macos")]
impl Parent {
    fn of(window: &Window) -> anyhow::Result<Self> {
        use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window)?.as_raw()
        else {
            anyhow::bail!("Browser needs an AppKit window.");
        };
        // The handle's view belongs to the window and is alive here.
        let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
        Ok(Self(objc2::Message::retain(view)))
    }
}

#[cfg(target_os = "macos")]
impl wry::raw_window_handle::HasWindowHandle for Parent {
    fn window_handle(
        &self,
    ) -> Result<wry::raw_window_handle::WindowHandle<'_>, wry::raw_window_handle::HandleError> {
        use wry::raw_window_handle::{AppKitWindowHandle, RawWindowHandle, WindowHandle};
        let handle = AppKitWindowHandle::new(std::ptr::NonNull::from(&*self.0).cast());
        // The retained view outlives the borrow.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(handle)) })
    }
}

/// The handlers every page's Wry view gets, on every platform.
fn handlers<'a>(
    builder: wry::WebViewBuilder<'a>,
    id: u64,
    private: bool,
    events: &Sender<Event>,
    navigation_gate: FileGate,
) -> wry::WebViewBuilder<'a> {
    let title_events = events.clone();
    let load_events = events.clone();
    let navigation_events = events.clone();
    let download_events = events.clone();
    builder
        .with_visible(false)
        .with_focused(false)
        .with_incognito(private)
        .with_autoplay(false)
        .with_devtools(true)
        .with_back_forward_navigation_gestures(true)
        .with_document_title_changed_handler(move |title| {
            let _ = title_events.try_send(Event::Title(id, title.chars().take(200).collect()));
        })
        .with_on_page_load_handler(move |event, url| {
            let _ = load_events.try_send(Event::Loading(
                id,
                matches!(event, wry::PageLoadEvent::Started),
                url,
            ));
        })
        .with_navigation_handler(move |url| {
            if crate::model::is_file_url(&url) {
                // Browser applied the file policy before admitting this.
                return navigation_gate.replace(false);
            }
            let allowed = crate::model::allowed_navigation(&url);
            if !allowed {
                let _ = navigation_events.try_send(Event::Notice(
                    "This prototype supports HTTP and HTTPS links only.".into(),
                ));
            }
            allowed
        })
        // Registering a handler makes Wry answer WebKit with "download".
        // On macOS, `Protection` then takes the download over (see
        // `download.rs`), so this only runs elsewhere.
        .with_download_started_handler(move |_, _| {
            let _ = download_events.try_send(Event::Notice(
                "Downloads are not available on this platform yet.".into(),
            ));
            false
        })
}

#[cfg(target_os = "macos")]
struct NativeHost(objc2::rc::Retained<objc2_app_kit::NSView>);
#[cfg(target_os = "macos")]
impl Drop for NativeHost {
    fn drop(&mut self) {
        self.0.removeFromSuperview();
    }
}

/// One-shot permission for the next `file:` decision Wry's navigation handler
/// sees. Only Browser sets it: on macOS, `Protection` after applying the file
/// policy; elsewhere, `Page::load_file`. A web page can never set it.
type FileGate = Rc<Cell<bool>>;

pub struct Page {
    pub raw: wry::WebView,
    #[cfg(target_os = "macos")]
    _protection: objc2::rc::Retained<protection::Protection>,
    /// Popups and site permission prompts (WebKit's UI delegate).
    #[cfg(target_os = "macos")]
    _site: objc2::rc::Retained<site::Site>,
    #[cfg(not(target_os = "macos"))]
    file_gate: FileGate,
    #[cfg(target_os = "macos")]
    media: media::Media,
    /// Observes the page's color (`Event::PageTint`).
    #[cfg(target_os = "macos")]
    _theme: theme::Theme,
    #[cfg(target_os = "macos")]
    find: find::Find,
    visible: Cell<bool>,
    bounds: Cell<Bounds<Pixels>>,
    #[cfg(target_os = "macos")]
    executor: gpui::ForegroundExecutor,
    #[cfg(target_os = "macos")]
    attaching: Cell<bool>,
    #[cfg(target_os = "macos")]
    host: std::cell::RefCell<Option<NativeHost>>,
    /// The page's color, painted behind it (`fade::set_backdrop`).
    #[cfg(target_os = "macos")]
    backdrop: Cell<u32>,
}

impl Drop for Page {
    fn drop(&mut self) {
        let _ = self.raw.set_visible(false);
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            notifications::forget(&self.raw.webview());
        }
    }
}

/// Sites a Space allowed to send notifications, for its pages that start later.
pub fn set_notification_sites(profile: [u8; 16], sites: Vec<String>) {
    #[cfg(target_os = "macos")]
    notifications::set_allowed(profile, sites);
    #[cfg(not(target_os = "macos"))]
    let _ = (profile, sites);
}

/// The user allowed or reset a site's notifications in a Space; its open
/// pages learn it now.
pub fn set_notification_site(profile: [u8; 16], site: &str, allowed: bool) {
    #[cfg(target_os = "macos")]
    notifications::set_site(profile, site, allowed);
    #[cfg(not(target_os = "macos"))]
    let _ = (profile, site, allowed);
}

/// Tells the page its notification was clicked; returns the tab to show.
pub fn notification_clicked(identifier: &str) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        notifications::clicked(identifier)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = identifier;
        None
    }
}

impl Page {
    /// Loads an address on the browser's behalf (address field, agent). Only
    /// these loads, Back/Forward and Reload pass through the work guard.
    pub fn load_url(&self, url: &str) -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        self._protection.expect_shell_navigation();
        self.load(url)
    }

    fn load(&self, url: &str) -> anyhow::Result<()> {
        if crate::model::is_file_url(url) {
            self.load_file(url)
        } else {
            Ok(self.raw.load_url(url)?)
        }
    }

    /// WebKit refuses a plain `loadRequest:` of a file URL, so local files
    /// load with read access to their own folder (relative assets work), never
    /// the whole disk. The navigation is marked as Browser's own.
    fn load_file(&self, url: &str) -> anyhow::Result<()> {
        use anyhow::Context;
        let path = crate::model::file_path(url).context("That file address is not valid.")?;
        let access = crate::model::read_access(&path);
        #[cfg(target_os = "macos")]
        {
            use objc2_foundation::{NSString, NSURL};
            use wry::WebViewExtMacOS;
            let target = NSURL::URLWithString(&NSString::from_str(url))
                .context("That file address is not valid.")?;
            let access = NSURL::fileURLWithPath_isDirectory(
                &NSString::from_str(&access.to_string_lossy()),
                access.is_dir(),
            );
            self._protection.expect_file_navigation(path);
            unsafe {
                self.raw
                    .webview()
                    .loadFileURL_allowingReadAccessToURL(&target, &access);
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            // WebKitGTK loads file URIs directly; Wry's handler admits it once.
            let _ = (path, access);
            self.file_gate.set(true);
            Ok(self.raw.load_url(url)?)
        }
    }

    pub fn navigation_pending(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self._protection.navigation_pending()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    pub fn create(
        options: PageOptions<'_>,
        web_context: &mut wry::WebContext,
        events: Sender<Event>,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<Rc<Self>> {
        let PageOptions {
            id,
            url,
            private,
            profile,
            ids,
        } = options;
        #[cfg(target_os = "macos")]
        let page = {
            let _ = web_context;
            let host = Host {
                events,
                ids: ids.clone(),
                foreground: cx.foreground_executor().clone(),
                background: cx.background_executor().clone(),
                parent: Parent::of(window)?,
            };
            // WebKit serves an extension's own pages (options, dashboards)
            // only to views made from that extension's configuration.
            let extension = if crate::model::is_extension_url(url) {
                Some(extensions::page_configuration(profile, url).ok_or_else(|| {
                    anyhow::anyhow!("the extension that owns it isn’t running in this Space.")
                })?)
            } else {
                None
            };
            Self::build(id, private, profile, None, extension, &host)?
        };
        #[cfg(not(target_os = "macos"))]
        let page = {
            let _ = (profile, ids);
            let file_gate = FileGate::default();
            let popup_events = events.clone();
            let permission_events = events.clone();
            let builder = handlers(
                wry::WebViewBuilder::new_with_web_context(web_context),
                id,
                private,
                &events,
                file_gate.clone(),
            )
            .with_new_window_req_handler(move |url, _| {
                if crate::model::allowed_navigation(&url) && url != "about:blank" {
                    let _ = popup_events.try_send(Event::NewTab(id, url));
                }
                wry::NewWindowResponse::Deny
            })
            .with_permission_handler(move |_| {
                let _ = permission_events.try_send(Event::Notice("Site permission blocked. Camera, microphone, and other site grants are not implemented on this platform yet.".into()));
                wry::PermissionResponse::Deny
            });
            linux::initialize(cx)?;
            let raw = builder.build_as_child(&linux::Parent::new(window)?)?;
            Rc::new(Self {
                raw,
                file_gate,
                visible: Cell::new(false),
                bounds: Cell::new(Bounds::default()),
            })
        };
        page.load(url)?;
        Ok(page)
    }

    /// Builds a page's view. `opened` is WebKit's configuration for a page
    /// this one's opener asked for: WebKit loads it itself, and it shares the
    /// opener's process, data store and content controller. `extension` is
    /// an extension context's configuration, for one of its own pages.
    #[cfg(target_os = "macos")]
    fn build(
        id: u64,
        private: bool,
        profile: [u8; 16],
        opened: Option<objc2::rc::Retained<objc2_web_kit::WKWebViewConfiguration>>,
        extension: Option<objc2::rc::Retained<objc2_web_kit::WKWebViewConfiguration>>,
        host: &Host,
    ) -> anyhow::Result<Rc<Self>> {
        use wry::{WebViewBuilderExtMacos, WebViewExtMacOS};
        let file_gate = FileGate::default();
        let dropped = protection::Dropped::default();
        let drop_record = dropped.clone();
        let inherited = opened.is_some();
        let configuration = opened
            .or(extension)
            .unwrap_or_else(|| page_configuration(private, profile));
        let raw = handlers(
            wry::WebViewBuilder::new(),
            id,
            private,
            &host.events,
            file_gate.clone(),
        )
        .with_webview_configuration(configuration)
        // Pages keep their own drop zones. WebKit's fallback of loading a
        // dropped file becomes new tabs (see `Protection`), which needs the
        // paths the user actually dropped.
        .with_drag_drop_handler(move |event| {
            if let wry::DragDropEvent::Drop { paths, .. } = event {
                drop_record.record(paths);
            }
            false
        })
        .build_as_child(&host.parent)?;
        let view = raw.webview();
        let protection = protection::Protection::install(
            &view,
            protection::Policy {
                id,
                events: host.events.clone(),
                file_gate,
                dropped,
            },
            inherited,
            host.foreground.clone(),
            host.background.clone(),
        );
        notifications::install(&view, id, profile, &host.events);
        let opener = host.clone();
        let site = site::Site::install(
            &view,
            site::Policy {
                id,
                events: host.events.clone(),
                foreground: host.foreground.clone(),
                open: Rc::new(move |configuration, opening| {
                    opener.open(id, private, profile, configuration, opening)
                }),
            },
        );
        let media = media::Media::install(
            objc2::rc::Retained::into_super(view.clone()),
            id,
            host.events.clone(),
        );
        let find = find::Find::install(
            objc2::rc::Retained::into_super(view.clone()),
            id,
            host.events.clone(),
        );
        let theme = theme::Theme::install(
            objc2::rc::Retained::into_super(view),
            id,
            host.events.clone(),
        );
        Ok(Rc::new(Self {
            raw,
            _protection: protection,
            _site: site,
            media,
            _theme: theme,
            #[cfg(target_os = "macos")]
            find,
            visible: Cell::new(false),
            bounds: Cell::new(Bounds::default()),
            executor: host.foreground.clone(),
            attaching: Cell::new(false),
            host: std::cell::RefCell::new(None),
            backdrop: Cell::new(0xffffff),
        }))
    }

    /// Stops any camera or microphone capture, after the user withdrew it.
    pub fn stop_capture(&self) {
        #[cfg(target_os = "macos")]
        {
            use objc2_web_kit::WKMediaCaptureState;
            use wry::WebViewExtMacOS;
            let view = self.raw.webview();
            unsafe {
                view.setCameraCaptureState_completionHandler(WKMediaCaptureState::None, None);
                view.setMicrophoneCaptureState_completionHandler(WKMediaCaptureState::None, None);
            }
        }
    }

    pub fn media_state(&self) -> MediaState {
        #[cfg(target_os = "macos")]
        {
            self.media.state()
        }
        #[cfg(not(target_os = "macos"))]
        {
            MediaState::default()
        }
    }

    pub fn set_muted(&self, muted: bool) {
        #[cfg(target_os = "macos")]
        self.media.set_muted(muted);
        #[cfg(not(target_os = "macos"))]
        let _ = muted;
    }

    pub fn play_media(&self) {
        #[cfg(target_os = "macos")]
        self.media.play();
    }

    pub fn pause_media(&self) {
        #[cfg(target_os = "macos")]
        self.media.pause();
    }

    /// The page's color, shown behind it while WebKit has nothing drawn.
    pub fn set_backdrop(&self, color: u32) {
        #[cfg(target_os = "macos")]
        {
            self.backdrop.set(color);
            if let Some(host) = self.host.borrow().as_ref() {
                fade::set_backdrop(&host.0, color, true);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = color;
    }

    /// A new document committed: drop the previous page's edge sample.
    pub fn reset_theme(&self) {
        #[cfg(target_os = "macos")]
        self._theme.reset();
    }

    /// Samples the page's top edge (`Event::PageTint`).
    pub fn sample_theme(&self) {
        #[cfg(target_os = "macos")]
        self._theme.sample();
    }

    /// Samples the page's left edge between `top` and `bottom`, in points
    /// from the page's top (`Event::SideColor`).
    pub fn sample_side(&self, top: f64, bottom: f64) {
        #[cfg(target_os = "macos")]
        self._theme.sample_side(top, bottom);
        #[cfg(not(target_os = "macos"))]
        let _ = (top, bottom);
    }

    /// A new document committed: hold the page out of sight until
    /// `fade_in`; the host's color (the page's) stands in.
    pub fn hold(&self) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            fade::hold(&self.raw.webview());
        }
    }

    /// Fades a held page in.
    pub fn fade_in(&self) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            fade::fade_in(&self.raw.webview());
        }
    }

    /// Selects a match for `query`; counts arrive as `Event::FindMatches`.
    /// Find needs WebKit SPI and is macOS-only for now.
    pub fn find(&self, query: &str, step: FindStep) {
        #[cfg(target_os = "macos")]
        self.find.find(query, step);
        #[cfg(not(target_os = "macos"))]
        let _ = (query, step);
    }

    /// Clears find's overlay and highlights.
    pub fn end_find(&self) {
        #[cfg(target_os = "macos")]
        self.find.end();
    }

    /// Pictures the page as it is now, for the shell to show in its place
    /// while something draws over it; arrives as `Event::Snapshot`.
    pub fn snapshot(&self, id: u64, events: Sender<Event>) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            snapshot::take(&self.raw.webview(), id, events);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = events.try_send(Event::Snapshot { id, image: None });
    }

    /// Scales the page's content (CSS zoom), not just its rendering.
    pub fn set_zoom(&self, zoom: f64) {
        let _ = self.raw.zoom(zoom);
    }

    /// Opens the system print panel as a sheet; its PDF menu saves a PDF.
    /// `title` names the job, and so the suggested PDF file name.
    pub fn print(&self, title: &str) {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::{NSPrintInfo, NSPrintingPaginationMode};
            use objc2_foundation::{NSCopying, NSString};
            use wry::WebViewExtMacOS;
            let view = self.raw.webview();
            let Some(window) = view.window() else {
                return;
            };
            // A copy, so fitting the page width doesn't change the app-wide
            // defaults other print jobs start from.
            let info = NSPrintInfo::sharedPrintInfo().copy();
            info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
            info.setVerticallyCentered(false);
            unsafe {
                let operation = view.printOperationWithPrintInfo(&info);
                operation.setJobTitle(Some(&NSString::from_str(title)));
                operation.setShowsPrintPanel(true);
                operation.setShowsProgressPanel(true);
                // Two widely reported WKWebView pitfalls, both of which print
                // blank pages: a print view left without a frame, and a
                // synchronous `runOperation`. Size it and run it as a sheet.
                if let Some(printing) = operation.view() {
                    printing.setFrame(view.bounds());
                }
                operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                    &window,
                    None,
                    None,
                    std::ptr::null_mut(),
                );
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = title;
            let _ = self.raw.print();
        }
    }

    pub fn skip_track(&self, forward: bool) {
        #[cfg(target_os = "macos")]
        self.media.skip(forward);
        #[cfg(not(target_os = "macos"))]
        let _ = forward;
    }

    pub fn request_media_session(&self, id: u64, events: Sender<Event>) {
        #[cfg(target_os = "macos")]
        self.media.request_session(id, events);
        #[cfg(not(target_os = "macos"))]
        let _ = (id, events);
    }

    pub async fn work_state(
        &self,
        background: gpui::BackgroundExecutor,
    ) -> crate::lifecycle::WorkState {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            protection::assess(
                objc2::rc::Retained::into_super(self.raw.webview()),
                None,
                background,
            )
            .await
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = background;
            crate::lifecycle::WorkState::unknown()
        }
    }

    /// WKWebView has no URL before its first navigation commits, including
    /// after a failed initial load. Wry 0.57's macOS getter unwraps that nil.
    /// Read the nullable native property directly so callers can handle it.
    pub fn url(&self) -> anyhow::Result<String> {
        #[cfg(target_os = "macos")]
        {
            use anyhow::Context;
            use wry::WebViewExtMacOS;
            let url = unsafe { self.raw.webview().URL() }
                .and_then(|url| url.absoluteString())
                .context("The page has no committed URL yet")?;
            Ok(url.to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(self.raw.url()?)
        }
    }

    /// Detect a settled initial-load failure without mistaking an in-flight
    /// navigation for failure. Later navigation failures need delegate support.
    pub fn initial_load_failed(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            let view = self.raw.webview();
            unsafe { !view.isLoading() && view.URL().is_none() }
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Where the page sits, in window coordinates.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds.get()
    }

    pub fn contains(&self, point: Point<Pixels>) -> bool {
        self.visible.get() && self.bounds.get().contains(&point)
    }

    pub fn hide(&self) {
        if self.visible.replace(false) {
            let _ = self.raw.focus_parent();
            let _ = self.raw.set_visible(false);
            #[cfg(target_os = "macos")]
            if let Some(host) = self.host.borrow().as_ref() {
                host.0.setHidden(true);
            }
        }
    }

    pub fn place(self: &Rc<Self>, bounds: Bounds<Pixels>, scale: f32) {
        #[cfg(target_os = "macos")]
        if native_host_needs_update(self, bounds) && !self.attaching.replace(true) {
            let page = self.clone();
            // Reparenting synchronously inside GPUI layout re-enters AppKit's
            // window callbacks while GPUI is borrowed. Run between UI updates.
            self.executor
                .spawn(async move {
                    if let Err(error) = attach_accessible_native_view(&page) {
                        log::error!("Native accessibility: {error}");
                    }
                    page.attaching.set(false);
                })
                .detach();
        }
        if self.bounds.replace(bounds) != bounds {
            #[cfg(target_os = "macos")]
            let rect = {
                let _ = scale;
                wry::Rect {
                    position: wry::dpi::LogicalPosition::new(
                        f64::from(bounds.origin.x),
                        f64::from(bounds.origin.y),
                    )
                    .into(),
                    size: wry::dpi::LogicalSize::new(
                        f64::from(bounds.size.width),
                        f64::from(bounds.size.height),
                    )
                    .into(),
                }
            };
            #[cfg(target_os = "linux")]
            let rect = wry::Rect {
                position: wry::dpi::PhysicalPosition::new(
                    f32::from(bounds.origin.x) * scale,
                    f32::from(bounds.origin.y) * scale,
                )
                .into(),
                size: wry::dpi::PhysicalSize::new(
                    f32::from(bounds.size.width) * scale,
                    f32::from(bounds.size.height) * scale,
                )
                .into(),
            };
            // A docked inspector owns the webview's sub-rectangle. Resize its
            // host instead; WebKit adjusts both children through autoresizing.
            #[cfg(target_os = "macos")]
            if self.host.borrow().is_none() {
                let _ = self.raw.set_bounds(rect);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = self.raw.set_bounds(rect);
        }
        if !self.visible.replace(true) {
            let _ = self.raw.set_visible(true);
        }
    }
}

/// Clicks on a native page go straight to WebKit, so GPUI never learns that its
/// own text field lost focus. Report every primary click to the shell, which
/// checks it against the visible page. Local monitors see only this app.
#[cfg(target_os = "macos")]
pub fn watch_native_clicks(events: Sender<Event>) {
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;
    let handler = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // AppKit delivers monitored events on the main thread.
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return event.as_ptr();
        };
        let native = unsafe { event.as_ref() };
        if let Some(content) = native.window(mtm).and_then(|window| window.contentView()) {
            let location = native.locationInWindow();
            let height = content.frame().size.height;
            let _ = events.try_send(Event::NativeClick(gpui::point(
                gpui::px(location.x as f32),
                gpui::px((height - location.y) as f32),
            )));
        }
        event.as_ptr()
    });
    // The monitor lives as long as the app; AppKit copies the handler.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::LeftMouseDown, &handler)
    };
    std::mem::forget(monitor);
}

#[cfg(target_os = "macos")]
static INPUT_REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The shell took the last `Event::PageInput`; report the next.
pub fn page_input_seen() {
    #[cfg(target_os = "macos")]
    INPUT_REPORTED.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Scrolling and typing reach the page without GPUI seeing them, and either
/// can move its content. Report them so the shell can resample what it
/// matches to the page (`Page::sample_side`).
#[cfg(target_os = "macos")]
pub fn watch_page_input(events: Sender<Event>) {
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;
    let handler = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // One report until the shell takes it: a momentum scroll sends a
        // hundred events a second, which mustn't crowd out the rest.
        if !INPUT_REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let _ = events.try_send(Event::PageInput);
        }
        event.as_ptr()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::ScrollWheel | NSEventMask::KeyDown,
            &handler,
        )
    };
    std::mem::forget(monitor);
}

#[cfg(target_os = "macos")]
fn native_frame(
    container: &objc2_app_kit::NSView,
    bounds: Bounds<Pixels>,
) -> objc2_foundation::NSRect {
    let y = if container.isFlipped() {
        f64::from(bounds.origin.y)
    } else {
        container.bounds().size.height - f64::from(bounds.origin.y) - f64::from(bounds.size.height)
    };
    objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(f64::from(bounds.origin.x), y),
        objc2_foundation::NSSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
    )
}

#[cfg(target_os = "macos")]
fn native_host_needs_update(page: &Page, bounds: Bounds<Pixels>) -> bool {
    use objc2::ClassType;
    use objc2_app_kit::NSView;
    let host = page.host.borrow();
    let Some(host) = host.as_ref() else {
        return true;
    };
    // Page placement and its retained AppKit host are confined to the UI thread.
    let Some(container) = (unsafe { host.0.superview() }) else {
        return true;
    };
    // AccessKit may lazily subclass the content view. Preserve a plain wrapper
    // around both accessible trees even if this happens after first layout.
    if container.class() != NSView::class() {
        return true;
    }
    host.0.isHidden() || host.0.frame() != native_frame(&container, bounds)
}

#[cfg(target_os = "macos")]
thread_local! {
    /// One throwaway store per Private Space, so its tabs share sign-ins with
    /// each other and with nothing else. See `forget_private_store`.
    static PRIVATE_STORES: std::cell::RefCell<
        std::collections::HashMap<[u8; 16], objc2::rc::Retained<objc2_web_kit::WKWebsiteDataStore>>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A Private Space closed: its website data goes with it once its last page
/// is gone.
pub fn forget_private_store(profile: [u8; 16]) {
    #[cfg(target_os = "macos")]
    PRIVATE_STORES.with_borrow_mut(|stores| stores.remove(&profile));
    #[cfg(not(target_os = "macos"))]
    let _ = profile;
}

/// Erases a deleted Space's website data store (cookies, storage, caches).
/// WebKit refuses while a page still uses it; a failure is reported.
pub fn remove_data_store(profile: [u8; 16], events: Sender<Event>) {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSError, NSUUID};
        use objc2_web_kit::WKWebsiteDataStore;
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        let done = block2::RcBlock::new(move |error: *mut NSError| {
            if let Some(error) = unsafe { error.as_ref() } {
                log::warn!(
                    "Removing a Space's data store: {}",
                    error.localizedDescription()
                );
                let _ = events.try_send(Event::Notice(
                    "The deleted Space’s website data couldn’t be erased.".into(),
                ));
            }
        });
        unsafe {
            WKWebsiteDataStore::removeDataStoreForIdentifier_completionHandler(
                &NSUUID::from_bytes(profile),
                &done,
                mtm,
            )
        };
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (profile, events);
}

/// A Space's website data store: one throwaway store per Private Space, the
/// Space's persistent store otherwise.
#[cfg(target_os = "macos")]
fn data_store(
    private: bool,
    profile: [u8; 16],
    mtm: objc2::MainThreadMarker,
) -> objc2::rc::Retained<objc2_web_kit::WKWebsiteDataStore> {
    use objc2_foundation::NSUUID;
    use objc2_web_kit::WKWebsiteDataStore;
    if private {
        PRIVATE_STORES.with_borrow_mut(|stores| {
            stores
                .entry(profile)
                .or_insert_with(|| unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) })
                .clone()
        })
    } else {
        unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&NSUUID::from_bytes(profile), mtm) }
    }
}

/// The page's WebKit configuration. Wry skips its own data-store setup when
/// given one, so this applies the same rules: an ephemeral store for private
/// tabs, the profile's persistent store otherwise. On top of that:
/// - Safari's user-agent suffix, so sites stop treating Browser as an unknown
///   (Gmail warned it was "no longer supported") and Browser blends in with
///   every Safari user instead of standing out;
/// - WebKit's page-top color sampling (SPI, Safari's own settings), which
///   reports the color actually visible at the top of the page. Gmail paints
///   dark inside a white document;
/// - element fullscreen (`requestFullscreen`, video players' fullscreen
///   buttons). WebKit moves the page into a fullscreen window of its own and
///   back into its host on exit; Wry turns this on too, but the page's
///   configuration is Browser's to state;
/// - the Space's web extension controller (see `extensions.rs`).
#[cfg(target_os = "macos")]
fn page_configuration(
    private: bool,
    profile: [u8; 16],
) -> objc2::rc::Retained<objc2_web_kit::WKWebViewConfiguration> {
    use objc2_foundation::NSString;
    use objc2_web_kit::WKWebViewConfiguration;
    let mtm = objc2::MainThreadMarker::new().expect("Pages are created on the UI thread");
    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let store = data_store(private, profile, mtm);
    extensions::attach(&config, profile, private, &store, mtm);
    unsafe {
        config.setWebsiteDataStore(&store);
        config.preferences().setElementFullscreenEnabled(true);
        config.setApplicationNameForUserAgent(Some(&NSString::from_str(&format!(
            "Version/{} Safari/605.1.15",
            safari_version()
        ))));
        // Scripts open windows only from a click or key press, as in Safari;
        // AppKit's WebKit default lets any script open them.
        let preferences = config.preferences();
        preferences.setJavaScriptCanOpenWindowsAutomatically(false);
        // WebKit hides `navigator.mediaDevices` from apps by default (SPI,
        // Safari's own preference). Each use still asks; see `site.rs`.
        let enable = objc2::sel!(_setMediaDevicesEnabled:);
        let supported: bool = objc2::msg_send![&preferences, respondsToSelector: enable];
        if supported {
            let _: () = objc2::msg_send![&preferences, _setMediaDevicesEnabled: true];
        }
        let responds =
            |selector| -> bool { objc2::msg_send![&config, respondsToSelector: selector] };
        if responds(objc2::sel!(_setSampledPageTopColorMaxDifference:))
            && responds(objc2::sel!(_setSampledPageTopColorMinHeight:))
        {
            let _: () = objc2::msg_send![&config, _setSampledPageTopColorMaxDifference: 5.0f64];
            let _: () = objc2::msg_send![&config, _setSampledPageTopColorMinHeight: 10.0f64];
        }
    }
    config
}

/// The installed Safari's version, as Safari reports it in its user agent.
#[cfg(target_os = "macos")]
fn safari_version() -> &'static str {
    use objc2_foundation::{NSBundle, NSString};
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        NSBundle::bundleWithPath(&NSString::from_str("/Applications/Safari.app"))
            .and_then(|bundle| {
                bundle.objectForInfoDictionaryKey(&NSString::from_str("CFBundleShortVersionString"))
            })
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|value| value.to_string())
            .filter(|value| {
                !value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.')
            })
            .unwrap_or_else(|| "26.0".into())
    })
}

/// The window's plain AppKit content container, created on first use: the
/// system sidebar material at the bottom (the frosted chrome), GPUI's view
/// above it, and native page hosts above that.
#[cfg(target_os = "macos")]
fn content_container(
    window: &objc2_app_kit::NSWindow,
    mtm: objc2::MainThreadMarker,
) -> anyhow::Result<objc2::rc::Retained<objc2_app_kit::NSView>> {
    use objc2::{ClassType, MainThreadOnly};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    };
    let content = window
        .contentView()
        .ok_or_else(|| anyhow::anyhow!("Missing content view"))?;
    if content.class() == NSView::class() {
        return Ok(content);
    }
    let fill =
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable;
    let container = NSView::initWithFrame(NSView::alloc(mtm), content.frame());
    window.setContentView(Some(&container));
    content.setFrame(container.bounds());
    content.setAutoresizingMask(fill);
    container.addSubview(&content);
    // Finder/Mail's sidebar material: the desktop shows through, frosted,
    // adapting to light and dark. GPUI draws transparently over it.
    let backdrop =
        NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), container.bounds());
    backdrop.setMaterial(NSVisualEffectMaterial::Sidebar);
    backdrop.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    backdrop.setState(NSVisualEffectState::Active);
    backdrop.setAutoresizingMask(fill);
    container.addSubview_positioned_relativeTo(&backdrop, NSWindowOrderingMode::Below, None);
    Ok(container)
}

/// Installs the frosted backdrop as soon as the window exists, before any
/// page is attached.
#[cfg(target_os = "macos")]
pub fn install_backdrop(window: &Window) {
    if let Some(native) = native_window(window) {
        let mtm = objc2::MainThreadMarker::new().expect("Windows are created on the UI thread");
        if let Err(error) = content_container(&native, mtm) {
            log::error!("Window backdrop: {error}");
        }
    }
}

/// Pins the native window's appearance (frost, WebKit's color scheme) for
/// visual checks, or follows the system with `None`.
#[cfg(target_os = "macos")]
pub fn pin_appearance(window: &Window, dark: Option<bool>) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    };
    if let Some(native) = native_window(window) {
        let appearance = dark.and_then(|dark| unsafe {
            NSAppearance::appearanceNamed(if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            })
        });
        native.setAppearance(appearance.as_deref());
    }
}

#[cfg(target_os = "macos")]
fn native_window(window: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // The handle's view belongs to this window and outlives this call.
    let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
    view.window()
}

/// GPUI's accessibility provider owns its view's children. Put WKWebView beside
/// that view in an ordinary AppKit container so VoiceOver sees both native trees.
#[cfg(target_os = "macos")]
fn attach_accessible_native_view(page: &Page) -> anyhow::Result<()> {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
    use wry::WebViewExtMacOS;
    let mtm = MainThreadMarker::new().expect("WebKit is created on the UI thread");
    unsafe {
        let web = page.raw.webview();
        let parent = web
            .superview()
            .ok_or_else(|| anyhow::anyhow!("Missing webview parent"))?;
        let window = parent
            .window()
            .ok_or_else(|| anyhow::anyhow!("Missing native window"))?;
        let container = content_container(&window, mtm)?;
        if page.host.borrow().is_none() {
            let host = NSView::initWithFrame(NSView::alloc(mtm), web.frame());
            // AppKit resizes the host in the same native resize transaction as
            // the window. GPUI only reconciles layout changes, not every frame.
            host.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            // Clip the page and any docked Inspector to the shell's panel,
            // just inside its 1 px edge; the window's corners clip the rest.
            host.setWantsLayer(true);
            let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![&host, layer];
            if let Some(layer) = layer.as_ref() {
                // The page card rounds its top corners and its bottom-left;
                // the window's corner rounds the bottom-right (MinY is the
                // bottom in an unflipped layer).
                const BOTTOM_LEFT: usize = 1; // kCALayerMinXMinYCorner
                const TOP_LEFT: usize = 4; // kCALayerMinXMaxYCorner
                const TOP_RIGHT: usize = 8; // kCALayerMaxXMaxYCorner
                let radius = f64::from(crate::ui::CARD_RADIUS - 1.);
                let _: () = objc2::msg_send![layer, setCornerRadius: radius];
                let _: () = objc2::msg_send![
                    layer,
                    setMaskedCorners: BOTTOM_LEFT | TOP_LEFT | TOP_RIGHT
                ];
                let _: () = objc2::msg_send![layer, setMasksToBounds: true];
            }
            container.addSubview(&host);
            web.removeFromSuperview();
            host.addSubview(&web);
            web.setFrame(host.bounds());
            web.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            fade::set_backdrop(&host, page.backdrop.get(), false);
            *page.host.borrow_mut() = Some(NativeHost(host));
        }
        if let Some(host) = page.host.borrow().as_ref() {
            if host.0.superview().as_ref() != Some(&container) {
                host.0.removeFromSuperview();
                container.addSubview(&host.0);
            }
            let frame = native_frame(&container, page.bounds.get());
            if host.0.frame() != frame {
                host.0.setFrame(frame);
            }
            if host.0.isHidden() == page.visible.get() {
                host.0.setHidden(!page.visible.get());
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use gpui::{App, Global, Window};
    use wry::raw_window_handle::{
        HandleError, HasWindowHandle, RawWindowHandle, WindowHandle, XlibWindowHandle,
    };
    struct GtkLoop;
    impl Global for GtkLoop {}
    pub fn initialize(cx: &mut App) -> anyhow::Result<()> {
        if cx.has_global::<GtkLoop>() {
            return Ok(());
        }
        gtk::gdk::set_allowed_backends("x11");
        gtk::init()?;
        cx.set_global(GtkLoop);
        cx.spawn(async move |cx| {
            loop {
                while gtk::events_pending() {
                    gtk::main_iteration_do(false);
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
            }
        })
        .detach();
        Ok(())
    }
    pub struct Parent(XlibWindowHandle);
    impl Parent {
        pub fn new(window: &Window) -> anyhow::Result<Self> {
            match HasWindowHandle::window_handle(window)?.as_raw() {
                RawWindowHandle::Xlib(handle) => Ok(Self(handle)),
                RawWindowHandle::Xcb(handle) => {
                    let mut xlib = XlibWindowHandle::new(handle.window.get().into());
                    xlib.visual_id = handle.visual_id.map_or(0, |v| v.get().into());
                    Ok(Self(xlib))
                }
                _ => anyhow::bail!("Browser requires X11 or XWayland on Linux."),
            }
        }
    }
    impl HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            // The GPUI parent window outlives the attached WebView.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xlib(self.0)) })
        }
    }
}

#[cfg(target_os = "macos")]
mod termination;
#[cfg(target_os = "macos")]
pub use termination::TerminationGuard;

#[cfg(target_os = "macos")]
mod protection;

#[cfg(target_os = "macos")]
mod app_link;

#[cfg(target_os = "macos")]
mod download;
#[cfg(target_os = "macos")]
mod notifications;
#[cfg(target_os = "macos")]
mod site;

#[cfg(target_os = "macos")]
mod extensions;
#[cfg(target_os = "macos")]
mod fade;
#[cfg(target_os = "macos")]
mod find;
#[cfg(target_os = "macos")]
mod media;
#[cfg(target_os = "macos")]
mod snapshot;
#[cfg(target_os = "macos")]
mod theme;
#[cfg(target_os = "macos")]
pub use media::MediaState;
/// Media playback is only observable through WebKit on macOS.
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaState {
    pub playing: bool,
    pub muted: bool,
    pub session: bool,
}
