//! Spike: web extensions through WebKit's `WKWebExtension` API (macOS 15.4+).
//! `BROWSER_EXTENSIONS` names unpacked extension folders (colon-separated);
//! they load into one controller that every page joins. Each extension gets
//! everything it requests, with no prompts, and there is no tab or window
//! model yet, so APIs that need one (`runtime.sendMessage` from a page,
//! `tabs`) fail. Findings: docs/extensions-spike.md.
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionContext, WKWebExtensionContextPermissionStatus,
    WKWebExtensionController, WKWebExtensionControllerConfiguration, WKWebExtensionMatchPattern,
    WKWebViewConfiguration,
};
use std::cell::OnceCell;

const GRANTED: WKWebExtensionContextPermissionStatus =
    WKWebExtensionContextPermissionStatus::GrantedExplicitly;

thread_local! {
    static CONTROLLER: OnceCell<Option<Retained<WKWebExtensionController>>> =
        const { OnceCell::new() };
}

/// The shared controller, when extensions were requested.
pub fn controller(mtm: MainThreadMarker) -> Option<Retained<WKWebExtensionController>> {
    CONTROLLER.with(|cell| cell.get_or_init(|| start(mtm)).clone())
}

fn start(mtm: MainThreadMarker) -> Option<Retained<WKWebExtensionController>> {
    let folders = std::env::var("BROWSER_EXTENSIONS").ok()?;
    let available: bool = unsafe {
        objc2::msg_send![objc2::class!(WKWebViewConfiguration), instancesRespondToSelector: objc2::sel!(setWebExtensionController:)]
    };
    if !available {
        log::warn!("Web extensions need macOS 15.4 or later.");
        return None;
    }
    let configuration =
        unsafe { WKWebExtensionControllerConfiguration::nonPersistentConfiguration(mtm) };
    unsafe {
        // Extension pages and workers identify as Safari, as Browser's pages
        // do; extensions pick their code paths by user agent.
        let pages = WKWebViewConfiguration::new(mtm);
        pages.setApplicationNameForUserAgent(Some(&NSString::from_str(&format!(
            "Version/{} Safari/605.1.15",
            super::safari_version()
        ))));
        configuration.setWebViewConfiguration(Some(&pages));
    }
    let controller = unsafe {
        WKWebExtensionController::initWithConfiguration(
            WKWebExtensionController::alloc(mtm),
            &configuration,
        )
    };
    for folder in folders.split(':').filter(|folder| !folder.is_empty()) {
        load(&controller, folder, mtm);
    }
    Some(controller)
}

fn load(controller: &Retained<WKWebExtensionController>, folder: &str, mtm: MainThreadMarker) {
    let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(folder), true);
    let controller = controller.clone();
    let name = folder.to_string();
    let done = block2::RcBlock::new(move |extension: *mut WKWebExtension, error: *mut NSError| {
        let Some(extension) = (unsafe { extension.as_ref() }) else {
            let reason =
                unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
            log::warn!("Extension {name} didn't load: {reason:?}");
            return;
        };
        unsafe {
            let context = WKWebExtensionContext::contextForExtension(extension);
            context.setInspectable(true);
            for permission in extension.requestedPermissions().allObjects().iter() {
                let _: () = objc2::msg_send![&context, setPermissionStatus: GRANTED, forPermission: &*permission];
            }
            for pattern in extension
                .requestedPermissionMatchPatterns()
                .allObjects()
                .iter()
            {
                let _: () = objc2::msg_send![&context, setPermissionStatus: GRANTED, forMatchPattern: &*pattern];
            }
            let all = WKWebExtensionMatchPattern::allURLsMatchPattern(mtm);
            let _: () =
                objc2::msg_send![&context, setPermissionStatus: GRANTED, forMatchPattern: &*all];
            match controller.loadExtensionContext_error(&context) {
                Ok(()) => log::warn!(
                    "Extension loaded: {} {}",
                    extension
                        .displayName()
                        .map(|name| name.to_string())
                        .unwrap_or_default(),
                    extension
                        .version()
                        .map(|version| version.to_string())
                        .unwrap_or_default()
                ),
                Err(error) => log::warn!(
                    "Extension {name} didn't start: {}",
                    error.localizedDescription()
                ),
            }
            for error in context.errors().iter() {
                log::warn!("Extension {name}: {}", error.localizedDescription());
            }
        }
    });
    unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm) };
}

/// Joins a page configuration to the controller, if there is one.
pub fn attach(configuration: &WKWebViewConfiguration, mtm: MainThreadMarker) {
    if let Some(controller) = controller(mtm) {
        unsafe { configuration.setWebExtensionController(Some(&controller)) };
    }
}
