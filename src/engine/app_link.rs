//! Hands non-web links (`mailto:`, `tel:`, `zoommtg:`…) to the app macOS has
//! registered for them, as Safari does. Callers confirm with the user first.
use objc2::rc::Retained;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSFileManager, NSString, NSURL};

fn native(url: &str) -> Option<Retained<NSURL>> {
    if url.len() > crate::model::APP_LINK_MAX {
        return None;
    }
    NSURL::URLWithString(&NSString::from_str(url))
}

/// Display name of the app that would open `url`, or `None` when no app
/// (other than this browser) is registered for its scheme.
pub fn handler(url: &str) -> Option<String> {
    let url = native(url)?;
    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.URLForApplicationToOpenURL(&url)?;
    let path = app.path()?;
    // Never hand a link back to ourselves (e.g. a scheme the bundle claims).
    if NSBundle::mainBundle().bundleURL().path().as_deref() == Some(&*path) {
        return None;
    }
    let name = NSFileManager::defaultManager().displayNameAtPath(&path);
    let name = name.to_string();
    Some(name.strip_suffix(".app").unwrap_or(&name).to_string())
}

/// Opens `url` in its registered app. Only call after the user confirmed.
pub fn open(url: &str) -> bool {
    native(url).is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url))
}
