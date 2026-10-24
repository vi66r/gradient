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
    /// The page's own color (`0xRRGGBB`), for the control zone above it.
    PageTint(u64, Option<u32>),
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
    Title(u64, String),
    Loading(u64, bool, String),
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

pub struct PageOptions<'a> {
    pub id: u64,
    pub url: &'a str,
    pub private: bool,
    pub profile: [u8; 16],
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
    #[cfg(not(target_os = "macos"))]
    file_gate: FileGate,
    #[cfg(target_os = "macos")]
    media: media::Media,
    /// Observes the page's color for the control strip (`Event::PageTint`).
    #[cfg(target_os = "macos")]
    _theme: theme::Theme,
    visible: Cell<bool>,
    bounds: Cell<Bounds<Pixels>>,
    #[cfg(target_os = "macos")]
    executor: gpui::ForegroundExecutor,
    #[cfg(target_os = "macos")]
    attaching: Cell<bool>,
    #[cfg(target_os = "macos")]
    host: std::cell::RefCell<Option<NativeHost>>,
    /// The top-edge dissolve into the control strip, and its color.
    #[cfg(target_os = "macos")]
    edge: std::cell::RefCell<Option<objc2::rc::Retained<fade::EdgeFade>>>,
    #[cfg(target_os = "macos")]
    edge_color: Cell<u32>,
}

impl Drop for Page {
    fn drop(&mut self) {
        let _ = self.raw.set_visible(false);
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
        } = options;
        let protection_events = events.clone();
        let media_events = events.clone();
        let title_events = events.clone();
        let load_events = events.clone();
        let popup_events = events.clone();
        let navigation_events = events.clone();
        let permission_events = events.clone();
        let file_gate = FileGate::default();
        let navigation_gate = file_gate.clone();
        #[cfg(target_os = "macos")]
        let dropped = protection::Dropped::default();
        #[cfg(target_os = "macos")]
        let drop_record = dropped.clone();
        let builder = wry::WebViewBuilder::new_with_web_context(web_context)
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
                let _ = load_events.try_send(Event::Loading(id, matches!(event, wry::PageLoadEvent::Started), url));
            })
            .with_navigation_handler(move |url| {
                if crate::model::is_file_url(&url) {
                    // Browser applied the file policy before admitting this.
                    return navigation_gate.replace(false);
                }
                let allowed = crate::model::allowed_navigation(&url);
                if !allowed { let _ = navigation_events.try_send(Event::Notice("This prototype supports HTTP and HTTPS links only.".into())); }
                allowed
            })
            .with_new_window_req_handler(move |url, _| {
                if crate::model::allowed_navigation(&url) && url != "about:blank" {
                    let _ = popup_events.try_send(Event::NewTab(id, url));
                }
                wry::NewWindowResponse::Deny
            })
            .with_permission_handler(move |_| {
                let _ = permission_events.try_send(Event::Notice("Site permission blocked. Camera, microphone, and other site grants are not implemented yet.".into()));
                wry::PermissionResponse::Deny
            })
            .with_download_started_handler(move |_, _| {
                let _ = events.try_send(Event::Notice("Downloads are not available in this prototype yet.".into()));
                false
            });
        #[cfg(target_os = "macos")]
        let builder = {
            use wry::WebViewBuilderExtDarwin;
            builder
                .with_data_store_identifier(profile)
                // Pages keep their own drop zones. WebKit's fallback of
                // loading a dropped file becomes new tabs (see `Protection`),
                // which needs the paths the user actually dropped.
                .with_drag_drop_handler(move |event| {
                    if let wry::DragDropEvent::Drop { paths, .. } = event {
                        drop_record.record(paths);
                    }
                    false
                })
        };
        #[cfg(not(target_os = "macos"))]
        let _ = profile;
        #[cfg(target_os = "linux")]
        let raw = {
            linux::initialize(cx)?;
            builder.build_as_child(&linux::Parent::new(window)?)?
        };
        #[cfg(not(target_os = "linux"))]
        let raw = {
            let _ = cx;
            builder.build_as_child(&*window)?
        };
        #[cfg(target_os = "macos")]
        let protection = {
            use wry::WebViewExtMacOS;
            protection::Protection::install(
                &raw.webview(),
                protection::Policy {
                    id,
                    events: protection_events,
                    file_gate,
                    dropped,
                },
                cx,
            )
        };
        #[cfg(target_os = "macos")]
        let media = {
            use wry::WebViewExtMacOS;
            media::Media::install(
                objc2::rc::Retained::into_super(raw.webview()),
                id,
                media_events.clone(),
            )
        };
        #[cfg(target_os = "macos")]
        let theme = {
            use wry::WebViewExtMacOS;
            theme::Theme::install(
                objc2::rc::Retained::into_super(raw.webview()),
                id,
                media_events.clone(),
            )
        };
        #[cfg(not(target_os = "macos"))]
        let _ = (protection_events, media_events);
        let page = Rc::new(Self {
            raw,
            #[cfg(target_os = "macos")]
            _protection: protection,
            #[cfg(not(target_os = "macos"))]
            file_gate,
            #[cfg(target_os = "macos")]
            media,
            #[cfg(target_os = "macos")]
            _theme: theme,
            visible: Cell::new(false),
            bounds: Cell::new(Bounds::default()),
            #[cfg(target_os = "macos")]
            executor: cx.foreground_executor().clone(),
            #[cfg(target_os = "macos")]
            attaching: Cell::new(false),
            #[cfg(target_os = "macos")]
            host: std::cell::RefCell::new(None),
            #[cfg(target_os = "macos")]
            edge: std::cell::RefCell::new(None),
            #[cfg(target_os = "macos")]
            edge_color: Cell::new(0xffffff),
        });
        page.load(url)?;
        Ok(page)
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

    /// The control strip's color above this page, for the edge dissolve.
    pub fn set_edge_color(&self, color: u32) {
        #[cfg(target_os = "macos")]
        {
            self.edge_color.set(color);
            if let Some(edge) = self.edge.borrow().as_ref() {
                edge.set_color(color);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = color;
    }

    /// Fades a newly committed document in.
    pub fn fade_in(&self) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            fade::fade_in(&self.raw.webview());
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
                // The control zone covers the panel's top-left corner, so the
                // page rounds only its bottom-left (MinY is the bottom in an
                // unflipped layer).
                const BOTTOM_LEFT: usize = 1; // kCALayerMinXMinYCorner
                let radius = f64::from(crate::ui::CARD_RADIUS - 1.);
                let _: () = objc2::msg_send![layer, setCornerRadius: radius];
                let _: () = objc2::msg_send![layer, setMaskedCorners: BOTTOM_LEFT];
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
            // Above the page, so it dissolves into the control strip.
            *page.edge.borrow_mut() =
                Some(fade::EdgeFade::attach(&host, mtm, page.edge_color.get()));
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
mod fade;
#[cfg(target_os = "macos")]
mod media;
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
