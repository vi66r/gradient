//! Web notifications. WebKit hands a page's `new Notification(…)` to its
//! process pool's notification provider, a C SPI that Safari uses, resolved
//! here at runtime: if WebKit drops a symbol, notifications stay denied.
//! Browser shows them through Notification Center; a click fires the page's
//! `onclick` and brings its tab to the front. Ephemeral (private) stores deny
//! notifications inside WebKit, so they never get here.
use crate::engine::Event;
use async_channel::Sender;
use block2::{Block, RcBlock};
use objc2::{
    AllocAnyThread, DefinedClass, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol},
};
use objc2_foundation::{NSBundle, NSError, NSString, NSUInteger};
use objc2_web_kit::WKWebView;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::{CStr, c_char, c_void},
    sync::OnceLock,
};

type Ref = *const c_void;

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// WebKit's C API for notifications (`WKNotificationManager.h`,
/// `WKNotification.h`, `WKNotificationProvider.h`).
struct Api {
    set_provider: unsafe extern "C" fn(Ref, *const Provider),
    did_show: unsafe extern "C" fn(Ref, u64),
    did_click: unsafe extern "C" fn(Ref, u64),
    did_update_policy: unsafe extern "C" fn(Ref, Ref, bool),
    did_remove_policies: unsafe extern "C" fn(Ref, Ref),
    page_context: unsafe extern "C" fn(Ref) -> Ref,
    context_manager: unsafe extern "C" fn(Ref) -> Ref,
    title: unsafe extern "C" fn(Ref) -> Ref,
    body: unsafe extern "C" fn(Ref) -> Ref,
    notification_id: unsafe extern "C" fn(Ref) -> u64,
    origin: unsafe extern "C" fn(Ref) -> Ref,
    origin_string: unsafe extern "C" fn(Ref) -> Ref,
    origin_from_string: unsafe extern "C" fn(Ref) -> Ref,
    string_create: unsafe extern "C" fn(*const c_char) -> Ref,
    string_copy_cf: unsafe extern "C" fn(Ref, Ref) -> *mut NSString,
    dictionary_create: unsafe extern "C" fn() -> Ref,
    dictionary_set: unsafe extern "C" fn(Ref, Ref, Ref) -> bool,
    boolean_create: unsafe extern "C" fn(bool) -> Ref,
    array_create: unsafe extern "C" fn(*const Ref, usize) -> Ref,
    array_size: unsafe extern "C" fn(Ref) -> usize,
    array_item: unsafe extern "C" fn(Ref, usize) -> Ref,
    uint64_value: unsafe extern "C" fn(Ref) -> u64,
    release: unsafe extern "C" fn(Ref),
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let find = |name: &CStr| {
            // RTLD_DEFAULT: WebKit is already linked into the process.
            let found = unsafe { dlsym(-2isize as *mut c_void, name.as_ptr()) };
            (!found.is_null()).then_some(found)
        };
        // Each field's own type says which C signature the symbol has.
        fn function<F: Copy>(symbol: *mut c_void) -> F {
            assert_eq!(size_of::<F>(), size_of::<*mut c_void>());
            unsafe { std::mem::transmute_copy(&symbol) }
        }
        macro_rules! load {
            ($($field:ident = $name:literal),* $(,)?) => {
                Some(Api { $($field: function(find($name)?),)* })
            };
        }
        let api = load!(
            set_provider = c"WKNotificationManagerSetProvider",
            did_show = c"WKNotificationManagerProviderDidShowNotification",
            did_click = c"WKNotificationManagerProviderDidClickNotification",
            did_update_policy = c"WKNotificationManagerProviderDidUpdateNotificationPolicy",
            did_remove_policies = c"WKNotificationManagerProviderDidRemoveNotificationPolicies",
            page_context = c"WKPageGetContext",
            context_manager = c"WKContextGetNotificationManager",
            title = c"WKNotificationCopyTitle",
            body = c"WKNotificationCopyBody",
            notification_id = c"WKNotificationGetID",
            origin = c"WKNotificationGetSecurityOrigin",
            origin_string = c"WKSecurityOriginCopyToString",
            origin_from_string = c"WKSecurityOriginCreateFromString",
            string_create = c"WKStringCreateWithUTF8CString",
            string_copy_cf = c"WKStringCopyCFString",
            dictionary_create = c"WKMutableDictionaryCreate",
            dictionary_set = c"WKDictionarySetItem",
            boolean_create = c"WKBooleanCreate",
            array_create = c"WKArrayCreate",
            array_size = c"WKArrayGetSize",
            array_item = c"WKArrayGetItemAtIndex",
            uint64_value = c"WKUInt64GetValue",
            release = c"WKRelease",
        );
        if api.is_none() {
            log::warn!("WebKit's notification SPI is unavailable; notifications stay off");
        }
        api
    })
    .as_ref()
}

/// `WKNotificationProviderV0`.
#[repr(C)]
struct Provider {
    version: i32,
    client_info: *const c_void,
    show: extern "C" fn(Ref, Ref, *const c_void),
    cancel: extern "C" fn(Ref, *const c_void),
    did_destroy: extern "C" fn(Ref, *const c_void),
    add_manager: extern "C" fn(Ref, *const c_void),
    remove_manager: extern "C" fn(Ref, *const c_void),
    permissions: extern "C" fn(*const c_void) -> Ref,
    clear: extern "C" fn(Ref, *const c_void),
}

// Only its function pointers are read, and `client_info` is null.
unsafe impl Sync for Provider {}

static PROVIDER: Provider = Provider {
    version: 0,
    client_info: std::ptr::null(),
    show,
    cancel,
    did_destroy,
    add_manager,
    remove_manager,
    permissions,
    clear,
};

/// One shown notification: which pool's manager owns it, and its tab.
struct Shown {
    manager: usize,
    notification: u64,
    tab: Option<u64>,
}

#[derive(Default)]
struct State {
    /// `WKPageRef` → tab ID.
    pages: HashMap<usize, u64>,
    /// Every pool's notification manager, to tell them of policy changes.
    managers: HashSet<usize>,
    /// Sites allowed to notify, as origins (`https://chat.example`).
    allowed: Vec<String>,
    /// Notification Center identifier → the notification behind it.
    shown: HashMap<String, Shown>,
    center: Option<Retained<AnyObject>>,
    _delegate: Option<Retained<Clicks>>,
    authorized: bool,
}

thread_local! {
    // WebKit calls the provider on the main thread, where pages live.
    static STATE: RefCell<State> = RefCell::default();
}

fn string(api: &Api, value: Ref) -> String {
    if value.is_null() {
        return String::new();
    }
    let copied = unsafe { (api.string_copy_cf)(std::ptr::null(), value) };
    unsafe { (api.release)(value) };
    // CFString and NSString are toll-free bridged; the copy is +1.
    unsafe { Retained::from_raw(copied) }
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn page_ref(view: &WKWebView) -> Option<usize> {
    let selector = objc2::sel!(_pageRefForTransitionToWKWebView);
    let responds: bool = unsafe { msg_send![view, respondsToSelector: selector] };
    responds.then(|| {
        let page: *const c_void = unsafe { msg_send![view, _pageRefForTransitionToWKWebView] };
        page as usize
    })
}

fn manager_of(api: &Api, page: usize) -> usize {
    unsafe { (api.context_manager)((api.page_context)(page as Ref)) as usize }
}

/// Lets a page's pool deliver notifications and remembers which tab it is.
/// Each configuration has its own pool; a popup shares its opener's.
pub fn install(view: &WKWebView, tab: u64, events: &Sender<Event>) {
    let (Some(api), Some(page)) = (api(), page_ref(view)) else {
        return;
    };
    let manager = manager_of(api, page);
    let new = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.pages.insert(page, tab);
        !state.managers.contains(&manager)
    });
    // Outside the borrow: WebKit calls `add_manager` back from inside.
    if new {
        unsafe { (api.set_provider)(manager as Ref, &PROVIDER) };
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.managers.insert(manager);
        if state.center.is_none() {
            state.center = notification_center();
            if let Some(center) = &state.center {
                let delegate = Clicks::new(events.clone());
                let _: () = unsafe { msg_send![center, setDelegate: &*delegate] };
                state._delegate = Some(delegate);
            }
        }
    });
}

/// The page is going away; its notifications already went with it.
pub fn forget(view: &WKWebView) {
    if let Some(page) = page_ref(view) {
        STATE.with(|state| state.borrow_mut().pages.remove(&page));
    }
}

/// The sites remembered as allowed, read by each pool as it starts.
pub fn set_allowed(sites: Vec<String>) {
    STATE.with(|state| state.borrow_mut().allowed = sites);
}

/// The user allowed (or reset) a site: tell every pool, so pages already
/// open read the new `Notification.permission`.
pub fn set_site(site: &str, allowed: bool) {
    let Some(api) = api() else {
        return;
    };
    let managers: Vec<usize> = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.allowed.retain(|known| known != site);
        if allowed {
            state.allowed.push(site.to_string());
        }
        state.managers.iter().copied().collect()
    });
    let Ok(text) = std::ffi::CString::new(site) else {
        return;
    };
    unsafe {
        let text = (api.string_create)(text.as_ptr());
        let origin = (api.origin_from_string)(text);
        (api.release)(text);
        for manager in managers {
            if allowed {
                (api.did_update_policy)(manager as Ref, origin, true);
            } else {
                let origins = (api.array_create)(&origin, 1);
                (api.did_remove_policies)(manager as Ref, origins);
                (api.release)(origins);
            }
        }
        (api.release)(origin);
    }
    if allowed {
        authorize();
    }
}

/// A notification the user clicked: tell its page and return its tab.
pub fn clicked(identifier: &str) -> Option<u64> {
    let api = api()?;
    let shown = STATE.with(|state| state.borrow_mut().shown.remove(identifier))?;
    unsafe { (api.did_click)(shown.manager as Ref, shown.notification) };
    shown.tab
}

/// Notification Center is only reachable from the app bundle; a bare binary
/// (`cargo run`) has no bundle identity and the center would throw.
fn notification_center() -> Option<Retained<AnyObject>> {
    let bundle = NSBundle::mainBundle();
    let bundled =
        bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app");
    let class = AnyClass::get(c"UNUserNotificationCenter")?;
    if !bundled {
        log::warn!("Notifications need the app bundle; this run can't show them");
        return None;
    }
    unsafe { msg_send![class, currentNotificationCenter] }
}

/// Asks macOS once per run; it prompts only the first time.
fn authorize() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(center) = state.center.clone() else {
            return;
        };
        if std::mem::replace(&mut state.authorized, true) {
            return;
        }
        const ALERT_AND_SOUND: NSUInteger = (1 << 2) | (1 << 1);
        let done = RcBlock::new(|granted: objc2::runtime::Bool, _error: *mut NSError| {
            if !granted.as_bool() {
                log::warn!("macOS notifications are turned off for Browser");
            }
        });
        let _: () = unsafe {
            msg_send![&center, requestAuthorizationWithOptions: ALERT_AND_SOUND, completionHandler: &*done]
        };
    });
}

fn identifier(notification: u64) -> String {
    format!("web-{notification}")
}

extern "C" fn show(page: Ref, notification: Ref, _: *const c_void) {
    let Some(api) = api() else {
        return;
    };
    let id = unsafe { (api.notification_id)(notification) };
    let title = string(api, unsafe { (api.title)(notification) });
    let body = string(api, unsafe { (api.body)(notification) });
    let origin = unsafe { (api.origin)(notification) };
    let origin = if origin.is_null() {
        String::new()
    } else {
        string(api, unsafe { (api.origin_string)(origin) })
    };
    let manager = manager_of(api, page as usize);
    authorize();
    let key = identifier(id);
    let posted = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let tab = state.pages.get(&(page as usize)).copied();
        let center = state.center.clone()?;
        state.shown.insert(
            key.clone(),
            Shown {
                manager,
                notification: id,
                tab,
            },
        );
        // Old entries belong to pages long gone; keep the map small.
        if state.shown.len() > 500
            && let Some(oldest) = state
                .shown
                .iter()
                .min_by_key(|(_, shown)| shown.notification)
                .map(|(key, _)| key.clone())
        {
            state.shown.remove(&oldest);
        }
        Some(center)
    });
    let Some(center) = posted else {
        return;
    };
    let (Some(content_class), Some(request_class)) = (
        AnyClass::get(c"UNMutableNotificationContent"),
        AnyClass::get(c"UNNotificationRequest"),
    ) else {
        return;
    };
    unsafe {
        let content: Retained<AnyObject> = msg_send![content_class, new];
        let _: () = msg_send![&content, setTitle: &*NSString::from_str(&title)];
        // The site, as Safari shows it, so a page can't pose as another app.
        let site = crate::model::display_host(&origin);
        let _: () = msg_send![&content, setSubtitle: &*NSString::from_str(&site)];
        let _: () = msg_send![&content, setBody: &*NSString::from_str(&body)];
        let _: () = msg_send![&content, setThreadIdentifier: &*NSString::from_str(&origin)];
        let request: Retained<AnyObject> = msg_send![
            request_class,
            requestWithIdentifier: &*NSString::from_str(&key),
            content: &*content,
            trigger: std::ptr::null::<AnyObject>()
        ];
        let added = RcBlock::new(|error: *mut NSError| {
            if let Some(error) = error.as_ref() {
                log::warn!("Notification not shown: {}", error.localizedDescription());
            }
        });
        let _: () =
            msg_send![&center, addNotificationRequest: &*request, withCompletionHandler: &*added];
        (api.did_show)(manager as Ref, id);
    }
}

fn remove(identifiers: &[String]) {
    let center = STATE.with(|state| {
        let mut state = state.borrow_mut();
        for identifier in identifiers {
            state.shown.remove(identifier);
        }
        state.center.clone()
    });
    let Some(center) = center else {
        return;
    };
    let identifiers: Vec<Retained<NSString>> = identifiers
        .iter()
        .map(|id| NSString::from_str(id))
        .collect();
    let identifiers = objc2_foundation::NSArray::from_retained_slice(&identifiers);
    let _: () =
        unsafe { msg_send![&center, removeDeliveredNotificationsWithIdentifiers: &*identifiers] };
    let _: () = unsafe {
        msg_send![&center, removePendingNotificationRequestsWithIdentifiers: &*identifiers]
    };
}

extern "C" fn cancel(notification: Ref, _: *const c_void) {
    if let Some(api) = api() {
        remove(&[identifier(unsafe { (api.notification_id)(notification) })]);
    }
}

// The page's Notification object was collected; what's on screen stays.
extern "C" fn did_destroy(_: Ref, _: *const c_void) {}

extern "C" fn add_manager(manager: Ref, _: *const c_void) {
    STATE.with(|state| state.borrow_mut().managers.insert(manager as usize));
}

extern "C" fn remove_manager(manager: Ref, _: *const c_void) {
    STATE.with(|state| state.borrow_mut().managers.remove(&(manager as usize)));
}

extern "C" fn permissions(_: *const c_void) -> Ref {
    let Some(api) = api() else {
        return std::ptr::null();
    };
    let allowed = STATE.with(|state| state.borrow().allowed.clone());
    unsafe {
        let dictionary = (api.dictionary_create)();
        let yes = (api.boolean_create)(true);
        for site in allowed {
            if let Ok(site) = std::ffi::CString::new(site) {
                let key = (api.string_create)(site.as_ptr());
                (api.dictionary_set)(dictionary, key, yes);
                (api.release)(key);
            }
        }
        (api.release)(yes);
        // WebKit adopts the returned dictionary.
        dictionary
    }
}

/// A page closed: its notifications close with it, as in other browsers.
extern "C" fn clear(notifications: Ref, _: *const c_void) {
    let Some(api) = api() else {
        return;
    };
    let count = unsafe { (api.array_size)(notifications) };
    let identifiers: Vec<String> = (0..count)
        .map(|index| unsafe { (api.uint64_value)((api.array_item)(notifications, index)) })
        .map(identifier)
        .collect();
    remove(&identifiers);
}

// Notification Center may call its delegate off the main thread, so clicks
// only cross to the shell as an event.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[ivars = Sender<Event>]
    struct Clicks;
    unsafe impl NSObjectProtocol for Clicks {}
    impl Clicks {
        // Show banners even while Browser is frontmost: the tab that
        // notified is usually not the one in view.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn present(&self, _center: &AnyObject, _notification: &AnyObject, handler: &Block<dyn Fn(NSUInteger)>) {
            const SOUND_LIST_BANNER: NSUInteger = (1 << 1) | (1 << 3) | (1 << 4);
            handler.call((SOUND_LIST_BANNER,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn respond(&self, _center: &AnyObject, response: &AnyObject, handler: &Block<dyn Fn()>) {
            let identifier: Option<Retained<NSString>> = unsafe {
                let notification: Option<Retained<AnyObject>> = msg_send![response, notification];
                notification.and_then(|notification| {
                    let request: Option<Retained<AnyObject>> = msg_send![&notification, request];
                    request.and_then(|request| msg_send![&request, identifier])
                })
            };
            if let Some(identifier) = identifier {
                let _ = self.ivars().try_send(Event::NotificationClicked(identifier.to_string()));
            }
            handler.call(());
        }
    }
);

impl Clicks {
    fn new(events: Sender<Event>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(events);
        unsafe { msg_send![super(this), init] }
    }
}
