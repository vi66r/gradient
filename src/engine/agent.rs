//! What a page gives the agents it's shared with, natively:
//! - their scripts run in a content world of their own, which shares the
//!   page's document but none of its JavaScript, so a page can't rewrite
//!   what they see or notice them reading;
//! - their clicks and typing are real input, the events the user's own
//!   would make, rather than script-dispatched ones the page can tell
//!   apart and ignore;
//! - screenshots, and for trusted sites the console and network logs.
//!   Neither log runs until the site is trusted: the console hook is inert
//!   without its message handler, and WebKit reports loads only to a
//!   delegate that's set.
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventModifierFlags, NSEventType, NSImage,
};
use objc2_foundation::{
    NSDictionary, NSError, NSJSONSerialization, NSJSONWritingOptions, NSPoint, NSString,
};
use objc2_web_kit::{
    WKContentWorld, WKScriptMessage, WKScriptMessageHandler, WKSnapshotConfiguration,
    WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView,
};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Instant};

/// How many console messages and network loads a page keeps.
const KEPT: usize = 300;
const CONSOLE_HANDLER: &str = "browserConsole";

/// Wraps the page's console and reports to `browserConsole`, which exists
/// only while the site is trusted; otherwise it changes nothing.
const CONSOLE_HOOK: &str = r#"(() => {
  const handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.browserConsole;
  if (!handler || console.log.__browserConsole) return;
  const text = value => {
    if (typeof value === 'string') return value;
    if (value instanceof Error) return value.stack || String(value);
    try { return JSON.stringify(value); } catch (_) { return String(value); }
  };
  const post = (level, values) => {
    try { handler.postMessage({ level, text: values.map(text).join(' ').slice(0, 4000), url: location.href }); } catch (_) {}
  };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level];
    const wrapped = function (...values) { post(level, values); return original.apply(this, values); };
    wrapped.__browserConsole = true;
    console[level] = wrapped;
  }
  addEventListener('error', event => post('error', [`${event.message} (${event.filename}:${event.lineno}:${event.colno})`]));
  addEventListener('unhandledrejection', event => post('error', ['Unhandled rejection: ' + text(event.reason)]));
})();"#;

#[derive(Default)]
struct Logs {
    console: RefCell<VecDeque<Value>>,
    /// Loads by WebKit's load ID, oldest first, with when each started.
    network: RefCell<VecDeque<(u64, Instant, Value)>>,
}

fn keep<T>(list: &mut VecDeque<T>, item: T) {
    if list.len() == KEPT {
        list.pop_front();
    }
    list.push_back(item);
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Logs>]
    struct Console;

    unsafe impl NSObjectProtocol for Console {}

    unsafe impl WKScriptMessageHandler for Console {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn received(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            let body = unsafe { message.body() };
            if let Some(value) = json_of(&body) {
                keep(&mut self.ivars().console.borrow_mut(), value);
            }
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Logs>]
    struct Loads;

    // WebKit's `_WKResourceLoadDelegate` (SPI), as Safari's Web Inspector
    // network tab is fed.
    impl Loads {
        #[unsafe(method(webView:resourceLoad:didSendRequest:))]
        fn sent(&self, _view: &AnyObject, load: &AnyObject, _request: &AnyObject) {
            let Some(id) = load_id(load) else { return };
            let url = unsafe {
                let url: Option<Retained<objc2_foundation::NSURL>> = msg_send![load, originalURL];
                url.and_then(|url| url.absoluteString()).map(|url| url.to_string())
            };
            let method = unsafe {
                let method: Option<Retained<NSString>> = msg_send![load, originalHTTPMethod];
                method.map(|method| method.to_string())
            };
            let cached: bool = unsafe { msg_send![load, loadedFromCache] };
            keep(
                &mut self.ivars().network.borrow_mut(),
                (id, Instant::now(), json!({"url": url, "method": method, "cached": cached})),
            );
        }

        #[unsafe(method(webView:resourceLoad:didReceiveResponse:))]
        fn responded(&self, _view: &AnyObject, load: &AnyObject, response: &AnyObject) {
            self.update(load, |entry| describe_response(entry, response));
        }

        #[unsafe(method(webView:resourceLoad:didCompleteWithError:response:))]
        fn completed(&self, _view: &AnyObject, load: &AnyObject, error: *mut NSError, response: *mut AnyObject) {
            self.update(load, |entry| {
                if let Some(response) = unsafe { response.as_ref() } {
                    describe_response(entry, response);
                }
                if let Some(error) = unsafe { error.as_ref() } {
                    entry["error"] = json!(error.localizedDescription().to_string());
                }
            });
        }
    }
);

impl Loads {
    fn update(&self, load: &AnyObject, change: impl FnOnce(&mut Value)) {
        let Some(id) = load_id(load) else { return };
        let mut network = self.ivars().network.borrow_mut();
        if let Some((_, started, entry)) = network.iter_mut().rev().find(|(load, ..)| *load == id) {
            change(entry);
            entry["ms"] = json!(started.elapsed().as_millis() as u64);
        }
    }
}

fn load_id(load: &AnyObject) -> Option<u64> {
    let responds: bool =
        unsafe { msg_send![load, respondsToSelector: objc2::sel!(resourceLoadID)] };
    responds.then(|| unsafe { msg_send![load, resourceLoadID] })
}

fn describe_response(entry: &mut Value, response: &AnyObject) {
    unsafe {
        let http: bool = msg_send![response, respondsToSelector: objc2::sel!(statusCode)];
        if http {
            let status: isize = msg_send![response, statusCode];
            entry["status"] = json!(status);
        }
        let mime: Option<Retained<NSString>> = msg_send![response, MIMEType];
        if let Some(mime) = mime {
            entry["type"] = json!(mime.to_string());
        }
        let length: i64 = msg_send![response, expectedContentLength];
        if length >= 0 {
            entry["bytes"] = json!(length);
        }
    }
}

/// A Foundation property list (a script message's body) as JSON.
fn json_of(object: &AnyObject) -> Option<Value> {
    let valid = unsafe { NSJSONSerialization::isValidJSONObject(object) };
    if !valid {
        return None;
    }
    let data = unsafe {
        NSJSONSerialization::dataWithJSONObject_options_error(object, NSJSONWritingOptions::empty())
    }
    .ok()?;
    serde_json::from_slice(&data.to_vec()).ok()
}

pub struct Agent {
    view: Weak<WKWebView>,
    world: Retained<WKContentWorld>,
    logs: Rc<Logs>,
    /// The page's own content controller, which the console hook and its
    /// handler need; `None` for a popup or extension page sharing another's.
    controller: Option<Retained<WKUserContentController>>,
    console: RefCell<Option<Retained<Console>>>,
    loads: RefCell<Option<Retained<Loads>>>,
}

impl Agent {
    pub fn install(view: &WKWebView, own_configuration: bool) -> Self {
        let mtm = MainThreadMarker::new().expect("Pages are created on the UI thread");
        let world =
            unsafe { WKContentWorld::worldWithName(&NSString::from_str("BrowserAgent"), mtm) };
        let controller =
            own_configuration.then(|| unsafe { view.configuration().userContentController() });
        if let Some(controller) = &controller {
            let script = unsafe {
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    WKUserScript::alloc(mtm),
                    &NSString::from_str(CONSOLE_HOOK),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    true,
                )
            };
            unsafe { controller.addUserScript(&script) };
        }
        Self {
            view: Weak::from_retained(&objc2::Message::retain(view)),
            world,
            logs: Rc::default(),
            controller,
            console: RefCell::default(),
            loads: RefCell::default(),
        }
    }

    /// Starts or stops the console and network logs, as the page's site is
    /// trusted or not. Starting hooks the current document's console too.
    pub fn set_trusted(&self, trusted: bool) {
        let Some(view) = self.view.load() else { return };
        let mtm = MainThreadMarker::new().expect("Pages live on the UI thread");
        let name = NSString::from_str(CONSOLE_HANDLER);
        if trusted {
            if let Some(controller) = &self.controller
                && self.console.borrow().is_none()
            {
                let console: Retained<Console> = unsafe {
                    msg_send![
                        super(Console::alloc(mtm).set_ivars(self.logs.clone())),
                        init
                    ]
                };
                unsafe {
                    controller
                        .addScriptMessageHandler_name(ProtocolObject::from_ref(&*console), &name);
                    view.evaluateJavaScript_completionHandler(
                        &NSString::from_str(CONSOLE_HOOK),
                        None,
                    );
                }
                *self.console.borrow_mut() = Some(console);
            }
            let settable: bool = unsafe {
                msg_send![&*view, respondsToSelector: objc2::sel!(_setResourceLoadDelegate:)]
            };
            if settable && self.loads.borrow().is_none() {
                let loads: Retained<Loads> = unsafe {
                    msg_send![super(Loads::alloc(mtm).set_ivars(self.logs.clone())), init]
                };
                let _: () = unsafe { msg_send![&*view, _setResourceLoadDelegate: &*loads] };
                *self.loads.borrow_mut() = Some(loads);
            }
        } else {
            if let Some(controller) = &self.controller
                && self.console.borrow_mut().take().is_some()
            {
                unsafe { controller.removeScriptMessageHandlerForName(&name) };
            }
            if self.loads.borrow_mut().take().is_some() {
                let _: () = unsafe {
                    msg_send![&*view, _setResourceLoadDelegate: std::ptr::null::<AnyObject>()]
                };
            }
            self.logs.console.borrow_mut().clear();
            self.logs.network.borrow_mut().clear();
        }
    }

    pub fn console(&self) -> Value {
        json!({"messages": self.logs.console.borrow().iter().collect::<Vec<_>>()})
    }

    pub fn network(&self) -> Value {
        let network = self.logs.network.borrow();
        let loads: Vec<&Value> = network.iter().map(|(.., entry)| entry).collect();
        json!({"loads": loads})
    }

    /// Runs `body` as an async function with `args` (JSON) parsed into
    /// `args`, in the agent's own world or, for trusted code, the page's.
    /// Its return value comes back as JSON text.
    pub fn call(
        &self,
        body: &str,
        args: &str,
        page_world: bool,
        done: impl FnOnce(Result<String, String>) + 'static,
    ) {
        let Some(view) = self.view.load() else {
            return done(Err("The page closed.".into()));
        };
        let mtm = MainThreadMarker::new().expect("Pages live on the UI thread");
        let world = if page_world {
            unsafe { WKContentWorld::pageWorld(mtm) }
        } else {
            self.world.clone()
        };
        let source = format!(
            "const args = JSON.parse(argsJSON);\nconst result = await (async () => {{\n{body}\n}})();\ntry {{ return JSON.stringify(result === undefined ? null : result); }} catch (_) {{ return JSON.stringify(String(result)); }}"
        );
        let arguments = NSDictionary::from_slices(
            &[&*NSString::from_str("argsJSON")],
            &[&*Retained::into_super(Retained::into_super(
                NSString::from_str(args),
            ))],
        );
        let done = std::cell::Cell::new(Some(done));
        let block = block2::RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            let Some(done) = done.take() else { return };
            if let Some(error) = unsafe { error.as_ref() } {
                let message = error
                    .userInfo()
                    .objectForKey(&NSString::from_str("WKJavaScriptExceptionMessage"))
                    .and_then(|message| message.downcast::<NSString>().ok())
                    .map(|message| message.to_string())
                    .unwrap_or_else(|| error.localizedDescription().to_string());
                return done(Err(message));
            }
            let text = unsafe { result.as_ref() }
                .and_then(|result| result.downcast_ref::<NSString>())
                .map(|text| text.to_string())
                .unwrap_or_else(|| "null".into());
            done(Ok(text));
        });
        unsafe {
            view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                &NSString::from_str(&source),
                Some(&arguments),
                None,
                &world,
                Some(&block),
            )
        };
    }

    /// Whether real input can reach the page: it's on screen in a window.
    pub fn can_input(&self) -> bool {
        self.view
            .load()
            .is_some_and(|view| view.window().is_some() && !view.isHiddenOrHasHiddenAncestor())
    }

    /// A real click at a point of the page's viewport, in CSS pixels.
    pub fn click(&self, x: f64, y: f64) -> Result<(), String> {
        let view = self.view.load().ok_or("The page closed.")?;
        let mtm = MainThreadMarker::new().expect("Pages live on the UI thread");
        let window = view.window().ok_or("The page isn't in a window.")?;
        let zoom = unsafe { view.pageZoom() };
        let point = view.convertPoint_toView(NSPoint::new(x * zoom, y * zoom), None);
        let number = window.windowNumber();
        let time = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
        let event = |kind| {
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind, point, NSEventModifierFlags::empty(), time, number, None, 0, 1, 1.,
            )
        };
        let _ = mtm;
        let (moved, down, up) = (
            event(NSEventType::MouseMoved),
            event(NSEventType::LeftMouseDown),
            event(NSEventType::LeftMouseUp),
        );
        let (Some(moved), Some(down), Some(up)) = (moved, down, up) else {
            return Err("Couldn't make the click.".into());
        };
        view.mouseMoved(&moved);
        view.mouseDown(&down);
        view.mouseUp(&up);
        Ok(())
    }

    /// Types `text` into the focused element as real text input.
    pub fn type_text(&self, text: &str) -> Result<(), String> {
        let view = self.view.load().ok_or("The page closed.")?;
        let none = objc2_foundation::NSRange::new(objc2_foundation::NSNotFound as usize, 0);
        let text = NSString::from_str(text);
        let _: () = unsafe { msg_send![&*view, insertText: &*text, replacementRange: none] };
        Ok(())
    }

    /// Presses a key ("Enter", "ArrowDown", "a") with modifiers ("cmd",
    /// "shift", "alt", "ctrl"), as a real key down and up.
    pub fn press(&self, key: &str, modifiers: &[&str]) -> Result<(), String> {
        let view = self.view.load().ok_or("The page closed.")?;
        let window = view.window().ok_or("The page isn't in a window.")?;
        let (code, characters) = key_code(key).ok_or_else(|| format!("Unknown key “{key}”."))?;
        let mut flags = NSEventModifierFlags::empty();
        for modifier in modifiers {
            flags |= match *modifier {
                "cmd" | "meta" => NSEventModifierFlags::Command,
                "shift" => NSEventModifierFlags::Shift,
                "alt" | "option" => NSEventModifierFlags::Option,
                "ctrl" | "control" => NSEventModifierFlags::Control,
                other => return Err(format!("Unknown modifier “{other}”.")),
            };
        }
        let shown = if flags.contains(NSEventModifierFlags::Shift) {
            characters.to_uppercase()
        } else {
            characters.clone()
        };
        let time = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
        let event = |kind| {
            NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind, NSPoint::ZERO, flags, time, window.windowNumber(), None,
                &NSString::from_str(&shown), &NSString::from_str(&characters), false, code,
            )
        };
        let (Some(down), Some(up)) = (event(NSEventType::KeyDown), event(NSEventType::KeyUp))
        else {
            return Err("Couldn't make the key press.".into());
        };
        if !flags.contains(NSEventModifierFlags::Command) || !view.performKeyEquivalent(&down) {
            view.keyDown(&down);
        }
        view.keyUp(&up);
        Ok(())
    }

    /// A PNG of what the page shows now, base64-encoded.
    pub fn screenshot(&self, done: impl FnOnce(Result<String, String>) + 'static) {
        let Some(view) = self.view.load() else {
            return done(Err("The page closed.".into()));
        };
        let mtm = MainThreadMarker::new().expect("Pages live on the UI thread");
        let config = unsafe { WKSnapshotConfiguration::new(mtm) };
        let done = std::cell::Cell::new(Some(done));
        let block = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let Some(done) = done.take() else { return };
            let Some(image) = (unsafe { image.as_ref() }) else {
                let message = unsafe { error.as_ref() }
                    .map(|error| error.localizedDescription().to_string())
                    .unwrap_or_else(|| "Couldn't take the screenshot.".into());
                return done(Err(message));
            };
            done(
                png(image)
                    .map(|png| crate::mcp::base64(&png))
                    .ok_or_else(|| "Couldn't encode the screenshot.".into()),
            );
        });
        unsafe { view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block) };
    }
}

fn png(image: &NSImage) -> Option<Vec<u8>> {
    let cg =
        unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    let data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(data.to_vec())
}

/// macOS virtual key codes and the characters a key types.
fn key_code(key: &str) -> Option<(u16, String)> {
    let named = |code: u16, text: &str| Some((code, text.to_string()));
    let function = |code: u16, scalar: u32| Some((code, char::from_u32(scalar)?.to_string()));
    match key {
        "Enter" | "Return" => named(36, "\r"),
        "Tab" => named(48, "\t"),
        "Escape" => named(53, "\u{1b}"),
        "Backspace" => named(51, "\u{7f}"),
        "Space" | " " => named(49, " "),
        "Delete" => function(117, 0xF728),
        "ArrowUp" => function(126, 0xF700),
        "ArrowDown" => function(125, 0xF701),
        "ArrowLeft" => function(123, 0xF702),
        "ArrowRight" => function(124, 0xF703),
        "Home" => function(115, 0xF729),
        "End" => function(119, 0xF72B),
        "PageUp" => function(116, 0xF72C),
        "PageDown" => function(121, 0xF72D),
        _ => {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let lower = c.to_ascii_lowercase();
            const LETTERS: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.";
            let code = LETTERS.find(lower).map(|index| index as u16).unwrap_or(0);
            Some((code, lower.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::key_code;

    #[test]
    fn keys_map_to_macos_codes() {
        assert_eq!(key_code("Enter").unwrap().0, 36);
        assert_eq!(key_code("a").unwrap(), (0, "a".to_string()));
        assert_eq!(key_code("A").unwrap(), (0, "a".to_string()));
        assert_eq!(key_code("z").unwrap().0, 6);
        assert!(key_code("Hyper").is_none());
    }
}
