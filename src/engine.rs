//! Keep native web content outside GPUI's texture pipeline: WebKit owns rendering,
//! text input, accessibility, and GPU compositing. The shell only positions it.
use async_channel::Sender;
use gpui::{App, Bounds, Pixels, Window};
use std::{cell::Cell, rc::Rc};

#[derive(Debug)]
pub enum Event {
    Title(u64, String),
    Loading(u64, bool, String),
    NewTab(u64, String),
    Notice(String),
    AgentResult {
        id: u64,
        epoch: u64,
        origin: String,
        reply: std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>,
        value: String,
    },
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

pub struct Page {
    pub raw: wry::WebView,
    visible: Cell<bool>,
    bounds: Cell<Bounds<Pixels>>,
    #[cfg(target_os = "macos")]
    executor: gpui::ForegroundExecutor,
    #[cfg(target_os = "macos")]
    attaching: Cell<bool>,
    #[cfg(target_os = "macos")]
    host: std::cell::RefCell<Option<NativeHost>>,
}

impl Drop for Page {
    fn drop(&mut self) {
        let _ = self.raw.set_visible(false);
    }
}

impl Page {
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
        let title_events = events.clone();
        let load_events = events.clone();
        let popup_events = events.clone();
        let navigation_events = events.clone();
        let permission_events = events.clone();
        let builder = wry::WebViewBuilder::new_with_web_context(web_context)
            .with_url(url)
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
            builder.with_data_store_identifier(profile)
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
        Ok(Rc::new(Self {
            raw,
            visible: Cell::new(false),
            bounds: Cell::new(Bounds::default()),
            #[cfg(target_os = "macos")]
            executor: cx.foreground_executor().clone(),
            #[cfg(target_os = "macos")]
            attaching: Cell::new(false),
            #[cfg(target_os = "macos")]
            host: std::cell::RefCell::new(None),
        }))
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

/// GPUI's accessibility provider owns its view's children. Put WKWebView beside
/// that view in an ordinary AppKit container so VoiceOver sees both native trees.
#[cfg(target_os = "macos")]
fn attach_accessible_native_view(page: &Page) -> anyhow::Result<()> {
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly};
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
        let content = window
            .contentView()
            .ok_or_else(|| anyhow::anyhow!("Missing content view"))?;
        let container = if content.class() != NSView::class() {
            let container = NSView::initWithFrame(NSView::alloc(mtm), content.frame());
            window.setContentView(Some(&container));
            content.setFrame(container.bounds());
            content.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            container.addSubview(&content);
            container
        } else {
            content
        };
        if page.host.borrow().is_none() {
            let host = NSView::initWithFrame(NSView::alloc(mtm), web.frame());
            // AppKit resizes the host in the same native resize transaction as
            // the window. GPUI only reconciles layout changes, not every frame.
            host.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            container.addSubview(&host);
            web.removeFromSuperview();
            host.addSubview(&web);
            web.setFrame(host.bounds());
            web.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
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
